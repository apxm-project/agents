//! Built-in HTTP capabilities: `http_get` and `http_post`.
//!
//! Let workflow `inv_cap` nodes call HTTP/JSON APIs (e.g. public REST
//! endpoints) directly. Read-only `http_get` and mutating `http_post`. Auth
//! tokens arrive as a `headers` object (resolved from a credential by the
//! caller), never inlined by the compiler. Returns the response body as a
//! string.

use super::{
    MAX_HEADER_COUNT, MAX_REQUEST_BODY_BYTES, MAX_URL_BYTES, collect_bounded_body,
    provenance_source_uri, require_string_arg, typed_headers, untrusted_content_value,
};
use crate::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::RuntimeCapability,
};
use apxm_core::{constants::capabilities, error::RuntimeError, types::Value};
use async_trait::async_trait;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::json;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_BODY_BYTES: usize = 1_000_000;

fn validate_request_url(capability: &str, raw: &str) -> CapabilityResult<()> {
    if raw.len() > MAX_URL_BYTES {
        return Err(RuntimeError::Capability {
            capability: capability.to_owned(),
            message: format!("URL exceeds {MAX_URL_BYTES} bytes"),
        });
    }
    let parsed = reqwest::Url::parse(raw).map_err(|error| RuntimeError::Capability {
        capability: capability.to_owned(),
        message: format!("invalid URL: {error}"),
    })?;
    if parsed.host_str().is_none()
        || !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(RuntimeError::Capability {
            capability: capability.to_owned(),
            message: "URL must be an absolute HTTP(S) URL without credentials or a fragment"
                .to_owned(),
        });
    }
    Ok(())
}

/// Explicit destination policy for outbound HTTP capabilities.
///
/// SSRF filtering protects private network ranges, but it does not stop a
/// caller from sending local data to an arbitrary public endpoint. Empty
/// `allowed_hosts` is deny-by-default; hosts opt in exact names or a
/// subdomain suffix beginning with `.`.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct HttpConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_hosts: Vec::new(),
        }
    }
}

fn host_allowed(capability: &str, config: &HttpConfig, raw: &str) -> CapabilityResult<()> {
    let url = reqwest::Url::parse(raw).map_err(|error| RuntimeError::Capability {
        capability: capability.to_owned(),
        message: format!("invalid url: {error}"),
    })?;
    let host = url.host_str().ok_or_else(|| RuntimeError::Capability {
        capability: capability.to_owned(),
        message: "url has no host".to_owned(),
    })?;
    let host = host.to_ascii_lowercase();
    let allowed = config.allowed_hosts.iter().any(|entry| {
        let entry = entry.trim().trim_end_matches('.').to_ascii_lowercase();
        if entry.is_empty() {
            return false;
        }
        entry.strip_prefix('.').map_or(host == entry, |suffix| {
            host.ends_with(&format!(".{suffix}"))
        })
    });
    if allowed {
        Ok(())
    } else {
        Err(RuntimeError::Capability {
            capability: capability.to_owned(),
            message: format!("HTTP destination '{host}' is not allowed by host policy"),
        })
    }
}

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
            v6.to_ipv4().is_some_and(|v4| is_blocked_ip(IpAddr::V4(v4)))
                || v6.is_loopback()
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

/// Redirect policy shared by every tool HTTP client. Redirect targets are not
/// re-resolved and SSRF-vetted by this boundary, so redirects stop at the
/// original response rather than creating a second unpinned request.
fn hardened_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| attempt.stop())
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
pub fn client_for(
    url: &str,
    addrs: &[SocketAddr],
) -> Result<std::borrow::Cow<'static, Client>, String> {
    if addrs.is_empty() {
        return Ok(std::borrow::Cow::Borrowed(shared_client()));
    }
    let Some(host) = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
    else {
        return Err("pinned request URL has no valid host".to_string());
    };
    match Client::builder()
        .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .redirect(hardened_redirect_policy())
        .resolve_to_addrs(&host, addrs)
        .build()
    {
        Ok(client) => Ok(std::borrow::Cow::Owned(client)),
        Err(error) => Err(format!("could not build DNS-pinned client: {error}")),
    }
}

/// One process-wide HTTP client, built lazily on first use (never at startup /
/// capability construction — building a reqwest client eagerly during runtime
/// setup wedged the executor's completion path).
pub fn shared_client() -> &'static Client {
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
fn header_map(args: &HashMap<String, Value>, capability: &str) -> CapabilityResult<HeaderMap> {
    let mut map = HeaderMap::new();
    for (name, value) in
        typed_headers(args.get("headers")).map_err(|message| RuntimeError::Capability {
            capability: capability.to_owned(),
            message,
        })?
    {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
            RuntimeError::Capability {
                capability: capability.to_owned(),
                message: format!("invalid header name: {error}"),
            }
        })?;
        let value = reqwest::header::HeaderValue::from_str(&value).map_err(|error| {
            RuntimeError::Capability {
                capability: capability.to_owned(),
                message: format!("invalid header value: {error}"),
            }
        })?;
        map.insert(name, value);
    }
    Ok(map)
}

async fn finish(
    cap: &str,
    source_uri: &str,
    resp: Result<reqwest::Response, reqwest::Error>,
) -> CapabilityResult<Value> {
    let resp = resp.map_err(|e| RuntimeError::Capability {
        capability: cap.to_string(),
        message: format!("request failed: {e}"),
    })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(RuntimeError::Capability {
            capability: cap.to_string(),
            message: format!("HTTP request returned status {status}"),
        });
    }
    let body = collect_bounded_body(resp, MAX_BODY_BYTES)
        .await
        .map_err(|error| RuntimeError::Capability {
            capability: cap.to_string(),
            message: format!("HTTP response unavailable: {error}"),
        })?;
    untrusted_content_value(
        cap,
        provenance_source_uri(source_uri),
        String::from_utf8_lossy(&body).into_owned(),
    )
}

/// `http_get(url, headers?)` — fetch a URL, return the response body.
pub struct HttpGetCapability {
    metadata: RuntimeCapability,
    config: HttpConfig,
}

impl Default for HttpGetCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpGetCapability {
    pub fn new() -> Self {
        Self::with_config(HttpConfig::default())
    }

    pub fn with_config(config: HttpConfig) -> Self {
        Self {
            metadata: RuntimeCapability::new(
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
            .with_returns("object (untrusted quoted HTTP response)")
            .with_read_only()
            .with_groups(vec![
                capabilities::groups::HTTP.to_string(),
                capabilities::groups::WEB.to_string(),
            ])
            .with_latency(300),
            config,
        }
    }
}

#[async_trait]
impl CapabilityExecutor for HttpGetCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let url = require_string_arg(&args, "url", &self.metadata.name)?.to_string();
        validate_request_url(&self.metadata.name, &url)?;
        host_allowed(&self.metadata.name, &self.config, &url)?;
        let pinned = guard_url_ssrf_pinned(&self.metadata.name, &url).await?;
        let client = client_for(&url, &pinned).map_err(|message| RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message,
        })?;
        let headers = header_map(&args, &self.metadata.name)?;
        let resp = client.get(&url).headers(headers).send().await;
        finish(&self.metadata.name, &url, resp).await
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

/// `http_post(url, body?, headers?)` — POST a JSON/string body, return the body.
pub struct HttpPostCapability {
    metadata: RuntimeCapability,
    config: HttpConfig,
}

impl Default for HttpPostCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpPostCapability {
    pub fn new() -> Self {
        Self::with_config(HttpConfig::default())
    }

    pub fn with_config(config: HttpConfig) -> Self {
        Self {
            metadata: RuntimeCapability::new(
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
            .with_returns("object (untrusted quoted HTTP response)")
            .with_groups(vec![
                capabilities::groups::HTTP.to_string(),
                capabilities::groups::WEB.to_string(),
            ])
            .with_latency(400),
            config,
        }
    }
}

#[async_trait]
impl CapabilityExecutor for HttpPostCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let url = require_string_arg(&args, "url", &self.metadata.name)?.to_string();
        validate_request_url(&self.metadata.name, &url)?;
        host_allowed(&self.metadata.name, &self.config, &url)?;
        let pinned = guard_url_ssrf_pinned(&self.metadata.name, &url).await?;
        let client = client_for(&url, &pinned).map_err(|message| RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message,
        })?;
        let mut headers = header_map(&args, &self.metadata.name)?;
        let mut request_body = None;
        if let Some(body) = args.get("body") {
            let bytes =
                match serde_json::to_value(body).map_err(|error| RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!("request body is not serializable: {error}"),
                })? {
                    serde_json::Value::String(s) => s.into_bytes(),
                    jv => {
                        if headers.len() >= MAX_HEADER_COUNT {
                            return Err(RuntimeError::Capability {
                                capability: self.metadata.name.clone(),
                                message: format!("headers exceed {MAX_HEADER_COUNT} entries"),
                            });
                        }
                        headers.insert(
                            reqwest::header::CONTENT_TYPE,
                            HeaderValue::from_static("application/json"),
                        );
                        jv.to_string().into_bytes()
                    }
                };
            if bytes.len() > MAX_REQUEST_BODY_BYTES {
                return Err(RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!("request body exceeds {MAX_REQUEST_BODY_BYTES} bytes"),
                });
            }
            request_body = Some(bytes);
        }
        let mut req = client.post(&url).headers(headers);
        if let Some(body) = request_body {
            req = req.body(body);
        }
        finish(&self.metadata.name, &url, req.send().await).await
    }

    fn metadata(&self) -> &RuntimeCapability {
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
        assert!(is_blocked_ip("::ffff:127.0.0.1".parse().unwrap()));
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

    /// Defence-in-depth: a public-looking endpoint that 302s to any target must
    /// not be followed by the shared client. The final response remains the
    /// 302, so a redirect hostname cannot bypass the initial SSRF guard.
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

    #[tokio::test]
    async fn response_errors_do_not_include_remote_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(500).set_body_string("secret-response-body"))
            .mount(&server)
            .await;

        let response = shared_client()
            .get(server.uri())
            .send()
            .await
            .expect("mock response");
        let error = finish("http_test", "https://example.test/", Ok(response))
            .await
            .expect_err("status failures are errors");
        let rendered = format!("{error}");
        assert!(rendered.contains("500"));
        assert!(!rendered.contains("secret-response-body"));
    }

    #[tokio::test]
    async fn oversized_multibyte_response_is_rejected_without_panicking() {
        let server = MockServer::start().await;
        let body = "é".repeat((MAX_BODY_BYTES / 2) + 1);
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let response = shared_client()
            .get(server.uri())
            .send()
            .await
            .expect("mock response");
        let error = finish("http_test", "https://example.test/", Ok(response))
            .await
            .expect_err("oversized responses are rejected");
        assert!(format!("{error}").contains("exceeds 1000000 bytes"));
    }

    #[test]
    fn malformed_pinned_url_fails_closed() {
        let address: SocketAddr = "8.8.8.8:443".parse().expect("valid test address");
        assert!(client_for("not a URL", &[address]).is_err());
    }

    #[test]
    fn outbound_host_policy_is_explicit_and_boundary_aware() {
        let denied = HttpConfig::default();
        assert!(host_allowed("http_get", &denied, "https://example.com/").is_err());

        let allowed = HttpConfig {
            allowed_hosts: vec![".example.com".to_owned()],
            ..HttpConfig::default()
        };
        assert!(host_allowed("http_get", &allowed, "https://api.example.com/").is_ok());
        assert!(host_allowed("http_get", &allowed, "https://example.com.evil/").is_err());
    }

    #[tokio::test]
    async fn http_get_rejects_unlisted_destination_before_network_access() {
        let capability = HttpGetCapability::new();
        let error = capability
            .execute(HashMap::from([(
                "url".to_owned(),
                Value::String("https://example.com/exfil".to_owned()),
            )]))
            .await
            .expect_err("default HTTP policy must deny arbitrary destinations");
        assert!(format!("{error}").contains("not allowed by host policy"));
    }
}
