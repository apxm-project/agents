use std::collections::HashMap;
use std::sync::Arc;

use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use apxm_runtime::capability::executor::{CapabilityExecutor, CapabilityResult};
use apxm_runtime::capability::metadata::CapabilityMetadata;
use async_trait::async_trait;
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use tracing::info;

use crate::error::ApiError;
use crate::state::AppState;
use crate::types::responses::{CapabilityEntry, OkAckName};

/// A capability that forwards invocations to an external HTTP endpoint.
///
/// The endpoint receives the capability arguments as a JSON object via POST and
/// must return a JSON object with a `"result"` key (or any parseable JSON value).
#[derive(Clone)]
pub(crate) struct HttpCapability {
    pub(crate) metadata: CapabilityMetadata,
    pub(crate) endpoint: String,
    pub(crate) timeout_ms: u64,
    /// Static headers attached to every forwarded call (e.g. an
    /// `Authorization: Bearer <token>` so the endpoint can authenticate the
    /// callback). Empty by default — loopback endpoints need no auth.
    pub(crate) headers: HashMap<String, String>,
    pub(crate) client: reqwest::Client,
}

#[async_trait]
impl CapabilityExecutor for HttpCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        // Serialize arguments to JSON
        let body: serde_json::Map<String, JsonValue> = args
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.to_json()
                        .unwrap_or_else(|_| JsonValue::String(v.to_string())),
                )
            })
            .collect();

        let cap_err = |msg: String| RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message: msg,
        };

        let mut req_builder = self
            .client
            .post(&self.endpoint)
            .timeout(std::time::Duration::from_millis(self.timeout_ms))
            .json(&body);
        for (name, value) in &self.headers {
            req_builder = req_builder.header(name.as_str(), value.as_str());
        }
        let resp = req_builder
            .send()
            .await
            .map_err(|e| cap_err(format!("request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(cap_err(format!("endpoint returned {status}: {text}")));
        }

        let result: JsonValue = resp
            .json()
            .await
            .map_err(|e| cap_err(format!("response parse failed: {e}")))?;

        // Unwrap a `{"result": ...}` envelope if present, otherwise use the raw response
        let inner = if let Some(r) = result.get("result") {
            r.clone()
        } else {
            result
        };

        Value::try_from(inner).map_err(|e| cap_err(format!("value conversion failed: {e}")))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegisterCapabilityRequest {
    name: String,
    description: String,
    #[serde(default)]
    parameters_schema: JsonValue,
    /// Mark capability as read-only for raw workflow execution admission.
    #[serde(default)]
    read_only: bool,
    /// Optional least-privilege tool groups exposed to LLM/tool admission.
    #[serde(default)]
    groups: Vec<String>,
    /// Optional tags for inventory and routing.
    #[serde(default)]
    tags: Vec<String>,
    /// If set, an `HttpCapability` is created that POSTs to this URL.
    /// Takes priority over `static_response`.
    #[serde(default)]
    endpoint: Option<String>,
    /// Timeout in milliseconds for HTTP capability calls (default: 30 000).
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Optional static headers forwarded on every HTTP callback — e.g.
    /// `{"Authorization": "Bearer <token>"}` so a bearer-authenticated endpoint
    /// accepts the call. Omit for loopback-trusted endpoints.
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
    /// Fallback: return a fixed static value (used when `endpoint` is absent).
    #[serde(default)]
    static_response: JsonValue,
    /// Explicit backing kind for the capability-id contract:
    /// `provider` (REST via apxm-auth /proxy — the connector default),
    /// `http` (forward to `endpoint`), `static`, or `mcp` (MCP-server bridge).
    /// When unset, falls back to the legacy endpoint/static selection.
    #[serde(default)]
    kind: Option<String>,
    /// MCP server URL for `kind=mcp`.
    #[serde(default)]
    server_url: Option<String>,
    /// MCP tool name for `kind=mcp` (defaults to `name`).
    #[serde(default)]
    mcp_tool: Option<String>,
}

#[derive(Clone)]
pub(crate) struct StaticCapability {
    pub(crate) metadata: CapabilityMetadata,
    pub(crate) static_response: Value,
}

#[async_trait]
impl CapabilityExecutor for StaticCapability {
    async fn execute(&self, _args: HashMap<String, Value>) -> CapabilityResult<Value> {
        Ok(self.static_response.clone())
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

/// One `[[tool]]` entry in a pack-root `tools.toml` — the declarative block
/// spec the install-gated catalog renders and the kernel registers.
#[derive(Debug, Deserialize)]
struct PackToolDecl {
    /// Capability id the inv_tool node lowers to (e.g. `slack.post`).
    capability: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// Backing kind: `provider` (default), `http`, `static`. `mcp` reserved.
    #[serde(default)]
    kind: Option<String>,
    /// REST template for `kind = "provider"`: the request method + url. `{param}`
    /// placeholders in the url are filled from node args at dispatch; the rest of
    /// the args become the JSON body. When present the node needs no `url`.
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    endpoint_pattern: Option<String>,
    #[serde(default)]
    read_only: bool,
    /// Optional typed input schema; defaults to the provider.call arg shape.
    #[serde(default)]
    schema: JsonValue,
    /// MCP server URL for kind=mcp.
    #[serde(default)]
    server_url: Option<String>,
    /// MCP tool name for kind=mcp (defaults to the capability id).
    #[serde(default)]
    mcp_tool: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PackToolsFile {
    #[serde(default)]
    tool: Vec<PackToolDecl>,
}

fn default_provider_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "properties": {
            "credential": { "type": "string" },
            "method": { "type": "string" },
            "url": { "type": "string" },
            "headers": { "type": "object" },
            "body": {}
        },
        "required": ["credential", "url"]
    })
}

/// Auto-register the action blocks declared by every installed pack's pack-root
/// `tools.toml`, so installing a connector pack makes its blocks real
/// capabilities (listed in /v1/capabilities) with NO per-provider Rust. Each
/// `[[tool]]` is registered by its capability id behind the declared backing
/// kind (default `provider` → provider.call, REST via apxm-auth /proxy). This is
/// the keystone that makes capabilities declarative the way skills already are.
/// Build the capability executor for one declared tool (None if its kind is not
/// registerable yet, e.g. `mcp`).
fn capability_from_tool(t: &PackToolDecl) -> Option<Arc<dyn CapabilityExecutor>> {
    let kind = t.kind.as_deref().unwrap_or("provider");
    let schema = if t.schema.is_null() { default_provider_schema() } else { t.schema.clone() };
    let mut metadata = CapabilityMetadata::new(
        t.capability.clone(),
        t.description.clone().or_else(|| t.name.clone()).unwrap_or_else(|| t.capability.clone()),
        schema.clone(),
    );
    if t.read_only {
        metadata = metadata.with_read_only();
    }
    match kind {
        // A REST template (method + url) makes the block self-contained: the node
        // carries only loose args, the cap fills the url + body. Without a url it
        // falls back to the bare named cap (the node must supply `url`).
        "provider" => Some(Arc::new(match t.url.clone() {
            Some(url) => apxm_runtime::capability::builtins::ProviderCallCapability::named_rest(
                t.capability.clone(),
                metadata.description.clone(),
                schema,
                t.method.clone().unwrap_or_else(|| "POST".to_string()),
                url,
            ),
            None => apxm_runtime::capability::builtins::ProviderCallCapability::named(
                t.capability.clone(),
                metadata.description.clone(),
                schema,
            ),
        })),
        "http" => t.endpoint_pattern.clone().map(|endpoint| {
            Arc::new(HttpCapability { metadata, endpoint, timeout_ms: 30_000, headers: HashMap::new(), client: reqwest::Client::new() })
                as Arc<dyn CapabilityExecutor>
        }),
        "mcp" => t.server_url.clone().map(|server_url| {
            let tool = t.mcp_tool.clone().unwrap_or_else(|| t.capability.clone());
            Arc::new(apxm_runtime::capability::builtins::McpBridgeCapability::named(
                t.capability.clone(),
                metadata.description.clone(),
                server_url,
                tool,
            )) as Arc<dyn CapabilityExecutor>
        }),
        other => {
            info!(capability = %t.capability, kind = %other, "skipping tools.toml entry (kind not registerable yet)");
            None
        }
    }
}

/// Read a pack dir's pack-root `tools.toml` and build its capability executors.
fn pack_tools_in_dir(pack_dir: &std::path::Path) -> Vec<Arc<dyn CapabilityExecutor>> {
    let Ok(raw) = std::fs::read_to_string(pack_dir.join("tools.toml")) else { return Vec::new() };
    let file: PackToolsFile = match toml::from_str(&raw) {
        Ok(f) => f,
        Err(e) => {
            info!(dir = %pack_dir.display(), error = %e, "skipping malformed tools.toml");
            return Vec::new();
        }
    };
    file.tool.iter().filter_map(capability_from_tool).collect()
}

/// Auto-register the action blocks declared by every installed pack's pack-root
/// `tools.toml`, so installing a connector pack makes its blocks real
/// capabilities (listed in /v1/capabilities) with NO per-provider Rust. Each
/// `[[tool]]` registers by its capability id behind the declared backing kind
/// (default `provider` → provider.call, REST via apxm-auth /proxy). This is the
/// keystone that makes capabilities declarative the way skills already are.
pub(crate) fn register_pack_tools(runtime: &apxm_runtime::Runtime, roots: &[std::path::PathBuf]) {
    let sys = runtime.capability_system();
    let mut registered = 0u32;
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let pack_dir = entry.path();
            if !pack_dir.is_dir() {
                continue;
            }
            for cap in pack_tools_in_dir(&pack_dir) {
                // register() errors if already present (builtin / re-scan) — fine.
                if sys.register(cap).is_ok() {
                    registered += 1;
                }
            }
        }
    }
    if registered > 0 {
        info!(count = registered, "registered pack tool capabilities from tools.toml");
    }
}

#[cfg(test)]
mod pack_tools_tests {
    use super::*;

    #[test]
    fn derives_provider_block_from_tools_toml() {
        let dir = std::env::temp_dir().join(format!("apxm-pack-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("tools.toml"),
            "[[tool]]\nname = \"Post message\"\ncapability = \"slack.post\"\nkind = \"provider\"\nread_only = false\n",
        )
        .unwrap();

        let caps = pack_tools_in_dir(&dir);
        assert_eq!(caps.len(), 1, "one tool registered");
        let m = caps[0].metadata();
        assert_eq!(m.name, "slack.post");
        assert!(!m.read_only, "slack.post is write-class");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_kind_is_skipped_for_now() {
        let t = PackToolDecl {
            capability: "x.tool".into(),
            name: None,
            description: None,
            kind: Some("mcp".into()),
            method: None,
            url: None,
            endpoint_pattern: None,
            read_only: false,
            schema: JsonValue::Null,
            server_url: None,
            mcp_tool: None,
        };
        assert!(capability_from_tool(&t).is_none(), "mcp backing not registerable yet");
    }

    #[test]
    fn provider_tool_with_url_builds_rest_backed_capability() {
        let t = PackToolDecl {
            capability: "instagram.create_media".into(),
            name: Some("Create media".into()),
            description: None,
            kind: Some("provider".into()),
            method: Some("POST".into()),
            url: Some("https://graph.instagram.com/v25.0/{ig_user_id}/media".into()),
            endpoint_pattern: None,
            read_only: false,
            schema: JsonValue::Null,
            server_url: None,
            mcp_tool: None,
        };
        let cap = capability_from_tool(&t).expect("provider tool registers");
        assert_eq!(cap.metadata().name, "instagram.create_media");
    }
}

pub(crate) async fn list_capabilities(
    State(state): State<AppState>,
) -> Result<Json<Vec<CapabilityEntry>>, ApiError> {
    let caps: Vec<CapabilityEntry> = state
        .runtime
        .capability_system()
        .list_capabilities()
        .iter()
        .map(|m| CapabilityEntry {
            name: m.name.clone(),
            description: m.description.clone(),
            parameters_schema: m.parameters_schema.clone(),
            read_only: m.read_only,
            requires_auth: m.requires_auth,
        })
        .collect();
    Ok(Json(caps))
}

pub(crate) async fn register_capability(
    State(state): State<AppState>,
    Json(req): Json<RegisterCapabilityRequest>,
) -> Result<Json<OkAckName>, ApiError> {
    let mut metadata = CapabilityMetadata::new(
        req.name.clone(),
        req.description.clone(),
        req.parameters_schema,
    );
    if req.read_only {
        metadata = metadata.with_read_only();
    }
    if !req.groups.is_empty() {
        metadata = metadata.with_groups(req.groups.clone());
    }
    if !req.tags.is_empty() {
        metadata = metadata.with_tags(req.tags.clone());
    }

    // Resolve the backing kind. Explicit `kind` wins; otherwise fall back to the
    // legacy selection (endpoint => http, else static). This is the seam the
    // pack loader uses (kind=provider for connector blocks) and where the future
    // `mcp` bridge plugs in — the capability-id contract is unchanged either way.
    let kind = req
        .kind
        .clone()
        .unwrap_or_else(|| if req.endpoint.is_some() { "http".into() } else { "static".into() });

    let capability: Arc<dyn CapabilityExecutor> = match kind.as_str() {
        "provider" => {
            // Connector default: backed by provider.call (REST via apxm-auth
            // /proxy). The block carries url/method/body/credential in its args.
            info!(name = %req.name, "registering provider capability (apxm-auth proxy)");
            Arc::new(apxm_runtime::capability::builtins::ProviderCallCapability::named(
                req.name.clone(),
                req.description.clone(),
                metadata.parameters_schema.clone(),
            ))
        }
        "mcp" => {
            // MCP-client bridge: a per-tool block on an external MCP server.
            let server_url = req
                .server_url
                .clone()
                .ok_or_else(|| ApiError::bad_request("kind=mcp requires server_url".to_string()))?;
            let tool = req.mcp_tool.clone().unwrap_or_else(|| req.name.clone());
            info!(name = %req.name, server_url = %server_url, tool = %tool, "registering MCP-bridge capability");
            Arc::new(apxm_runtime::capability::builtins::McpBridgeCapability::named(
                req.name.clone(),
                req.description.clone(),
                server_url,
                tool,
            ))
        }
        "http" => {
            let endpoint = req
                .endpoint
                .clone()
                .ok_or_else(|| ApiError::bad_request("kind=http requires an endpoint".to_string()))?;
            info!(name = %req.name, endpoint = %endpoint, "registering HTTP capability");
            Arc::new(HttpCapability {
                metadata,
                endpoint,
                timeout_ms: req.timeout_ms.unwrap_or(30_000),
                headers: req.headers.clone().unwrap_or_default(),
                client: reqwest::Client::new(),
            })
        }
        _ => {
            // Static capability — always returns the same configured value.
            let response_value = Value::try_from(req.static_response)
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            info!(name = %req.name, "registering static capability");
            Arc::new(StaticCapability {
                metadata,
                static_response: response_value,
            })
        }
    };

    state
        .runtime
        .capability_system()
        .register_or_replace(capability)
        .map_err(ApiError::runtime)?;
    Ok(Json(OkAckName::new(req.name)))
}
