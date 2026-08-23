//! `provider.call` — the generic outbound block backing every connector action.
//!
//! Forwards an authenticated HTTP call to a provider through apxm-auth's
//! inject-and-forward `/proxy`: the kernel hands apxm-auth the connection id,
//! method, url, headers and body; apxm-auth attaches the secret and forwards.
//! The secret never enters the kernel. ONE capability serves every `api_base`
//! provider, so onboarding a connector needs no per-provider Rust — a pack just
//! declares an action block whose capability is `provider.call`.

use super::{
    MAX_HEADER_COUNT, MAX_REQUEST_BODY_BYTES, MAX_URL_BYTES, auth_base, auth_bearer, auth_owner,
    collect_bounded_body, provenance_source_uri, typed_headers, untrusted_content_value,
};
use crate::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::RuntimeCapability,
};
use apxm_core::{error::RuntimeError, types::Value};
use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde_json::{Value as JsonValue, json};
use std::collections::HashMap;
use std::sync::OnceLock;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_BODY_BYTES: usize = 1_000_000;

/// Bounded inline retry on a rate-limited provider response (429 / 503+Retry-After).
const MAX_RETRY_ATTEMPTS: u32 = 3;
/// Upper bound on how long a single retry will wait, regardless of Retry-After.
const MAX_RETRY_WAIT_SECS: u64 = 30;
/// Backoff used when the provider rate-limits without a usable Retry-After.
const DEFAULT_RETRY_WAIT_SECS: u64 = 1;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 512;
const MAX_CREDENTIAL_ID_BYTES: usize = 512;

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

/// Percent-encode a path segment (the connection id).
fn enc(seg: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(seg.len());
    for b in seg.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0f) as usize] as char);
            }
        }
    }
    out
}

/// Percent-encode a query component (key or value). Like [`enc`] but also
/// escapes characters that are reserved inside a query string.
fn enc_query(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0f) as usize] as char);
            }
        }
    }
    out
}

/// GET and DELETE carry no request body; their loose args belong in the query
/// string instead. Case-insensitive.
fn method_has_no_body(method: &str) -> bool {
    method.eq_ignore_ascii_case("GET")
        || method.eq_ignore_ascii_case("HEAD")
        || method.eq_ignore_ascii_case("DELETE")
}

/// Append `pairs` to `url` as url-encoded query parameters, choosing `?` or `&`
/// based on whether the url already has a query string.
fn append_query(url: &str, pairs: &[(&String, &Value)]) -> String {
    if pairs.is_empty() {
        return url.to_string();
    }
    let mut out = url.to_string();
    let mut sep = if url.contains('?') { '&' } else { '?' };
    for (k, v) in pairs {
        out.push(sep);
        out.push_str(&enc_query(k));
        out.push('=');
        out.push_str(&enc_query(&value_to_str(v)));
        sep = '&';
    }
    out
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case(name))
}

pub struct ProviderCallCapability {
    metadata: RuntimeCapability,
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

/// Arg keys reserved for the capability itself, never forwarded as body fields.
const RESERVED_ARGS: &[&str] = &[
    "credential",
    "idempotency_key",
    "result_path",
    "method",
    "url",
    "headers",
    "body",
];

fn is_mutating_method(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    )
}

fn validate_method(method: &str) -> Result<String, String> {
    let normalized = method.trim().to_ascii_uppercase();
    if !matches!(
        normalized.as_str(),
        "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
    ) {
        return Err(format!("HTTP method '{method}' is not allowed"));
    }
    Ok(normalized)
}

fn validate_target_url(url: &str) -> Result<(), String> {
    if url.len() > MAX_URL_BYTES {
        return Err(format!("provider URL exceeds {MAX_URL_BYTES} bytes"));
    }
    let parsed =
        reqwest::Url::parse(url).map_err(|error| format!("provider URL is invalid: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("provider URL must be an absolute HTTP(S) URL".to_owned());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.fragment().is_some() {
        return Err("provider URL cannot contain credentials or a fragment".to_owned());
    }
    Ok(())
}

impl Default for ProviderCallCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderCallCapability {
    /// A named provider-backed capability (e.g. `provider.write`) with the same
    /// proxy-forwarding behaviour as `provider.call`. Used when a pack registers
    /// a `kind = "provider"` tool so `/v1/capabilities` lists the block by name.
    pub fn named(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: serde_json::Value,
    ) -> Self {
        Self {
            base: None,
            rest: None,
            metadata: RuntimeCapability::new(name, description, schema)
                .with_returns("object (untrusted quoted provider response)")
                .with_groups(vec!["provider".to_string(), "http".to_string()])
                .with_auth()
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
        cap.rest = Some(RestTemplate {
            method: method.into(),
            url: url.into(),
        });
        cap
    }

    /// Mark the capability's `requires_auth` metadata flag. A provider-backed
    /// connector block needs a bound connection, so the install-gated catalog
    /// gates the block on a connection when this is set. Additive and chainable.
    pub fn with_requires_auth(mut self, requires_auth: bool) -> Self {
        if requires_auth {
            self.metadata = self.take_metadata().with_auth();
        }
        self
    }

    /// Mark the capability read-only (safe for parallel raw-execute admission).
    /// Additive and chainable; used when a pack declares `read_only = true`.
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        if read_only {
            self.metadata = self.take_metadata().with_read_only();
        }
        self
    }

    /// Move `metadata` out for a chained `with_*` rebuild, leaving a cheap
    /// placeholder behind (immediately overwritten by the caller).
    fn take_metadata(&mut self) -> RuntimeCapability {
        std::mem::replace(
            &mut self.metadata,
            RuntimeCapability::new("", "", serde_json::Value::Null),
        )
    }

    pub fn new() -> Self {
        Self {
            base: None,
            rest: None,
            metadata: RuntimeCapability::new(
                apxm_core::constants::capabilities::PROVIDER_CALL,
                "Authenticated HTTP call to a connected provider (via apxm-auth proxy; secret stays in apxm-auth)",
                json!({
                    "type": "object",
                    "properties": {
                        "credential": { "type": "string", "description": "apxm-auth connection id; the secret never leaves apxm-auth" },
                        "idempotency_key": { "type": "string", "description": "Required for mutating methods; reused for safe retry/reconciliation" },
                        "method": { "type": "string", "description": "HTTP method (default POST)" },
                        "url": { "type": "string", "description": "Absolute provider URL (scoped to the provider api_base)" },
                        "headers": { "type": "object", "description": "Optional request headers" },
                        "body": { "description": "JSON object or string body" }
                    },
                    "required": ["credential", "url"]
                }),
            )
            .with_returns("object (untrusted quoted provider response)")
            .with_auth()
            .with_groups(vec!["provider".to_string(), "http".to_string()])
            .with_latency(500),
        }
    }

    /// Test-only: a named-REST capability pinned to an explicit apxm-auth base
    /// (a mock proxy) so the request-building + retry paths can be exercised
    /// without touching `APXM_AUTH_URL` or a live apxm-auth.
    #[cfg(test)]
    fn rest_with_base(
        base: impl Into<String>,
        method: impl Into<String>,
        url: impl Into<String>,
    ) -> Self {
        let mut cap = Self::named_rest(
            "provider.test",
            "test",
            serde_json::Value::Null,
            method,
            url,
        );
        cap.base = Some(base.into());
        cap
    }
}

#[async_trait]
impl CapabilityExecutor for ProviderCallCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let cap = self.metadata.name.clone();
        let cap_err = |msg: String| RuntimeError::Capability {
            capability: cap.clone(),
            message: msg,
        };
        let as_json = |k: &str| args.get(k).and_then(|v| serde_json::to_value(v).ok());

        let credential = as_json("credential")
            .and_then(|j| j.as_str().map(String::from))
            .ok_or_else(|| cap_err("missing `credential` (apxm-auth connection id)".into()))?;
        if credential.trim().is_empty() || credential.len() > MAX_CREDENTIAL_ID_BYTES {
            return Err(cap_err("credential connection id is invalid".into()));
        }
        let result_path = as_json("result_path").and_then(|j| j.as_str().map(String::from));
        let mut headers = typed_headers(args.get("headers")).map_err(cap_err)?;

        // Resolve method/url/body. Named REST mode fills the url template from
        // args and builds the body from loose (non-reserved) args; generic
        // provider.call takes url/method/body from args verbatim.
        let (method, url, body): (String, String, Option<JsonValue>) = if let Some(rest) =
            &self.rest
        {
            let mut url = rest.url.clone();
            let mut path_params: Vec<String> = Vec::new();
            for (k, v) in &args {
                let placeholder = format!("{{{k}}}");
                if url.contains(&placeholder) {
                    url = url.replace(&placeholder, &value_to_str(v));
                    path_params.push(k.clone());
                }
            }
            let method = as_json("method")
                .and_then(|j| j.as_str().map(String::from))
                .unwrap_or_else(|| rest.method.clone());
            // The loose args that are neither reserved nor consumed as url path
            // params. GET/DELETE have no body, so these become query parameters;
            // POST/PUT/PATCH carry them as the JSON body.
            let loose: Vec<(&String, &Value)> = args
                .iter()
                .filter(|(k, _)| !RESERVED_ARGS.contains(&k.as_str()) && !path_params.contains(k))
                .collect();
            if method_has_no_body(&method) {
                // Append loose args to the query string (url-encoded). An
                // explicit `body` is ignored for body-less methods.
                let mut pairs: Vec<(&String, &Value)> = loose;
                // Stable order so the url is deterministic (tests, caching).
                pairs.sort_by(|a, b| a.0.cmp(b.0));
                url = append_query(&url, &pairs);
                (method, url, None)
            } else {
                // Explicit `body` wins; else assemble it from the loose args.
                let body = as_json("body").or_else(|| {
                    let obj: serde_json::Map<String, JsonValue> = loose
                        .iter()
                        .filter_map(|(k, v)| {
                            serde_json::to_value(v).ok().map(|j| ((*k).clone(), j))
                        })
                        .collect();
                    (!obj.is_empty()).then_some(JsonValue::Object(obj))
                });
                (method, url, body)
            }
        } else {
            let url = as_json("url")
                .and_then(|j| j.as_str().map(String::from))
                .ok_or_else(|| cap_err("missing `url`".into()))?;
            let method = as_json("method")
                .and_then(|j| j.as_str().map(String::from))
                .unwrap_or_else(|| "POST".into());
            (method, url, as_json("body"))
        };

        let method = validate_method(&method).map_err(cap_err)?;
        validate_target_url(&url).map_err(cap_err)?;
        let idempotency_key =
            as_json("idempotency_key").and_then(|j| j.as_str().map(str::to_owned));
        let idempotency_key = if is_mutating_method(&method) {
            let key = idempotency_key
                .filter(|key| !key.trim().is_empty() && key.len() <= MAX_IDEMPOTENCY_KEY_BYTES)
                .ok_or_else(|| {
                    cap_err(format!(
                        "mutating provider method {method} requires an idempotency_key"
                    ))
                })?;
            Some(key)
        } else {
            idempotency_key.filter(|key| !key.trim().is_empty())
        };

        let b64 = base64::engine::general_purpose::STANDARD;
        let body_b64 = if let Some(body) = body {
            if !matches!(body, JsonValue::String(_)) && !has_header(&headers, "content-type") {
                if headers.len() >= MAX_HEADER_COUNT {
                    return Err(cap_err(format!(
                        "headers exceed {MAX_HEADER_COUNT} entries"
                    )));
                }
                headers.push(("Content-Type".to_string(), "application/json".to_string()));
            }
            let bytes = match body {
                JsonValue::String(s) => s.into_bytes(),
                other => other.to_string().into_bytes(),
            };
            if bytes.len() > MAX_REQUEST_BODY_BYTES {
                return Err(cap_err(format!(
                    "provider request body exceeds {MAX_REQUEST_BODY_BYTES} bytes"
                )));
            }
            Some(b64.encode(&bytes))
        } else {
            None
        };
        let mut payload = json!({ "method": method, "url": url, "headers": headers });
        if let Some(body_b64) = body_b64 {
            payload["body_b64"] = JsonValue::String(body_b64);
        }
        if let Some(idempotency_key) = &idempotency_key {
            payload["idempotency_key"] = JsonValue::String(idempotency_key.clone());
        }

        let base = self.base.clone().unwrap_or_else(auth_base);
        let endpoint = format!(
            "{}/v1/connections/{}/proxy?owner={}",
            base,
            enc(&credential),
            enc(&auth_owner())
        );
        let bearer = auth_bearer();

        // Forward to apxm-auth, retrying inline on a rate-limited provider
        // response (429, or 503 carrying Retry-After). Bounded: at most
        // MAX_RETRY_ATTEMPTS tries, honouring the upstream Retry-After when
        // present (capped) else a small backoff. Never busy-loops.
        let pr: ProxyResp = {
            let mut attempt: u32 = 0;
            loop {
                let mut req = shared_client().post(&endpoint).json(&payload);
                if let Some(b) = &bearer {
                    req = req.bearer_auth(b);
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| cap_err(format!("apxm-auth proxy request failed: {e}")))?;
                if !resp.status().is_success() {
                    let s = resp.status();
                    return Err(cap_err(format!("apxm-auth proxy returned status {s}")));
                }
                let response_body = collect_bounded_body(resp, MAX_BODY_BYTES)
                    .await
                    .map_err(|e| cap_err(format!("proxy response unavailable: {e}")))?;
                let pr: ProxyResp = serde_json::from_slice(&response_body)
                    .map_err(|e| cap_err(format!("proxy response parse: {e}")))?;

                let retryable = (pr.status == 429
                    || (pr.status == 503 && pr.retry_after.is_some()))
                    && (!is_mutating_method(&method) || idempotency_key.is_some());
                if retryable && attempt + 1 < MAX_RETRY_ATTEMPTS {
                    let wait = retry_after_delay(pr.retry_after.as_deref());
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                    continue;
                }
                break pr;
            }
        };

        if pr.status >= 400 {
            return Err(cap_err(format!("provider returned status {}", pr.status)));
        }

        let body_b64 = pr.body_b64.unwrap_or_default();
        if body_b64.len() > MAX_BODY_BYTES {
            return Err(cap_err(format!(
                "provider response body exceeds {MAX_BODY_BYTES} bytes"
            )));
        }
        let bytes = b64
            .decode(body_b64)
            .map_err(|e| cap_err(format!("body_b64 decode: {e}")))?;
        if bytes.len() > MAX_BODY_BYTES {
            return Err(cap_err(format!(
                "provider response body exceeds {MAX_BODY_BYTES} bytes"
            )));
        }
        let out = String::from_utf8_lossy(&bytes).into_owned();
        // Optional response shaping: return just `result_path` (a dot path like
        // `id` or `data.0.id`) instead of the whole body. Lets a downstream call
        // consume one field (e.g. chain a create-then-act REST pair) generically.
        if let Some(path) = result_path {
            let parsed: JsonValue = serde_json::from_str(&out)
                .map_err(|e| cap_err(format!("result_path set but response is not JSON: {e}")))?;
            let picked = json_dot_path(&parsed, &path)
                .ok_or_else(|| cap_err(format!("result_path '{path}' not found in response")))?;
            return untrusted_content_value(
                &cap,
                provenance_source_uri(&url),
                value_to_str(&apxm_value(picked)),
            );
        }
        untrusted_content_value(&cap, provenance_source_uri(&url), out)
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

/// Render a capability arg [`Value`] as a plain string for url/body building
/// (strings unquoted; everything else via its JSON form).
fn value_to_str(v: &Value) -> String {
    if let Some(s) = v.as_string() {
        return s.clone();
    }
    serde_json::to_value(v)
        .map(|j| match j {
            JsonValue::String(s) => s,
            other => other.to_string(),
        })
        .unwrap_or_default()
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
    /// Upstream `Retry-After` returned by apxm-auth: delta-seconds or HTTP-date.
    #[serde(default)]
    retry_after: Option<String>,
}

/// Decide how long to wait before a retry from an upstream `Retry-After` value.
/// Accepts delta-seconds (`"120"`) or an HTTP-date; falls back to a small
/// default when absent/unparseable; always capped at [`MAX_RETRY_WAIT_SECS`].
fn retry_after_delay(retry_after: Option<&str>) -> std::time::Duration {
    let secs = retry_after
        .and_then(parse_retry_after_secs)
        .unwrap_or(DEFAULT_RETRY_WAIT_SECS)
        .min(MAX_RETRY_WAIT_SECS);
    std::time::Duration::from_secs(secs)
}

/// Parse a `Retry-After` header value into seconds-from-now. Supports
/// delta-seconds and the IMF-fixdate / RFC1123 HTTP-date form. Returns `None`
/// when the value cannot be interpreted (caller substitutes a default).
fn parse_retry_after_secs(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(secs);
    }
    // HTTP-date (RFC 7231 IMF-fixdate, e.g. "Wed, 21 Oct 2015 07:28:00 GMT").
    let when = chrono::DateTime::parse_from_rfc2822(raw).ok()?;
    let delta = when.timestamp() - chrono::Utc::now().timestamp();
    // A past date means retry immediately (0s).
    Some(delta.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use serde_json::json;
    use wiremock::matchers::{method as m_method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn arg(s: &str) -> Value {
        Value::String(s.to_string())
    }

    /// The inner request the kernel posts to apxm-auth's `/proxy`: it carries the
    /// provider method/url and (for body methods) a base64 body.
    #[derive(serde::Deserialize)]
    struct InnerReq {
        url: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
        #[serde(default)]
        body_b64: Option<String>,
        #[serde(default)]
        idempotency_key: Option<String>,
    }

    fn inner(req: &Request) -> InnerReq {
        serde_json::from_slice(&req.body).expect("inner proxy payload is JSON")
    }

    // (A) GET: loose args go to the query string, never the body.
    #[tokio::test]
    async fn get_loose_args_go_to_query_string_not_body() {
        let server = MockServer::start().await;
        Mock::given(m_method("POST"))
            .and(path("/v1/connections/conn1/proxy"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": 200,
                "body_b64": STANDARD.encode(b"ok"),
            })))
            .mount(&server)
            .await;

        let cap = ProviderCallCapability::rest_with_base(
            server.uri(),
            "GET",
            "https://api.example.com/repos/{owner}/issues",
        );
        let mut args = HashMap::new();
        args.insert("credential".into(), arg("conn1"));
        args.insert("owner".into(), arg("octocat")); // url placeholder
        args.insert("state".into(), arg("open")); // loose -> query
        args.insert("labels".into(), arg("a b")); // needs url-encoding

        let out = cap.execute(args).await.expect("GET succeeds");
        let wire = serde_json::to_value(out).expect("provider output envelope");
        assert_eq!(wire["items"][0]["content"], "ok");

        let reqs = server.received_requests().await.unwrap();
        let inner = inner(&reqs[0]);
        assert!(
            inner.body_b64.is_none(),
            "GET must not send a body, got {:?}",
            inner.body_b64
        );
        // Placeholder filled; loose args appended as query, url-encoded, sorted.
        assert_eq!(
            inner.url,
            "https://api.example.com/repos/octocat/issues?labels=a%20b&state=open"
        );
    }

    // (A) POST: loose args become the JSON body, not the query string.
    #[tokio::test]
    async fn post_loose_args_go_to_body_not_query() {
        let server = MockServer::start().await;
        Mock::given(m_method("POST"))
            .and(path("/v1/connections/conn1/proxy"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": 200,
                "body_b64": STANDARD.encode(b"created"),
            })))
            .mount(&server)
            .await;

        let cap = ProviderCallCapability::rest_with_base(
            server.uri(),
            "POST",
            "https://api.example.com/repos/{owner}/issues",
        );
        let mut args = HashMap::new();
        args.insert("credential".into(), arg("conn1"));
        args.insert("idempotency_key".into(), arg("create-issue-1"));
        args.insert("owner".into(), arg("octocat"));
        args.insert("title".into(), arg("bug"));

        cap.execute(args).await.expect("POST succeeds");

        let reqs = server.received_requests().await.unwrap();
        let inner = inner(&reqs[0]);
        assert_eq!(
            inner.url, "https://api.example.com/repos/octocat/issues",
            "POST url keeps no query string"
        );
        assert!(
            inner.headers.iter().any(|(key, value)| {
                key.eq_ignore_ascii_case("content-type") && value == "application/json"
            }),
            "JSON body requests must carry a content type"
        );
        assert_eq!(inner.idempotency_key.as_deref(), Some("create-issue-1"));
        let body = inner.body_b64.expect("POST sends a body");
        let decoded = String::from_utf8(STANDARD.decode(body).unwrap()).unwrap();
        let v: JsonValue = serde_json::from_str(&decoded).unwrap();
        assert_eq!(v["title"], json!("bug"));
        assert!(
            v.get("owner").is_none(),
            "url placeholder is not in the body"
        );
    }

    #[tokio::test]
    async fn mutating_provider_call_without_idempotency_key_fails_before_proxy() {
        let server = MockServer::start().await;
        let cap = ProviderCallCapability::rest_with_base(
            server.uri(),
            "POST",
            "https://api.example.com/thing",
        );
        let args = HashMap::from([(String::from("credential"), arg("conn1"))]);
        let error = cap
            .execute(args)
            .await
            .expect_err("mutating provider calls require an idempotency key");
        assert!(format!("{error}").contains("idempotency_key"));
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "validation must happen before the auth proxy"
        );
    }

    // (B) A 429 with Retry-After triggers a bounded inline retry, then succeeds.
    #[tokio::test]
    async fn rate_limited_429_retries_then_succeeds() {
        let server = MockServer::start().await;
        // First reply: provider rate-limited (429) with an immediate Retry-After.
        Mock::given(m_method("POST"))
            .and(path("/v1/connections/conn1/proxy"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": 429,
                "retry_after": "0",
                "body_b64": STANDARD.encode(b"slow down"),
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Subsequent reply: success.
        Mock::given(m_method("POST"))
            .and(path("/v1/connections/conn1/proxy"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": 200,
                "body_b64": STANDARD.encode(b"done"),
            })))
            .mount(&server)
            .await;

        let cap = ProviderCallCapability::rest_with_base(
            server.uri(),
            "GET",
            "https://api.example.com/thing",
        );
        let mut args = HashMap::new();
        args.insert("credential".into(), arg("conn1"));

        let out = cap.execute(args).await.expect("retry then success");
        let wire = serde_json::to_value(out).expect("provider output envelope");
        assert_eq!(wire["items"][0]["content"], "done");

        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2, "one retry after the 429");
    }

    // (B) Persistent 429 returns the provider error after the bounded cap.
    #[tokio::test]
    async fn persistent_429_errors_after_cap() {
        let server = MockServer::start().await;
        Mock::given(m_method("POST"))
            .and(path("/v1/connections/conn1/proxy"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": 429,
                "retry_after": "0",
                "body_b64": STANDARD.encode(b"slow down"),
            })))
            .mount(&server)
            .await;

        let cap = ProviderCallCapability::rest_with_base(
            server.uri(),
            "GET",
            "https://api.example.com/thing",
        );
        let mut args = HashMap::new();
        args.insert("credential".into(), arg("conn1"));

        let err = cap.execute(args).await.expect_err("still rate-limited");
        assert!(format!("{err}").contains("429"));

        let reqs = server.received_requests().await.unwrap();
        assert_eq!(
            reqs.len(),
            MAX_RETRY_ATTEMPTS as usize,
            "bounded at MAX_RETRY_ATTEMPTS"
        );
    }

    #[tokio::test]
    async fn proxy_error_body_is_not_exposed() {
        let server = MockServer::start().await;
        Mock::given(m_method("POST"))
            .and(path("/v1/connections/conn1/proxy"))
            .respond_with(ResponseTemplate::new(500).set_body_string("secret-proxy-body"))
            .mount(&server)
            .await;

        let cap = ProviderCallCapability::rest_with_base(
            server.uri(),
            "GET",
            "https://api.example.com/thing",
        );
        let mut args = HashMap::new();
        args.insert("credential".into(), arg("conn1"));

        let error = cap
            .execute(args)
            .await
            .expect_err("proxy failure is an error");
        let rendered = format!("{error}");
        assert!(rendered.contains("500"));
        assert!(!rendered.contains("secret-proxy-body"));
    }

    #[test]
    fn retry_after_parsing_and_cap() {
        assert_eq!(parse_retry_after_secs("5"), Some(5));
        assert_eq!(parse_retry_after_secs("0"), Some(0));
        assert_eq!(parse_retry_after_secs("garbage"), None);
        // A past HTTP-date clamps to 0.
        assert_eq!(
            parse_retry_after_secs("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(0)
        );
        // Honor header, but never exceed the cap.
        assert_eq!(
            retry_after_delay(Some("9999")).as_secs(),
            MAX_RETRY_WAIT_SECS
        );
        // No/garbage header falls back to the small default.
        assert_eq!(retry_after_delay(None).as_secs(), DEFAULT_RETRY_WAIT_SECS);
    }
}
