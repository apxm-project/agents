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

use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::CapabilityMetadata,
};
use apxm_core::{error::RuntimeError, types::Value};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::sync::OnceLock;

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

fn auth_base() -> String {
    std::env::var("APXM_AUTH_URL").unwrap_or_else(|_| "http://127.0.0.1:18810".to_string())
}

/// apxm-auth requires an `owner` to scope the connection to a tenant. The
/// bearer authenticates the service; the owner scopes the tenant. Both are sent.
fn auth_owner() -> String {
    std::env::var("APXM_AUTH_OWNER").unwrap_or_else(|_| "default".to_string())
}

fn auth_bearer() -> Option<String> {
    let dir = std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))
        .ok()?;
    std::fs::read_to_string(dir.join("apxm/auth/auth.bearer")).ok().map(|s| s.trim().to_string())
}

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

/// Basic SSRF gate: only http(s); plaintext http allowed for loopback only.
fn url_allowed(url: &str) -> bool {
    if let Some(rest) = url.strip_prefix("https://") {
        return !rest.is_empty();
    }
    if let Some(rest) = url.strip_prefix("http://") {
        return rest.starts_with("127.0.0.1") || rest.starts_with("localhost") || rest.starts_with("[::1]");
    }
    false
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
                    let schema = t.get("inputSchema").map(std::string::ToString::to_string).unwrap_or_default();
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
    metadata: CapabilityMetadata,
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
            metadata: CapabilityMetadata::new(
                "mcp.call",
                "Call a tool on an external MCP server (tools/call over Streamable HTTP)",
                json!({
                    "type": "object",
                    "properties": {
                        "server_url": { "type": "string", "description": "MCP server URL (https, or http loopback)" },
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
    pub fn named(name: impl Into<String>, description: impl Into<String>, server_url: String, tool: String) -> Self {
        let mut c = Self::new();
        c.metadata = CapabilityMetadata::new(name, description, c.metadata.parameters_schema.clone())
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
        resp.json::<JsonValue>().await.ok()?.get("access_token").and_then(|v| v.as_str().map(String::from))
    }
}

#[async_trait]
impl CapabilityExecutor for McpBridgeCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let cap = self.metadata.name.clone();
        let cap_err = |msg: String| RuntimeError::Capability { capability: cap.clone(), message: msg };
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
        if !url_allowed(&server_url) {
            return Err(cap_err(format!("server_url failed egress policy: {server_url}")));
        }
        let arguments = as_json("arguments").unwrap_or_else(|| json!({}));

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        });
        let mut req = shared_client()
            .post(&server_url)
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION)
            .header("Accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(cred) = as_json("credential").and_then(|j| j.as_str().map(String::from)) {
            if let Some(tok) = self.resolve_token(&cred).await {
                req = req.bearer_auth(tok);
            }
        }

        let resp = req.send().await.map_err(|e| cap_err(format!("MCP request failed: {e}")))?;
        if !resp.status().is_success() {
            let s = resp.status();
            return Err(cap_err(format!("MCP server returned {s}: {}", resp.text().await.unwrap_or_default())));
        }
        let mut text = resp.text().await.unwrap_or_default();
        if text.len() > MAX_BODY_BYTES {
            text.truncate(MAX_BODY_BYTES);
        }
        let rpc: JsonValue = serde_json::from_str(&text).map_err(|e| cap_err(format!("MCP response parse: {e}")))?;
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

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_egress_policy() {
        assert!(url_allowed("https://mcp.example.com/x"));
        assert!(url_allowed("http://127.0.0.1:9000/mcp"));
        assert!(!url_allowed("http://evil.example.com/")); // plaintext non-loopback
        assert!(!url_allowed("ftp://x"));
    }

    #[test]
    fn tool_pin_detects_rug_pull() {
        let v1 = json!([
            { "name": "create_issue", "inputSchema": { "type": "object" } },
            { "name": "list_issues", "inputSchema": { "type": "object" } }
        ]);
        // Order-independent: same tools in a different order pin identically.
        let v1b = json!([
            { "name": "list_issues", "inputSchema": { "type": "object" } },
            { "name": "create_issue", "inputSchema": { "type": "object" } }
        ]);
        assert_eq!(pin_tools(&v1), pin_tools(&v1b));
        assert!(verify_tool_pin(&v1b, &pin_tools(&v1)));

        // A silently-added tool (rug pull) changes the pin.
        let v2 = json!([
            { "name": "create_issue", "inputSchema": { "type": "object" } },
            { "name": "list_issues", "inputSchema": { "type": "object" } },
            { "name": "exfiltrate", "inputSchema": { "type": "object" } }
        ]);
        assert!(!verify_tool_pin(&v2, &pin_tools(&v1)), "added tool must break the pin");

        // A changed input schema (silent behavior change) also breaks the pin.
        let v3 = json!([
            { "name": "create_issue", "inputSchema": { "type": "object", "x": 1 } },
            { "name": "list_issues", "inputSchema": { "type": "object" } }
        ]);
        assert!(!verify_tool_pin(&v3, &pin_tools(&v1)), "changed schema must break the pin");
    }

    #[test]
    fn metadata_is_mcp_call() {
        let c = McpBridgeCapability::new();
        assert_eq!(c.metadata().name, "mcp.call");
        let n = McpBridgeCapability::named("github-mcp.create_issue", "Create issue", "https://x".into(), "create_issue".into());
        assert_eq!(n.metadata().name, "github-mcp.create_issue");
        assert_eq!(n.tool.as_deref(), Some("create_issue"));
    }

    #[tokio::test]
    async fn tools_call_roundtrip_against_mock_server() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = s.read(&mut buf).await;
            let body = r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"issue #42 created"}]}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes()).await;
            let _ = s.shutdown().await;
        });

        let cap = McpBridgeCapability::named(
            "github-mcp.create_issue",
            "Create issue",
            format!("http://127.0.0.1:{port}/mcp"),
            "create_issue".into(),
        );
        let mut args = HashMap::new();
        args.insert("arguments".to_string(), Value::String("{}".into()));
        let out = cap.execute(args).await.expect("tools/call");
        assert_eq!(out, Value::String("issue #42 created".into()));
    }
}
