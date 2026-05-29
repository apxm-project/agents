//! `provider.call` — the generic outbound block backing every connector action.
//!
//! Forwards an authenticated HTTP call to a provider through apxm-auth's
//! inject-and-forward `/proxy`: the kernel hands apxm-auth the connection id,
//! method, url, headers and body; apxm-auth attaches the secret and forwards.
//! The secret never enters the kernel. ONE capability serves every `api_base`
//! provider, so onboarding a connector needs no per-provider Rust — a pack just
//! declares an action block whose capability is `provider.call`.

use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::CapabilityMetadata,
};
use apxm_core::{error::RuntimeError, types::Value};
use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::sync::OnceLock;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_BODY_BYTES: usize = 1_000_000;

/// Lazily-built shared client (never at construction — see http.rs note).
fn shared_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    })
}

/// apxm-auth base URL (`APXM_AUTH_URL`, default loopback).
fn auth_base() -> String {
    std::env::var("APXM_AUTH_URL").unwrap_or_else(|_| "http://127.0.0.1:18810".to_string())
}

/// apxm-auth's per-run bearer, written 0600 by `apxm-auth serve`.
fn auth_bearer() -> Option<String> {
    let dir = std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))
        .ok()?;
    std::fs::read_to_string(dir.join("apxm/auth/auth.bearer")).ok().map(|s| s.trim().to_string())
}

/// Percent-encode a path segment (the connection id).
fn enc(seg: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(seg.len());
    for b in seg.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0f) as usize] as char);
            }
        }
    }
    out
}

pub struct ProviderCallCapability {
    metadata: CapabilityMetadata,
    /// apxm-auth base override; `None` resolves from `APXM_AUTH_URL` at dispatch.
    base: Option<String>,
}

impl Default for ProviderCallCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderCallCapability {
    /// A named provider-backed capability (e.g. `slack.post`) with the same
    /// proxy-forwarding behaviour as `provider.call`. Used when a pack registers
    /// a `kind = "provider"` tool so `/v1/capabilities` lists the block by name.
    pub fn named(name: impl Into<String>, description: impl Into<String>, schema: serde_json::Value) -> Self {
        Self {
            base: None,
            metadata: CapabilityMetadata::new(name, description, schema)
                .with_returns("string")
                .with_groups(vec!["provider".to_string(), "http".to_string()])
                .with_latency(500),
        }
    }

    pub fn new() -> Self {
        Self {
            base: None,
            metadata: CapabilityMetadata::new(
                "provider.call",
                "Authenticated HTTP call to a connected provider (via apxm-auth proxy; secret stays in apxm-auth)",
                json!({
                    "type": "object",
                    "properties": {
                        "credential": { "type": "string", "description": "apxm-auth connection id; the secret never leaves apxm-auth" },
                        "method": { "type": "string", "description": "HTTP method (default POST)" },
                        "url": { "type": "string", "description": "Absolute provider URL (scoped to the provider api_base)" },
                        "headers": { "type": "object", "description": "Optional request headers" },
                        "body": { "description": "JSON object or string body" }
                    },
                    "required": ["credential", "url"]
                }),
            )
            .with_returns("string")
            .with_groups(vec!["provider".to_string(), "http".to_string()])
            .with_latency(500),
        }
    }
}

#[async_trait]
impl CapabilityExecutor for ProviderCallCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let cap = self.metadata.name.clone();
        let cap_err = |msg: String| RuntimeError::Capability { capability: cap.clone(), message: msg };
        let as_json = |k: &str| args.get(k).and_then(|v| serde_json::to_value(v).ok());

        let credential = as_json("credential")
            .and_then(|j| j.as_str().map(String::from))
            .ok_or_else(|| cap_err("missing `credential` (apxm-auth connection id)".into()))?;
        let url = as_json("url")
            .and_then(|j| j.as_str().map(String::from))
            .ok_or_else(|| cap_err("missing `url`".into()))?;
        let method = as_json("method").and_then(|j| j.as_str().map(String::from)).unwrap_or_else(|| "POST".into());
        let headers: Vec<(String, String)> = as_json("headers")
            .and_then(|j| j.as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()))
            .unwrap_or_default();

        let b64 = base64::engine::general_purpose::STANDARD;
        let mut payload = json!({ "method": method, "url": url, "headers": headers });
        if let Some(body) = as_json("body") {
            let bytes = match body {
                JsonValue::String(s) => s.into_bytes(),
                other => other.to_string().into_bytes(),
            };
            payload["body_b64"] = JsonValue::String(b64.encode(&bytes));
        }

        let base = self.base.clone().unwrap_or_else(auth_base);
        let endpoint = format!("{}/v1/connections/{}/proxy", base, enc(&credential));
        let mut req = shared_client().post(&endpoint).json(&payload);
        if let Some(b) = auth_bearer() {
            req = req.bearer_auth(b);
        }
        let resp = req.send().await.map_err(|e| cap_err(format!("apxm-auth proxy request failed: {e}")))?;
        if !resp.status().is_success() {
            let s = resp.status();
            return Err(cap_err(format!("apxm-auth proxy returned {s}: {}", resp.text().await.unwrap_or_default())));
        }
        let pr: ProxyResp = resp.json().await.map_err(|e| cap_err(format!("proxy response parse: {e}")))?;
        let mut bytes = b64.decode(pr.body_b64.unwrap_or_default()).map_err(|e| cap_err(format!("body_b64 decode: {e}")))?;
        if bytes.len() > MAX_BODY_BYTES {
            bytes.truncate(MAX_BODY_BYTES);
        }
        let out = String::from_utf8_lossy(&bytes).into_owned();
        if pr.status >= 400 {
            return Err(cap_err(format!("provider returned {}: {out}", pr.status)));
        }
        Ok(Value::String(out))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

#[derive(serde::Deserialize)]
struct ProxyResp {
    status: u16,
    #[serde(default)]
    body_b64: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enc_encodes_unsafe_segments() {
        assert_eq!(enc("ok-id_1.2~"), "ok-id_1.2~");
        assert_eq!(enc("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn metadata_is_provider_call_write_class() {
        let c = ProviderCallCapability::new();
        assert_eq!(c.metadata().name, "provider.call");
        assert!(!c.metadata().read_only, "provider.call is write-class");
    }

    #[tokio::test]
    async fn forwards_through_proxy_and_returns_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // Mock apxm-auth /proxy: returns {status, body_b64} for an echoed body.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = s.read(&mut buf).await;
            let b64 = base64::engine::general_purpose::STANDARD;
            let body = format!("{{\"status\":200,\"body_b64\":\"{}\"}}", b64.encode(b"{\"ok\":true}"));
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes()).await;
            let _ = s.shutdown().await;
        });
        let mut cap = ProviderCallCapability::new();
        cap.base = Some(format!("http://127.0.0.1:{port}"));
        let mut args = HashMap::new();
        args.insert("credential".to_string(), Value::String("conn-1".into()));
        args.insert("url".to_string(), Value::String("https://api.example.com/x".into()));
        let out = cap.execute(args).await.expect("provider.call");
        assert_eq!(out, Value::String("{\"ok\":true}".into()));
    }
}
