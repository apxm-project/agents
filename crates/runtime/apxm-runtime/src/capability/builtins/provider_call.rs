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

/// Owner/tenant scoping for apxm-auth requests. apxm-auth requires an `owner`
/// query parameter; we use `APXM_AUTH_OWNER` (default `"default"`, matching
/// apxm-auth's own oauth_start default). The service bearer is separate and
/// still sent: it authenticates the service, owner scopes the tenant.
fn auth_owner() -> String {
    std::env::var("APXM_AUTH_OWNER").unwrap_or_else(|_| "default".to_string())
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
    /// Named REST mode (set for a declared `kind = "provider"` block): the
    /// `tools.toml` method + url template. When present the request is built
    /// from loose args — `{param}` placeholders in the url are filled from args,
    /// the remaining (non-reserved) args form the JSON body. When `None`
    /// (`provider.call`), url/method/body are taken from args verbatim.
    rest: Option<RestTemplate>,
}

#[derive(Clone)]
struct RestTemplate {
    method: String,
    url: String,
}

/// Arg keys consumed by the capability itself, never forwarded as body fields.
const RESERVED_ARGS: &[&str] = &["credential", "result_path", "method", "url", "headers", "body"];

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
            rest: None,
            metadata: CapabilityMetadata::new(name, description, schema)
                .with_returns("string")
                .with_groups(vec!["provider".to_string(), "http".to_string()])
                .with_latency(500),
        }
    }

    /// A named provider block backed by a declared REST template (method + url
    /// from `tools.toml`). The node carries loose args; `{param}` placeholders in
    /// the url are filled from args and the rest become the JSON body — so a
    /// connector action needs no per-provider Rust and no `url` in the node.
    pub fn named_rest(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: serde_json::Value,
        method: impl Into<String>,
        url: impl Into<String>,
    ) -> Self {
        let mut cap = Self::named(name, description, schema);
        cap.rest = Some(RestTemplate { method: method.into(), url: url.into() });
        cap
    }

    pub fn new() -> Self {
        Self {
            base: None,
            rest: None,
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
        let result_path = as_json("result_path").and_then(|j| j.as_str().map(String::from));
        let headers: Vec<(String, String)> = as_json("headers")
            .and_then(|j| j.as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()))
            .unwrap_or_default();

        // Resolve method/url/body. Named REST mode fills the url template from
        // args and builds the body from loose (non-reserved) args; generic
        // provider.call takes url/method/body from args verbatim.
        let (method, url, body): (String, String, Option<JsonValue>) = if let Some(rest) = &self.rest {
            let mut url = rest.url.clone();
            let mut path_params: Vec<String> = Vec::new();
            for (k, v) in &args {
                let placeholder = format!("{{{k}}}");
                if url.contains(&placeholder) {
                    url = url.replace(&placeholder, &value_to_str(v));
                    path_params.push(k.clone());
                }
            }
            let method = as_json("method").and_then(|j| j.as_str().map(String::from)).unwrap_or_else(|| rest.method.clone());
            // Explicit `body` wins; else assemble it from the loose args that are
            // neither reserved nor consumed as url path params.
            let body = as_json("body").or_else(|| {
                let obj: serde_json::Map<String, JsonValue> = args
                    .iter()
                    .filter(|(k, _)| !RESERVED_ARGS.contains(&k.as_str()) && !path_params.contains(k))
                    .filter_map(|(k, v)| serde_json::to_value(v).ok().map(|j| (k.clone(), j)))
                    .collect();
                (!obj.is_empty()).then_some(JsonValue::Object(obj))
            });
            (method, url, body)
        } else {
            let url = as_json("url")
                .and_then(|j| j.as_str().map(String::from))
                .ok_or_else(|| cap_err("missing `url`".into()))?;
            let method = as_json("method").and_then(|j| j.as_str().map(String::from)).unwrap_or_else(|| "POST".into());
            (method, url, as_json("body"))
        };

        let b64 = base64::engine::general_purpose::STANDARD;
        let mut payload = json!({ "method": method, "url": url, "headers": headers });
        if let Some(body) = body {
            let bytes = match body {
                JsonValue::String(s) => s.into_bytes(),
                other => other.to_string().into_bytes(),
            };
            payload["body_b64"] = JsonValue::String(b64.encode(&bytes));
        }

        let base = self.base.clone().unwrap_or_else(auth_base);
        let endpoint = format!(
            "{}/v1/connections/{}/proxy?owner={}",
            base,
            enc(&credential),
            enc(&auth_owner())
        );
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
        // Optional response shaping: return just `result_path` (a dot path like
        // `id` or `data.0.id`) instead of the whole body. Lets a downstream call
        // consume one field (e.g. chain a create-then-act REST pair) generically.
        if let Some(path) = result_path {
            let parsed: JsonValue = serde_json::from_str(&out)
                .map_err(|e| cap_err(format!("result_path set but response is not JSON: {e}")))?;
            let picked = json_dot_path(&parsed, &path)
                .ok_or_else(|| cap_err(format!("result_path '{path}' not found in response")))?;
            return Ok(Value::String(value_to_str(&apxm_value(picked))));
        }
        Ok(Value::String(out))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

/// Render a capability arg [`Value`] as a plain string for url/body building
/// (strings unquoted; everything else via its JSON form).
fn value_to_str(v: &Value) -> String {
    if let Some(s) = v.as_string() {
        return s.clone();
    }
    serde_json::to_value(v).map(|j| match j {
        JsonValue::String(s) => s,
        other => other.to_string(),
    }).unwrap_or_default()
}

/// Render a borrowed JSON value as a string the same way (used after result_path).
fn apxm_value(j: &JsonValue) -> Value {
    match j {
        JsonValue::String(s) => Value::String(s.clone()),
        other => Value::String(other.to_string()),
    }
}

/// Navigate a dot path (`a.b.0.c`) into a JSON value. Numeric segments index
/// arrays; everything else is an object key. Returns `None` if any step misses.
fn json_dot_path<'a>(root: &'a JsonValue, path: &str) -> Option<&'a JsonValue> {
    let mut cur = root;
    for seg in path.split('.') {
        cur = match cur {
            JsonValue::Object(map) => map.get(seg)?,
            JsonValue::Array(arr) => arr.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
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

    #[test]
    fn json_dot_path_navigates_objects_and_arrays() {
        let v: JsonValue = serde_json::json!({ "id": "42", "data": [{ "x": "y" }] });
        assert_eq!(json_dot_path(&v, "id"), Some(&JsonValue::String("42".into())));
        assert_eq!(json_dot_path(&v, "data.0.x"), Some(&JsonValue::String("y".into())));
        assert_eq!(json_dot_path(&v, "missing"), None);
        assert_eq!(json_dot_path(&v, "data.9.x"), None);
    }

    /// REST mode: a captured /proxy payload shows the url template filled from a
    /// path param and the body assembled from the remaining loose args.
    #[tokio::test]
    async fn rest_mode_fills_url_template_and_builds_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = s.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let b64 = base64::engine::general_purpose::STANDARD;
            let body = format!("{{\"status\":200,\"body_b64\":\"{}\"}}", b64.encode(b"{\"id\":\"17841\"}"));
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes()).await;
            let _ = s.shutdown().await;
            req
        });
        let mut cap = ProviderCallCapability::named_rest(
            "instagram.create_media", "create", json!({}),
            "POST", "https://graph.instagram.com/v25.0/{ig_user_id}/media",
        );
        cap.base = Some(format!("http://127.0.0.1:{port}"));
        let mut args = HashMap::new();
        args.insert("credential".to_string(), Value::String("conn-1".into()));
        args.insert("ig_user_id".to_string(), Value::String("ME".into()));
        args.insert("image_url".to_string(), Value::String("https://x/a.jpg".into()));
        args.insert("caption".to_string(), Value::String("hi".into()));
        cap.execute(args).await.expect("rest call");
        let req = handle.await.unwrap();
        let line = req.lines().find(|l| l.contains("\"url\"")).unwrap_or("");
        // url template was filled (path param consumed, not in body)…
        assert!(req.contains("v25.0/ME/media"), "url should be templated: {line}");
        // …and the body carries the loose args (decoded from body_b64).
        let payload: JsonValue = serde_json::from_str(req.split("\r\n\r\n").nth(1).unwrap_or("{}")).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD;
        let body_bytes = b64.decode(payload["body_b64"].as_str().unwrap()).unwrap();
        let body: JsonValue = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["image_url"], "https://x/a.jpg");
        assert_eq!(body["caption"], "hi");
        assert!(body.get("ig_user_id").is_none(), "path param must not leak into body");
        assert!(body.get("credential").is_none(), "credential must never be in body");
    }

    /// `result_path` returns just the named field from the JSON response.
    #[tokio::test]
    async fn result_path_extracts_field() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = s.read(&mut buf).await;
            let b64 = base64::engine::general_purpose::STANDARD;
            let body = format!("{{\"status\":200,\"body_b64\":\"{}\"}}", b64.encode(b"{\"id\":\"17841\",\"other\":1}"));
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes()).await;
            let _ = s.shutdown().await;
        });
        let mut cap = ProviderCallCapability::new();
        cap.base = Some(format!("http://127.0.0.1:{port}"));
        let mut args = HashMap::new();
        args.insert("credential".to_string(), Value::String("c".into()));
        args.insert("url".to_string(), Value::String("https://api.example.com/x".into()));
        args.insert("result_path".to_string(), Value::String("id".into()));
        let out = cap.execute(args).await.expect("result_path");
        assert_eq!(out, Value::String("17841".into()), "should return just the id");
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
