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
use std::net::{IpAddr, SocketAddr};
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
/// private IP is rejected too. Validation-only — use [`guard_url_ssrf_pinned`]
/// when the resolved address must be pinned for the actual connect.
pub async fn guard_url_ssrf(cap: &str, raw: &str) -> CapabilityResult<()> {
    guard_url_ssrf_pinned(cap, raw).await.map(|_| ())
}

/// SSRF guard that also returns the vetted, resolved socket addresses for the
/// URL's host. Pinning these on the request (via [`pin_resolved_addrs`]) closes
/// the DNS-rebind TOCTOU window: the host is resolved + vetted ONCE here, and the
/// connect is forced to one of those exact addresses rather than re-resolving the
/// name (which an attacker could have rebound to a blocked IP in between).
///
/// IP-literal URLs resolve to themselves (the vet still applies); no addresses
/// are returned for a literal because reqwest connects to the literal directly
/// and there is no name to rebind.
pub async fn guard_url_ssrf_pinned(cap: &str, raw: &str) -> CapabilityResult<Vec<SocketAddr>> {
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
    let port = url.port_or_known_default().unwrap_or(443);
    // For an IP literal there is no name to rebind; vet the literal and return no
    // pin (reqwest connects to the literal directly). For a name, resolve once
    // and return the vetted socket addrs so the connect is pinned to them.
    let (ips, pinned): (Vec<IpAddr>, Vec<SocketAddr>) = if let Ok(ip) = host.parse::<IpAddr>() {
        (vec![ip], Vec::new())
    } else {
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| deny(format!("could not resolve host '{host}': {e}")))?
            .collect();
        (addrs.iter().map(|sa| sa.ip()).collect(), addrs)
    };
    if ips.is_empty() || ips.iter().copied().any(is_blocked_ip) {
        return Err(deny(format!(
            "host '{host}' resolves to a blocked (loopback/private/link-local) address"
        )));
    }
    Ok(pinned)
}

/// Redirect policy shared by every tool HTTP client: cap the redirect chain and
/// refuse any hop whose host is a blocked IP literal (defence-in-depth against
/// redirect-to-metadata SSRF).
fn hardened_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
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
    })
}

/// Choose the client to issue a tool HTTP request on, pinning DNS resolution to
/// the vetted addresses from [`guard_url_ssrf_pinned`] when there are any.
///
/// Pinning closes the DNS-rebind TOCTOU window: rather than letting reqwest
/// re-resolve the host name at connect time (which an attacker could rebind to a
/// blocked IP after the guard ran), the returned client resolves `host` to the
/// exact addresses we already vetted. The Host header + TLS SNI are unchanged
/// (`.resolve_to_addrs` overrides only the name→address mapping), so virtual
/// hosting and certificate validation still work. An IP-literal URL (empty
/// `addrs`) reuses the shared client — there is no name to rebind.
pub(crate) fn client_for(url: &str, addrs: &[SocketAddr]) -> std::borrow::Cow<'static, Client> {
    if addrs.is_empty() {
        return std::borrow::Cow::Borrowed(shared_client());
    }
    let Some(host) = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
    else {
        return std::borrow::Cow::Borrowed(shared_client());
    };
    match Client::builder()
        .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .redirect(hardened_redirect_policy())
        .resolve_to_addrs(&host, addrs)
        .build()
    {
        Ok(client) => std::borrow::Cow::Owned(client),
        // Fall back to the shared client (still SSRF-guarded by the pre-flight
        // resolve + the redirect policy) if the pinned client fails to build.
        Err(_) => std::borrow::Cow::Borrowed(shared_client()),
    }
}

/// One process-wide HTTP client, built lazily on first use (never at startup /
/// capability construction — building a reqwest client eagerly during runtime
/// setup wedged the executor's completion path).
pub(crate) fn shared_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .redirect(hardened_redirect_policy())
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
                    if let (Ok(name), Ok(val)) = (
                        HeaderName::from_bytes(k.as_bytes()),
                        HeaderValue::from_str(s),
                    ) {
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
        let pinned = guard_url_ssrf_pinned(&self.metadata.name, &url).await?;
        let client = client_for(&url, &pinned);
        let resp = client.get(&url).headers(header_map(&args)).send().await;
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
        let pinned = guard_url_ssrf_pinned(&self.metadata.name, &url).await?;
        let client = client_for(&url, &pinned);
        let mut req = client.post(&url).headers(header_map(&args));
        if let Some(body) = args.get("body") {
            match serde_json::to_value(body) {
                Ok(serde_json::Value::String(s)) => req = req.body(s),
                Ok(jv) => {
                    req = req
                        .header("content-type", "application/json")
                        .body(jv.to_string())
                }
                Err(_) => {}
            }
        }
        finish(&self.metadata.name, req.send().await).await
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

#[cfg(test)]
mod ssrf_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn blocked_ip_literals_are_rejected() {
        assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("169.254.169.254".parse().unwrap())); // cloud metadata
        assert!(is_blocked_ip("10.0.0.5".parse().unwrap()));
        assert!(is_blocked_ip("::1".parse().unwrap()));
        assert!(!is_blocked_ip("8.8.8.8".parse().unwrap()));
    }

    #[tokio::test]
    async fn pinned_guard_returns_addrs_for_name_and_none_for_literal() {
        // A resolvable public name yields one or more vetted socket addrs to pin.
        let addrs = guard_url_ssrf_pinned("test", "https://example.com/")
            .await
            .expect("public name vets");
        assert!(
            !addrs.is_empty(),
            "a resolved name returns addrs to pin the connect to"
        );
        assert!(
            addrs.iter().all(|sa| !is_blocked_ip(sa.ip())),
            "every pinned addr passed the SSRF vet"
        );

        // An IP literal returns no pin (reqwest connects to the literal directly)
        // but still passes the vet.
        let literal = guard_url_ssrf_pinned("test", "http://8.8.8.8/")
            .await
            .expect("public literal vets");
        assert!(literal.is_empty(), "an IP literal needs no resolution pin");
    }

    #[tokio::test]
    async fn pinned_guard_blocks_metadata_url() {
        assert!(
            guard_url_ssrf_pinned("test", "http://169.254.169.254/latest/meta-data/")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn guard_blocks_metadata_url() {
        assert!(
            guard_url_ssrf("test", "http://169.254.169.254/latest/meta-data/")
                .await
                .is_err()
        );
        assert!(
            guard_url_ssrf("test", "http://127.0.0.1:8080/")
                .await
                .is_err()
        );
    }

    /// Defence-in-depth: a public-looking endpoint that 302s to a blocked IP
    /// literal (cloud metadata) must NOT be followed by the shared client.
    /// `Policy::custom` calls `attempt.stop()` on the blocked hop, so the final
    /// response is the 302 itself — the request never reaches 169.254.169.254.
    #[tokio::test]
    async fn redirect_to_blocked_ip_is_not_followed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "http://169.254.169.254/latest/meta-data/"),
            )
            .mount(&server)
            .await;

        let resp = shared_client()
            .get(server.uri())
            .send()
            .await
            .expect("request to mock should resolve");

        // Redirect was stopped, not followed: we see the 302 status, not a 200
        // from the metadata endpoint (which is unreachable / would differ).
        assert_eq!(resp.status().as_u16(), 302);
    }
}
