//! `mcp.call` — the MCP-client bridge: the PEER backing to `provider.call`.
//!
//! Lets a workflow block invoke a tool on an external MCP server via JSON-RPC
//! `tools/call` over Streamable HTTP (MCP 2025-11-25). The capability-id stays
//! the AIR contract; this is just a second backing resolved at dispatch. Auth is
//! token-forward (v1): if a `credential` connection id is supplied, apxm-auth
//! resolves a short-lived, audience-bound token (GET /v1/connections/{id}/token)
//! which the bridge presents to the MCP server — the long-lived secret stays in
//! apxm-auth. SSRF-guarded; no redirects.
//!
//! Reserved for MCP-only providers and agent-driven dynamic tool use; the
//! deterministic per-node default remains `provider.call` (REST). Full OAuth-2.1
//! discovery (RFC 9728/8414/8707) for first-connect is an apxm-auth concern; this
//! bridge consumes the resolved token.

use super::guard_url_ssrf_pinned;
use crate::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::RuntimeCapability,
};
use apxm_core::{error::RuntimeError, types::Value};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value as JsonValue, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::OnceLock;

// This is the outbound MCP client role. It is a separate ADR-0018 trust
// boundary from APXM's managed inbound resource edge and is intentionally
// versioned independently.
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_BODY_BYTES: usize = 1_000_000;

fn shared_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none()) // SSRF: no auto-redirect
            .build()
            .unwrap_or_default()
    })
}

/// Reuse the hardened HTTP capability's address-aware SSRF guard, but keep MCP
/// transport on public HTTPS only.
async fn guard_server_url_pinned(cap: &str, raw: &str) -> CapabilityResult<Vec<SocketAddr>> {
    let deny = |m: String| RuntimeError::Capability {
        capability: cap.to_string(),
        message: m,
    };
    let url = reqwest::Url::parse(raw).map_err(|e| deny(format!("invalid url: {e}")))?;
    if url.scheme() != "https" {
        return Err(deny(format!(
            "scheme '{}' not allowed (https only)",
            url.scheme()
        )));
    }
    guard_url_ssrf_pinned(cap, raw).await
}

/// Choose a request client for the vetted MCP endpoint. When the server URL was
/// resolved by name, pin the connect to those exact vetted addresses to close
/// the DNS-rebind window while preserving the bridge's no-redirect policy.
fn mcp_client_for(url: &str, addrs: &[SocketAddr]) -> std::borrow::Cow<'static, Client> {
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
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, addrs)
        .build()
    {
        Ok(client) => std::borrow::Cow::Owned(client),
        Err(_) => std::borrow::Cow::Borrowed(shared_client()),
    }
}

fn auth_base() -> String {
    std::env::var("APXM_AUTH_URL").unwrap_or_else(|_| "http://127.0.0.1:18810".to_string())
}

/// apxm-auth requires an `owner` to scope the connection to a tenant. The
/// bearer authenticates the service; the owner scopes the tenant. Both are sent.
fn auth_owner() -> String {
    std::env::var("APXM_AUTH_OWNER").unwrap_or_else(|_| "default".to_string())
}

fn auth_bearer() -> Option<String> {
    if let Ok(value) = std::env::var("APXM_AUTH_BEARER") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let dir = std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|_| {
            std::env::var("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state"))
        })
        .ok()?;
    std::fs::read_to_string(dir.join("apxm/auth/auth.bearer"))
        .ok()
        .map(|s| s.trim().to_string())
}

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

/// Content pin of an MCP server's advertised `tools/list` (rug-pull defense).
/// Canonicalizes each tool to `name\u{1f}inputSchema` and BLAKE3-hashes the
/// sorted set, so a server silently adding/changing a tool (a "rug pull") yields
/// a different pin and can be rejected / re-consented at load time.
#[must_use]
pub fn pin_tools(tools: &JsonValue) -> String {
    let mut lines: Vec<String> = tools
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|t| {
                    let name = t.get("name").and_then(|n| n.as_str()).unwrap_or_default();
                    // Canonical JSON of the input schema (serde_json sorts object keys
                    // deterministically via BTreeMap is not guaranteed; use compact form).
                    let schema = t
                        .get("inputSchema")
                        .map(std::string::ToString::to_string)
                        .unwrap_or_default();
                    format!("{name}\u{1f}{schema}")
                })
                .collect()
        })
        .unwrap_or_default();
    lines.sort();
    let joined = lines.join("\u{1e}");
    format!("blake3:{}", blake3::hash(joined.as_bytes()).to_hex())
}

/// True if the live `tools/list` still matches the pin recorded at install.
#[must_use]
pub fn verify_tool_pin(tools: &JsonValue, pinned: &str) -> bool {
    pin_tools(tools) == pinned
}

pub struct McpBridgeCapability {
    metadata: RuntimeCapability,
    /// apxm-auth base override (tests). None = APXM_AUTH_URL.
    base: Option<String>,
    /// Baked server URL / tool name when registered per-tool (kind=mcp); when
    /// None they come from the call args (the generic `mcp.call`).
    server_url: Option<String>,
    tool: Option<String>,
}

impl Default for McpBridgeCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl McpBridgeCapability {
    pub fn new() -> Self {
        Self {
            base: None,
            server_url: None,
            tool: None,
            metadata: RuntimeCapability::new(
                "mcp.call",
                "Call a tool on an external MCP server (tools/call over Streamable HTTP)",
                json!({
                    "type": "object",
                    "properties": {
                        "server_url": { "type": "string", "description": "MCP server URL (public https only)" },
                        "tool": { "type": "string", "description": "MCP tool name to call" },
                        "arguments": { "type": "object", "description": "Tool arguments" },
                        "credential": { "type": "string", "description": "apxm-auth connection id; token-forwarded to the server" }
                    },
                    "required": ["server_url", "tool"]
                }),
            )
            .with_returns("string")
            .with_groups(vec!["mcp".to_string(), "provider".to_string()])
            .with_latency(800),
        }
    }

    /// A per-tool MCP block (kind=mcp) with the server URL + tool name baked in.
    pub fn named(
        name: impl Into<String>,
        description: impl Into<String>,
        server_url: String,
        tool: String,
    ) -> Self {
        let mut c = Self::new();
        c.metadata =
            RuntimeCapability::new(name, description, c.metadata.parameters_schema.clone())
                .with_returns("string")
                .with_groups(vec!["mcp".to_string(), "provider".to_string()])
                .with_latency(800);
        c.server_url = Some(server_url);
        c.tool = Some(tool);
        c
    }

    async fn resolve_token(&self, credential: &str) -> Option<String> {
        let base = self.base.clone().unwrap_or_else(auth_base);
        let mut req = shared_client().get(format!(
            "{}/v1/connections/{}/token?owner={}",
            base,
            enc(credential),
            enc(&auth_owner())
        ));
        if let Some(b) = auth_bearer() {
            req = req.bearer_auth(b);
        }
        let resp = req.send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json::<JsonValue>()
            .await
            .ok()?
            .get("access_token")
            .and_then(|v| v.as_str().map(String::from))
    }
}

#[async_trait]
impl CapabilityExecutor for McpBridgeCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let cap = self.metadata.name.clone();
        let cap_err = |msg: String| RuntimeError::Capability {
            capability: cap.clone(),
            message: msg,
        };
        let as_json = |k: &str| args.get(k).and_then(|v| serde_json::to_value(v).ok());

        let server_url = self
            .server_url
            .clone()
            .or_else(|| as_json("server_url").and_then(|j| j.as_str().map(String::from)))
            .ok_or_else(|| cap_err("missing `server_url`".into()))?;
        let tool = self
            .tool
            .clone()
            .or_else(|| as_json("tool").and_then(|j| j.as_str().map(String::from)))
            .ok_or_else(|| cap_err("missing `tool`".into()))?;
        let pinned = guard_server_url_pinned(&self.metadata.name, &server_url).await?;
        let arguments = as_json("arguments").unwrap_or_else(|| json!({}));

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        });
        let client = mcp_client_for(&server_url, &pinned);
        let mut req = client
            .post(&server_url)
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION)
            .header("Accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(cred) = as_json("credential").and_then(|j| j.as_str().map(String::from))
            && let Some(tok) = self.resolve_token(&cred).await
        {
            req = req.bearer_auth(tok);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| cap_err(format!("MCP request failed: {e}")))?;
        if !resp.status().is_success() {
            let s = resp.status();
            return Err(cap_err(format!(
                "MCP server returned {s}: {}",
                resp.text().await.unwrap_or_default()
            )));
        }
        let mut text = resp.text().await.unwrap_or_default();
        if text.len() > MAX_BODY_BYTES {
            text.truncate(MAX_BODY_BYTES);
        }
        let rpc: JsonValue =
            serde_json::from_str(&text).map_err(|e| cap_err(format!("MCP response parse: {e}")))?;
        if let Some(err) = rpc.get("error") {
            return Err(cap_err(format!("MCP tools/call error: {err}")));
        }
        // Extract result.content[].text (the MCP tool-result shape).
        let out = rpc
            .get("result")
            .and_then(|r| r.get("content"))
            .and_then(|c| c.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .or_else(|| rpc.get("result").map(std::string::ToString::to_string))
            .unwrap_or_default();
        Ok(Value::String(out))
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::MockServer;

    fn arg(s: &str) -> Value {
        Value::String(s.to_string())
    }

    impl McpBridgeCapability {
        fn with_base(base: impl Into<String>) -> Self {
            let mut cap = Self::new();
            cap.base = Some(base.into());
            cap
        }
    }

    #[tokio::test]
    async fn execute_rejects_metadata_and_private_server_urls() {
        let cap = McpBridgeCapability::new();

        for server_url in [
            "https://169.254.169.254/latest/meta-data/",
            "https://10.0.0.5/tools",
        ] {
            let mut args = HashMap::new();
            args.insert("server_url".into(), arg(server_url));
            args.insert("tool".into(), arg("status"));

            let err = cap
                .execute(args)
                .await
                .expect_err("blocked server_url must fail");
            let rendered = format!("{err}");
            assert!(
                rendered.contains("blocked"),
                "expected blocked-address error for {server_url}, got {rendered}"
            );
        }
    }

    #[tokio::test]
    async fn guard_accepts_public_https_server_url() {
        let addrs = guard_server_url_pinned("mcp.call", "https://example.com/")
            .await
            .expect("public https URL should pass the guard");
        assert!(
            !addrs.is_empty(),
            "DNS-backed public hosts should return pinned addrs"
        );
    }

    #[tokio::test]
    async fn rejected_server_url_does_not_resolve_credentials() {
        let auth = MockServer::start().await;
        let cap = McpBridgeCapability::with_base(auth.uri());

        let mut args = HashMap::new();
        args.insert(
            "server_url".into(),
            arg("https://169.254.169.254/latest/meta-data/"),
        );
        args.insert("tool".into(), arg("status"));
        args.insert("credential".into(), arg("conn1"));
        args.insert("arguments".into(), Value::Object(HashMap::new()));

        let err = cap
            .execute(args)
            .await
            .expect_err("guard should fail before auth resolution");
        assert!(format!("{err}").contains("blocked"));

        let requests = auth.received_requests().await.unwrap();
        assert!(
            requests.is_empty(),
            "credential resolution must not run when server_url is rejected"
        );
    }
}
