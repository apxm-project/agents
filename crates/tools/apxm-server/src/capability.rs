use std::collections::HashMap;
use std::sync::Arc;

use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use apxm_runtime::capability::builtins::guard_url_ssrf;
use apxm_runtime::capability::executor::{CapabilityExecutor, CapabilityResult};
use apxm_runtime::capability::metadata::CapabilityMetadata;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::{info, warn};

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

        guard_url_ssrf(&self.metadata.name, &self.endpoint).await?;

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
    /// Endpoint for `kind=http`.
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
    /// Static capability response for `kind=static`.
    #[serde(default)]
    static_response: JsonValue,
    /// Explicit backing kind for the capability-id contract:
    /// `provider` (REST via apxm-auth /proxy — the connector default),
    /// `http` (forward to `endpoint`), `static`, or `mcp` (MCP-server bridge).
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
    /// Capability id the inv_tool node lowers to (e.g. `provider.write`).
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
    /// Whether the block must be bound to a connection before it can run, so the
    /// studio install-gate renders a "connect" prompt on the block. Defaults to
    /// `true` for `kind = "provider"` (a connector action always needs a
    /// connection) and `false` otherwise; set explicitly to override.
    #[serde(default)]
    requires_auth: Option<bool>,
    /// Optional typed input schema; defaults to the provider.call arg shape.
    #[serde(default)]
    schema: JsonValue,
    /// MCP server URL for kind=mcp.
    #[serde(default)]
    server_url: Option<String>,
    /// MCP tool name for kind=mcp (defaults to the capability id).
    #[serde(default)]
    mcp_tool: Option<String>,
    /// Optional declared tool version (typeVersion). Metadata-only for now:
    /// decoded for forward-compat (catalog/frontend), not yet threaded into
    /// dispatch — so it is intentionally unread on this path.
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<u32>,
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

fn normalize_pack_schema(capability: &str, schema: &JsonValue) -> JsonValue {
    match schema {
        JsonValue::Null => default_provider_schema(),
        JsonValue::String(raw) => match serde_json::from_str::<JsonValue>(raw) {
            Ok(parsed) => parsed,
            Err(error) => {
                warn!(
                    capability = %capability,
                    %error,
                    "pack tool schema string is not valid JSON; using default provider schema"
                );
                default_provider_schema()
            }
        },
        other => other.clone(),
    }
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
    let schema = normalize_pack_schema(&t.capability, &t.schema);
    let mut metadata = CapabilityMetadata::new(
        t.capability.clone(),
        t.description
            .clone()
            .or_else(|| t.name.clone())
            .unwrap_or_else(|| t.capability.clone()),
        schema.clone(),
    );
    if t.read_only {
        metadata = metadata.with_read_only();
    }
    match kind {
        // A REST template (method + url) makes the block self-contained: the node
        // carries only loose args, the cap fills the url + body. Without a url it
        // falls back to the bare named cap (the node must supply `url`).
        "provider" => {
            // Provider-backed connector blocks need a bound connection, so the
            // install-gate gates them on a connection. Default the flag to true
            // unless the pack overrides it explicitly.
            let requires_auth = t.requires_auth.unwrap_or(true);
            let cap = match t.url.clone() {
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
            }
            .with_requires_auth(requires_auth)
            .with_read_only(t.read_only);
            Some(Arc::new(cap) as Arc<dyn CapabilityExecutor>)
        }
        "http" => t.endpoint_pattern.clone().map(|endpoint| {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default();
            if t.requires_auth.unwrap_or(false) {
                metadata = metadata.with_auth();
            }
            Arc::new(HttpCapability {
                metadata,
                endpoint,
                timeout_ms: 30_000,
                headers: HashMap::new(),
                client,
            }) as Arc<dyn CapabilityExecutor>
        }),
        "mcp" => t.server_url.clone().map(|server_url| {
            let tool = t.mcp_tool.clone().unwrap_or_else(|| t.capability.clone());
            Arc::new(
                apxm_runtime::capability::builtins::McpBridgeCapability::named(
                    t.capability.clone(),
                    metadata.description.clone(),
                    server_url,
                    tool,
                ),
            ) as Arc<dyn CapabilityExecutor>
        }),
        other => {
            info!(capability = %t.capability, kind = %other, "skipping tools.toml entry (kind not registerable yet)");
            None
        }
    }
}

/// Read a pack dir's pack-root `tools.toml` and build its capability executors.
fn pack_tools_in_dir(pack_dir: &std::path::Path) -> Vec<Arc<dyn CapabilityExecutor>> {
    let Ok(raw) = std::fs::read_to_string(pack_dir.join("tools.toml")) else {
        return Vec::new();
    };
    let file: PackToolsFile = match toml::from_str(&raw) {
        Ok(f) => f,
        Err(e) => {
            info!(dir = %pack_dir.display(), error = %e, "skipping malformed tools.toml");
            return Vec::new();
        }
    };
    file.tool.iter().filter_map(capability_from_tool).collect()
}

/// Rescan installed pack `tools.toml` files and register any new capabilities.
/// Idempotent: already-registered capability ids are skipped. Called on deploy
/// and via `POST /v1/capabilities/rescan` so freshly dropped packs show up
/// without a server restart.
pub(crate) fn rescan_pack_tools(runtime: &apxm_runtime::Runtime, roots: &[std::path::PathBuf]) -> u32 {
    let before = runtime.capability_system().list_capabilities().len();
    register_pack_tools(runtime, roots);
    let after = runtime.capability_system().list_capabilities().len();
    after.saturating_sub(before) as u32
}

/// Auto-register the action blocks declared by every installed pack's pack-root
/// `tools.toml`, so installing a connector pack makes its blocks real
/// capabilities (listed in /v1/capabilities) with NO per-provider Rust. Each
/// `[[tool]]` registers by its capability id behind the declared backing kind
/// (default `provider` → provider.call, REST via apxm-auth /proxy). This is the
/// keystone that makes capabilities declarative the way skills already are.
pub(crate) fn register_pack_tools(runtime: &apxm_runtime::Runtime, roots: &[std::path::PathBuf]) {
    let sys = runtime.capability_system();
    let mut total_registered = 0u32;
    for root in roots {
        let mut root_registered = 0u32;
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) => {
                info!(root = %root.display(), %error, "skipping pack capability root");
                continue;
            }
        };
        let mut root_seen = 0u32;
        for entry in entries.flatten() {
            let pack_dir = entry.path();
            if !pack_dir.is_dir() {
                continue;
            }
            root_seen += 1;
            for cap in pack_tools_in_dir(&pack_dir) {
                // register() errors if already present (builtin / re-scan) — fine.
                let name = cap.metadata().name.clone();
                match sys.register(cap) {
                    Ok(()) => {
                        total_registered += 1;
                        root_registered += 1;
                        info!(capability = %name, pack_dir = %pack_dir.display(), "registered pack tool capability");
                    }
                    Err(error) => {
                        warn!(capability = %name, pack_dir = %pack_dir.display(), %error, "failed to register pack tool capability");
                    }
                }
            }
        }
        info!(
            root = %root.display(),
            packs = root_seen,
            count = root_registered,
            "scanned pack tool capability root"
        );
    }
    info!(count = total_registered, "completed pack tool capability scan");
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

#[derive(Debug, Serialize)]
pub(crate) struct RescanCapabilitiesResponse {
    pub(crate) registered: u32,
    pub(crate) total: usize,
}

pub(crate) async fn rescan_capabilities(
    State(state): State<AppState>,
) -> Result<Json<RescanCapabilitiesResponse>, ApiError> {
    let roots = crate::startup::pack_capability_roots(state.skill_library.roots());
    let registered = rescan_pack_tools(&state.runtime, &roots);
    let total = state.runtime.capability_system().list_capabilities().len();
    Ok(Json(RescanCapabilitiesResponse { registered, total }))
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

    let kind = req.kind.clone().ok_or_else(|| {
        ApiError::bad_request(
            "capability registration requires explicit kind: provider, http, mcp, or static"
                .to_string(),
        )
    })?;

    let capability: Arc<dyn CapabilityExecutor> = match kind.as_str() {
        "provider" => {
            // Connector default: backed by provider.call (REST via apxm-auth
            // /proxy). The block carries url/method/body/credential in its args.
            info!(name = %req.name, "registering provider capability (apxm-auth proxy)");
            Arc::new(
                apxm_runtime::capability::builtins::ProviderCallCapability::named(
                    req.name.clone(),
                    req.description.clone(),
                    metadata.parameters_schema.clone(),
                ),
            )
        }
        "mcp" => {
            // MCP-client bridge: a per-tool block on an external MCP server.
            let server_url = req
                .server_url
                .clone()
                .ok_or_else(|| ApiError::bad_request("kind=mcp requires server_url".to_string()))?;
            let tool = req.mcp_tool.clone().unwrap_or_else(|| req.name.clone());
            info!(name = %req.name, server_url = %server_url, tool = %tool, "registering MCP-bridge capability");
            Arc::new(
                apxm_runtime::capability::builtins::McpBridgeCapability::named(
                    req.name.clone(),
                    req.description.clone(),
                    server_url,
                    tool,
                ),
            )
        }
        "http" => {
            let endpoint = req.endpoint.clone().ok_or_else(|| {
                ApiError::bad_request("kind=http requires an endpoint".to_string())
            })?;
            guard_url_ssrf(&req.name, &endpoint)
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            info!(name = %req.name, endpoint = %endpoint, "registering HTTP capability");
            Arc::new(HttpCapability {
                metadata,
                endpoint,
                timeout_ms: req.timeout_ms.unwrap_or(30_000),
                headers: req.headers.clone().unwrap_or_default(),
                client: reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|e| {
                        ApiError::internal_message(format!("failed to build HTTP client: {e}"))
                    })?,
            })
        }
        "static" => {
            let response_value = Value::try_from(req.static_response)
                .map_err(|e| ApiError::bad_request(e.to_string()))?;
            info!(name = %req.name, "registering static capability");
            Arc::new(StaticCapability {
                metadata,
                static_response: response_value,
            })
        }
        _ => {
            return Err(ApiError::bad_request(format!(
                "unknown capability kind '{}'; expected provider, http, mcp, or static",
                kind
            )));
        }
    };

    state
        .runtime
        .capability_system()
        .register_or_replace(capability)
        .map_err(ApiError::runtime)?;
    Ok(Json(OkAckName::new(req.name)))
}

/// Request body for `POST /v1/capabilities/{capability_id}/invoke`.
///
/// `connection` is the apxm-auth connection id (`provider/conn`) the studio
/// node is bound to; it is threaded into the provider.call invocation as the
/// `credential` arg exactly like an inv_tool node, so the secret is resolved
/// server-side by apxm-auth and never reaches the studio. `owner` is accepted
/// for parity with the rest of the surface but the provider.call backing scopes
/// the tenant from `APXM_AUTH_OWNER` server-side.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct InvokeCapabilityRequest {
    #[serde(default)]
    args: serde_json::Map<String, JsonValue>,
    #[serde(default)]
    connection: Option<String>,
    #[serde(default)]
    owner: Option<String>,
}

/// Response body for a single capability invocation. `result` is the raw value
/// the capability returned (a JSON array/object/string), which the studio's
/// load-options proxy maps into `{value,label}` options.
#[derive(Debug, Serialize)]
pub(crate) struct InvokeCapabilityResponse {
    result: JsonValue,
    status: &'static str,
}

/// Invoke a SINGLE read-only capability once and return its result, so the
/// studio can populate dynamic "load options" dropdowns (e.g. list a provider's
/// channels/repos for a node-config select).
///
/// This reuses the same invocation seam the runtime drives for an inv_tool node
/// ([`CapabilitySystem::invoke`]) rather than building a graph: load-options is
/// a read, not a workflow. The capability MUST be marked `read_only` — a
/// dropdown population must be side-effect-free — otherwise the call is rejected
/// with `400`. The credential is resolved server-side via apxm-auth exactly like
/// a normal provider.call (the studio never sees the secret).
pub(crate) async fn invoke_capability(
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
    Json(req): Json<InvokeCapabilityRequest>,
) -> Result<Json<InvokeCapabilityResponse>, ApiError> {
    let cap_sys = state.runtime.capability_system();

    if !cap_sys.has_capability(&capability_id) {
        return Err(ApiError::not_found(format!(
            "capability '{capability_id}' is not registered"
        )));
    }

    // Load-options must be side-effect-free: refuse anything that is not
    // explicitly read-only so this endpoint can never be used to drive a write.
    if !cap_sys.is_read_only(&capability_id) {
        return Err(ApiError::bad_request(format!(
            "capability '{capability_id}' is not read_only; only read-only capabilities may be invoked for load-options"
        )));
    }

    // Convert the JSON args to runtime values, injecting the bound connection id
    // as the `credential` arg the provider.call backing forwards to apxm-auth.
    let mut args: HashMap<String, Value> = HashMap::with_capacity(req.args.len() + 1);
    if let Some(connection) = req.connection {
        args.insert("credential".to_string(), Value::String(connection));
    }
    for (key, value) in req.args {
        let value = Value::try_from(value)
            .map_err(|e| ApiError::bad_request(format!("invalid arg '{key}': {e}")))?;
        args.insert(key, value);
    }

    info!(
        capability = %capability_id,
        owner = ?req.owner,
        "invoking read-only capability for load-options"
    );

    let result = cap_sys
        .invoke(&capability_id, args)
        .await
        .map_err(ApiError::runtime)?;
    let result_json = result
        .to_json()
        .unwrap_or_else(|_| JsonValue::String(result.to_string()));

    Ok(Json(InvokeCapabilityResponse {
        result: result_json,
        status: "ok",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_tool_decl_version_defaults_to_none() {
        // An existing-style tools.toml with no `version` key parses unchanged
        // and leaves `version` at its additive default of None.
        let raw = r#"
            [[tool]]
            capability = "provider.write"
            name = "Write thing"
            kind = "provider"
            url = "https://api.example.com/write"
        "#;
        let file: PackToolsFile = toml::from_str(raw).expect("parse tools.toml without version");
        assert_eq!(file.tool.len(), 1);
        assert_eq!(file.tool[0].capability, "provider.write");
        assert_eq!(file.tool[0].version, None);
    }

    #[test]
    fn pack_tool_decl_version_round_trips() {
        let raw = r#"
            [[tool]]
            capability = "provider.write"
            version = 2
        "#;
        let file: PackToolsFile = toml::from_str(raw).expect("parse tools.toml with version");
        assert_eq!(file.tool.len(), 1);
        assert_eq!(file.tool[0].version, Some(2));
    }

    #[test]
    fn provider_tool_defaults_to_requires_auth() {
        // A provider-kind block with no explicit `requires_auth` is gated on a
        // connection so the install-gate renders a connect prompt.
        let raw = r#"
            [[tool]]
            capability = "slack.post_message"
            kind = "provider"
            url = "https://slack.com/api/chat.postMessage"
        "#;
        let file: PackToolsFile = toml::from_str(raw).expect("parse provider tools.toml");
        let cap = capability_from_tool(&file.tool[0]).expect("provider cap builds");
        assert!(
            cap.metadata().requires_auth,
            "provider blocks default to requires_auth=true"
        );
    }

    #[test]
    fn c5_capability_rescan_route_is_pinned() {
        assert_eq!(
            crate::routes::CAPABILITIES_RESCAN,
            "/v1/capabilities/rescan"
        );
    }

    /// Cross-plane contract C4: os-dispatch execute envelope + correlation id.
    #[test]
    fn c4_execute_envelope_accepts_correlation_and_data_arg() {
        use crate::skills::SkillExecuteRequest;
        let envelope = serde_json::json!({
            "event": { "kind": "webhook", "correlation_id": "del-42", "payload": { "x": 1 } },
            "agent_context": { "beliefs": {} }
        });
        let req: SkillExecuteRequest = serde_json::from_value(serde_json::json!({
            "args": [envelope.to_string()],
            "session_id": "agent-99",
            "correlation_id": "del-42"
        }))
        .expect("SkillExecuteRequest deserializes");
        assert_eq!(req.session_id.as_deref(), Some("agent-99"));
        assert_eq!(req.correlation_id.as_deref(), Some("del-42"));
        let parsed =
            serde_json::from_str::<serde_json::Value>(&req.args[0]).expect("args[0] is JSON");
        assert_eq!(parsed["event"]["correlation_id"], "del-42");
    }

    /// A packs directory containing a `tools.toml` registers its `[[tool]]`
    /// entries as runtime capabilities, so `/v1/capabilities` lists them and the
    /// studio install-gate sees the blocks as AVAILABLE.
    #[tokio::test]
    async fn packs_dir_tools_toml_registers_capabilities() {
        use apxm_runtime::{Runtime, RuntimeConfig};

        let root = tempfile::tempdir().expect("packs root");
        let pack_dir = root.path().join("slack-connector");
        std::fs::create_dir_all(&pack_dir).expect("create pack dir");
        std::fs::write(
            pack_dir.join("tools.toml"),
            r#"
                [[tool]]
                capability = "slack.post_message"
                name = "Post Slack message"
                description = "Send a message to a Slack channel"
                kind = "provider"
                method = "POST"
                url = "https://slack.com/api/chat.postMessage"
                schema = """{"type":"object","required":["channel","text"],"properties":{"channel":{"type":"string"},"text":{"type":"string"}}}"""

                [[tool]]
                capability = "weather.lookup"
                kind = "provider"
                read_only = true
                requires_auth = false
            "#,
        )
        .expect("write tools.toml");
        std::fs::write(
            pack_dir.join("connector.toml"),
            r#"
                schema_version = 1
                profile_version = "connector-plugin/v1"
                app_id = "slack"
                provider = "slack"

                [connection]
                noun = "Workspace"

                [[operation]]
                id = "post_message"
                kind = "action"
                runtime_node = "tool"
                source = "tools.toml:slack.post_message"
                title = "Send message"
                summary = "Slack message to {{args.channel|label}}: {{args.text|label}}"
            "#,
        )
        .expect("write connector UI metadata");

        let runtime = Runtime::new(RuntimeConfig::in_memory())
            .await
            .expect("build in-memory runtime");
        register_pack_tools(&runtime, &[root.path().to_path_buf()]);

        let sys = runtime.capability_system();
        assert!(
            sys.has_capability("slack.post_message"),
            "provider block registered as a capability"
        );
        assert!(
            sys.has_capability("weather.lookup"),
            "second provider block registered as a capability"
        );

        let listed = sys.list_capabilities();
        let slack = listed
            .iter()
            .find(|m| m.name == "slack.post_message")
            .expect("slack capability listed");
        assert!(slack.requires_auth, "auth-gated provider block");
        assert!(
            slack.metadata.is_empty(),
            "apxm-server must not ingest Studio connector UI metadata"
        );
        let weather = listed
            .iter()
            .find(|m| m.name == "weather.lookup")
            .expect("weather capability listed");
        assert!(
            !weather.requires_auth,
            "explicit requires_auth=false overrides the provider default"
        );
        assert!(weather.read_only, "read_only flag honored");
    }

    use crate::state::AppState;
    use apxm_runtime::{Runtime, RuntimeConfig};
    use dashmap::DashMap;

    /// Minimal in-memory `AppState` for invoking the capability handler directly
    /// (no TCP port). Mirrors the field set assembled at startup with volatile
    /// in-memory stores so parallel tests do not contend on SQLite files.
    async fn test_state() -> AppState {
        let runtime = Arc::new(
            Runtime::new(RuntimeConfig::in_memory())
                .await
                .expect("in-memory runtime"),
        );
        AppState {
            runtime,
            agent_registry: Arc::new(DashMap::new()),
            task_manager: crate::tasks::TaskQueueManager::new(),
            checkpoint_store: crate::checkpoints::CheckpointStore::new(),
            start_time: std::time::SystemTime::now(),
            a2a_tasks: Arc::new(DashMap::new()),
            skill_library: crate::skills::SkillLibrary::new(Vec::new()),
            execution_store: crate::executions::ExecutionStore::new(),
            run_event_bus: crate::runs::RunEventBus::new(),
            webhook_dispatcher: None,
            rollout_paths: Arc::new(apxm_rollout::RolloutPaths::new({
                // Forget the tempdir so its lifetime spans the AppState; the test
                // process exits and the OS reclaims /tmp on its own.
                let dir = tempfile::tempdir().expect("rollout home");
                let path = dir.path().to_path_buf();
                std::mem::forget(dir);
                path
            })),
            rollout_index: Arc::new(tokio::sync::Mutex::new(
                apxm_rollout::IndexDb::open_in_memory().expect("rollout index"),
            )),
            rollout_registry: crate::rollout::RolloutRegistry::new(),
            inference_limiter: crate::state::InferenceLimiter::unlimited_for_tests(),
            server_config: apxm_driver::ServerConfig::default(),
            cancel_registry: Arc::new(DashMap::new()),
            goal_runs: crate::goal_runs::GoalRunRegistry::new(),
            session_registry: crate::conversations::SessionRegistry::new(),
        }
    }

    /// A read-only capability returns a static option list — exactly what the
    /// studio load-options dropdown consumes — and the handler returns it under
    /// the `{ result, status }` envelope.
    #[tokio::test]
    async fn invoke_read_only_capability_returns_result() {
        let state = test_state().await;
        let options = serde_json::json!([
            { "value": "C123", "label": "#general" },
            { "value": "C456", "label": "#random" }
        ]);
        let metadata = CapabilityMetadata::new(
            "slack.list_channels",
            "List Slack channels for a load-options dropdown",
            serde_json::json!({ "type": "object", "properties": {} }),
        )
        .with_read_only();
        state
            .runtime
            .capability_system()
            .register(Arc::new(StaticCapability {
                metadata,
                static_response: Value::try_from(options.clone()).expect("static value"),
            }))
            .expect("register read-only capability");

        let resp = invoke_capability(
            State(state),
            Path("slack.list_channels".to_string()),
            Json(InvokeCapabilityRequest {
                connection: Some("slack/acme".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect("invoke succeeds");

        assert_eq!(resp.0.status, "ok");
        assert_eq!(resp.0.result, options, "raw capability result is returned");
    }

    /// A capability that is NOT read-only is rejected: load-options must be
    /// side-effect-free, so a write capability can never be driven through it.
    #[tokio::test]
    async fn invoke_non_read_only_capability_is_refused() {
        let state = test_state().await;
        let metadata = CapabilityMetadata::new(
            "slack.post_message",
            "Send a Slack message (side-effecting)",
            serde_json::json!({ "type": "object", "properties": {} }),
        );
        state
            .runtime
            .capability_system()
            .register(Arc::new(StaticCapability {
                metadata,
                static_response: Value::String("sent".to_string()),
            }))
            .expect("register write capability");

        let err = invoke_capability(
            State(state),
            Path("slack.post_message".to_string()),
            Json(InvokeCapabilityRequest::default()),
        )
        .await
        .expect_err("non-read-only capability is refused");

        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        assert!(
            err.message.contains("read_only"),
            "rejection explains the read-only requirement: {}",
            err.message
        );
    }

    /// An unregistered capability id yields a 404 rather than a generic error.
    #[tokio::test]
    async fn invoke_unknown_capability_is_not_found() {
        let state = test_state().await;
        let err = invoke_capability(
            State(state),
            Path("nope.missing".to_string()),
            Json(InvokeCapabilityRequest::default()),
        )
        .await
        .expect_err("unknown capability is rejected");
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }
}
