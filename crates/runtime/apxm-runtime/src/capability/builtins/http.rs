//! Built-in HTTP capabilities: `http_get` and `http_post`.
//!
//! Let workflow `inv_tool` nodes call HTTP/JSON APIs (e.g. public REST
//! endpoints) directly. Read-only `http_get` and mutating `http_post`. Auth
//! tokens arrive as a `headers` object (resolved from a credential by the
//! caller), never inlined by the compiler. Returns the response body as a
//! string.

use super::require_string_arg;
use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::CapabilityMetadata,
};
use apxm_core::{constants::capabilities, error::RuntimeError, types::Value};
use async_trait::async_trait;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::json;
use std::collections::HashMap;
use std::sync::OnceLock;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_BODY_BYTES: usize = 1_000_000;

/// One process-wide HTTP client, built lazily on first use (never at startup /
/// capability construction — building a reqwest client eagerly during runtime
/// setup wedged the executor's completion path).
fn shared_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default()
    })
}

/// Build request headers from an optional `headers` object arg. Uses a
/// serde_json round-trip so we don't depend on the (build-generated) `Value`
/// variant shape.
fn header_map(args: &HashMap<String, Value>) -> HeaderMap {
    let mut map = HeaderMap::new();
    if let Some(hv) = args.get("headers") {
        if let Ok(serde_json::Value::Object(obj)) = serde_json::to_value(hv) {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    if let (Ok(name), Ok(val)) =
                        (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(s))
                    {
                        map.insert(name, val);
                    }
                }
            }
        }
    }
    map
}

async fn finish(
    cap: &str,
    resp: Result<reqwest::Response, reqwest::Error>,
) -> CapabilityResult<Value> {
    let resp = resp.map_err(|e| RuntimeError::Capability {
        capability: cap.to_string(),
        message: format!("request failed: {e}"),
    })?;
    let status = resp.status();
    let mut body = resp.text().await.unwrap_or_default();
    if body.len() > MAX_BODY_BYTES {
        body.truncate(MAX_BODY_BYTES);
    }
    if !status.is_success() {
        return Err(RuntimeError::Capability {
            capability: cap.to_string(),
            message: format!("HTTP {status}: {body}"),
        });
    }
    Ok(Value::String(body))
}

/// `http_get(url, headers?)` — fetch a URL, return the response body.
pub struct HttpGetCapability {
    metadata: CapabilityMetadata,
}

impl Default for HttpGetCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpGetCapability {
    pub fn new() -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                capabilities::HTTP_GET,
                "HTTP GET a URL and return the response body",
                json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "URL to GET" },
                        "headers": { "type": "object", "description": "Optional request headers" }
                    },
                    "required": ["url"]
                }),
            )
            .with_returns("string")
            .with_read_only()
            .with_groups(vec!["http".to_string(), "web".to_string()])
            .with_latency(300),
        }
    }
}

#[async_trait]
impl CapabilityExecutor for HttpGetCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let url = require_string_arg(&args, "url", &self.metadata.name)?.to_string();
        let resp = shared_client().get(&url).headers(header_map(&args)).send().await;
        finish(&self.metadata.name, resp).await
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

/// `http_post(url, body?, headers?)` — POST a JSON/string body, return the body.
pub struct HttpPostCapability {
    metadata: CapabilityMetadata,
}

impl Default for HttpPostCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpPostCapability {
    pub fn new() -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                capabilities::HTTP_POST,
                "HTTP POST a JSON/string body to a URL and return the response body",
                json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "URL to POST" },
                        "body": { "description": "JSON object or string body" },
                        "headers": { "type": "object", "description": "Optional request headers" }
                    },
                    "required": ["url"]
                }),
            )
            .with_returns("string")
            .with_groups(vec!["http".to_string(), "web".to_string()])
            .with_latency(400),
        }
    }
}

#[async_trait]
impl CapabilityExecutor for HttpPostCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let url = require_string_arg(&args, "url", &self.metadata.name)?.to_string();
        let mut req = shared_client().post(&url).headers(header_map(&args));
        if let Some(body) = args.get("body") {
            match serde_json::to_value(body) {
                Ok(serde_json::Value::String(s)) => req = req.body(s),
                Ok(jv) => req = req.header("content-type", "application/json").body(jv.to_string()),
                Err(_) => {}
            }
        }
        finish(&self.metadata.name, req.send().await).await
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}
