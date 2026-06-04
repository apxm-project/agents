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
use std::net::IpAddr;
use std::sync::OnceLock;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_BODY_BYTES: usize = 1_000_000;

/// True if an address must not be reached from a tool HTTP call — loopback,
/// private, link-local (incl. 169.254.169.254 cloud metadata), unspecified,
/// multicast/broadcast, IPv6 ULA. The SSRF block-list.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local() // 169.254.0.0/16 — covers cloud metadata
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

/// SSRF guard for tool HTTP calls: http(s) only, and the host must not resolve
/// to a blocked address. DNS names are resolved so a public name pointing at a
/// private IP is rejected too.
pub async fn guard_url_ssrf(cap: &str, raw: &str) -> CapabilityResult<()> {
    let deny = |m: String| RuntimeError::Capability {
        capability: cap.to_string(),
        message: m,
    };
    let url = reqwest::Url::parse(raw).map_err(|e| deny(format!("invalid url: {e}")))?;
    match url.scheme() {
        "http" | "https" => {}
        s => return Err(deny(format!("scheme '{s}' not allowed (http/https only)"))),
    }
    let host = url
        .host_str()
        .ok_or_else(|| deny("url has no host".to_string()))?;
    let ips: Vec<IpAddr> = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![ip]
    } else {
        let port = url.port_or_known_default().unwrap_or(443);
        tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| deny(format!("could not resolve host '{host}': {e}")))?
            .map(|sa| sa.ip())
            .collect()
    };
    if ips.is_empty() || ips.iter().copied().any(is_blocked_ip) {
        return Err(deny(format!(
            "host '{host}' resolves to a blocked (loopback/private/link-local) address"
        )));
    }
    Ok(())
}

/// One process-wide HTTP client, built lazily on first use (never at startup /
/// capability construction — building a reqwest client eagerly during runtime
/// setup wedged the executor's completion path).
fn shared_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            // Limit redirects AND refuse any hop whose host is a blocked IP
            // literal (defence-in-depth against redirect-to-metadata SSRF).
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.error("too many redirects");
                }
                if let Some(host) = attempt.url().host_str()
                    && let Ok(ip) = host.parse::<IpAddr>()
                    && is_blocked_ip(ip)
                {
                    return attempt.stop();
                }
                attempt.follow()
            }))
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
        guard_url_ssrf(&self.metadata.name, &url).await?;
        let resp = shared_client().get(&url).headers(header_map(&args)).send().await;
        finish(&self.metadata.name, resp).await
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

#[cfg(test)]
mod ssrf_tests {
    use super::*;

    #[test]
    fn blocks_private_loopback_and_metadata_ips() {
        for s in [
            "127.0.0.1", "10.0.0.5", "192.168.1.1", "172.16.0.1", "169.254.169.254", "0.0.0.0",
            "::1", "fc00::1", "fe80::1",
        ] {
            assert!(is_blocked_ip(s.parse().unwrap()), "{s} should be blocked");
        }
        for s in ["8.8.8.8", "1.1.1.1", "93.184.216.34"] {
            assert!(!is_blocked_ip(s.parse().unwrap()), "{s} should be allowed");
        }
    }

    #[tokio::test]
    async fn guard_rejects_bad_scheme_and_private_ip() {
        assert!(guard_url_ssrf("http_get", "file:///etc/passwd").await.is_err());
        assert!(guard_url_ssrf("http_get", "http://169.254.169.254/latest/meta-data").await.is_err());
        assert!(guard_url_ssrf("http_get", "http://127.0.0.1:8080/").await.is_err());
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
        guard_url_ssrf(&self.metadata.name, &url).await?;
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
