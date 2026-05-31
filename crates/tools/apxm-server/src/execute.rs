use std::collections::HashMap;
use std::sync::Arc;

use apxm_artifact::Artifact;
use apxm_compiler::AirModule;
use apxm_compiler::{Context as CompilerContext, Pipeline as CompilerPipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::events::payload::ErrorPayload;
use apxm_core::events::{ApxmEvent, EventSource};
use apxm_core::paths::ApxmPaths;
use apxm_core::types::AISOperationType;
use apxm_core::types::execution::Node;
use apxm_core::types::values::Value as RuntimeValue;
use apxm_runtime::EmitterAdapter;
use apxm_runtime::capability::CapabilitySandboxPreflight;
use axum::Json;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::sync::mpsc;

use crate::error::ApiError;
use crate::state::{AppState, ExecuteCompletePayload, TokioChannelEmitter};
use crate::types::responses::{ExecutionStats, LlmUsageSummary};

const ERROR_RAW_PYTHON_TOOL_SECTIONS: &str = "raw execute does not support python tool sections";
const ERROR_RAW_PYTHON_TOOL_HANDLERS: &str =
    "raw execute does not support python-backed tool handlers";
const ERROR_INV_TOOL_MISSING_CAPABILITY: &str = "INV_TOOL missing capability attribute";
const ERROR_INV_TOOL_PARAMS_NOT_OBJECT: &str = "INV_TOOL params_json must be a JSON object";
const ERROR_ASK_REQUIRES_READ_ONLY_TOOLS: &str = "ASK tool exposure requires read-only tools";

#[derive(Debug, Deserialize)]
pub(crate) struct ExecuteRequest {
    pub(crate) air: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) session_root: Option<String>,
    /// Capabilities the caller has explicitly granted this execution. A
    /// non-read-only, non-sandboxed (Direct) tool node is admitted only if its
    /// capability appears here — the itemized consent that lets a workflow
    /// perform writes. Read-only and sandboxed capabilities never need listing.
    #[serde(default)]
    pub(crate) admit_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExecuteResponse {
    pub(crate) results: HashMap<String, JsonValue>,
    pub(crate) content: Option<String>,
    pub(crate) session_dir: Option<String>,
    pub(crate) stats: ExecutionStats,
    pub(crate) llm_usage: LlmUsageSummary,
}

pub(crate) async fn execute(
    State(state): State<AppState>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    let PreparedRequest {
        air,
        args,
        session_id,
        session_dir,
        admit,
    } = prepare_request(req)?;
    let known_caps = registered_capability_names(&state);
    let mut artifact = air_to_artifact_with_caps(&air, &known_caps)?;
    validate_raw_execute_admission(&artifact, &state, &admit)?;
    inject_resolved_credentials(&mut artifact).await?;
    let _permit = state.inference_limiter.acquire().await?;
    let execution = state
        .runtime
        .execute_artifact_with_session_and_emitter(
            artifact,
            args,
            session_id,
            None,
            session_dir.clone(),
        )
        .await
        .map_err(ApiError::runtime)?;
    Ok(Json(to_execute_response(execution, session_dir)))
}

pub(crate) async fn execute_stream(
    State(state): State<AppState>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let PreparedRequest {
        air,
        args,
        session_id,
        session_dir,
        admit,
    } = prepare_request(req)?;
    let known_caps = registered_capability_names(&state);
    let mut artifact = air_to_artifact_with_caps(&air, &known_caps)?;
    validate_raw_execute_admission(&artifact, &state, &admit)?;
    inject_resolved_credentials(&mut artifact).await?;
    let permit = state.inference_limiter.acquire().await?;
    let stream_config = state.server_config.execution_stream;
    let (tx, mut rx) = mpsc::channel::<ApxmEvent>(stream_config.channel_capacity.max(1));
    let runtime = Arc::clone(&state.runtime);
    let trace_id = session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    tokio::spawn(async move {
        let _permit = permit;
        let emitter = Arc::new(EmitterAdapter::new(
            Arc::new(TokioChannelEmitter(tx.clone())),
            EventSource::Runtime,
            &trace_id,
        ));
        match runtime
            .execute_artifact_with_session_and_emitter(
                artifact,
                args,
                session_id,
                Some(emitter),
                session_dir.clone(),
            )
            .await
        {
            Ok(result) => {
                let _ = tx
                    .send(ApxmEvent::root(
                        ExecuteCompletePayload {
                            result: to_execute_response(result, session_dir),
                        },
                        EventSource::Server,
                        &trace_id,
                    ))
                    .await;
            }
            Err(err) => {
                let _ = tx
                    .send(ApxmEvent::root(
                        ErrorPayload {
                            message: err.to_string(),
                            status: None,
                            recoverable: false,
                        },
                        EventSource::Server,
                        &trace_id,
                    ))
                    .await;
            }
        }
    });

    let stream = async_stream::stream! {
        while let Some(item) = rx.recv().await {
            let data = serde_json::to_string(&item).unwrap_or_else(|_| "{}".to_string());
            yield Ok(Event::default().data(data));
        }
    };
    Ok(
        Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(
            stream_config.keep_alive_secs.max(1),
        ))),
    )
}

/// The validated, destructured parts of an execute request.
#[derive(Debug)]
pub(crate) struct PreparedRequest {
    pub(crate) air: String,
    pub(crate) args: Vec<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) session_dir: Option<String>,
    /// Capabilities the caller granted this execution (consent admit-list).
    pub(crate) admit: std::collections::HashSet<String>,
}

pub(crate) fn prepare_request(mut req: ExecuteRequest) -> Result<PreparedRequest, ApiError> {
    if req.air.trim().is_empty() {
        return Err(ApiError::bad_request("air must not be empty"));
    }
    let (session_id, session_dir) =
        resolve_session_request(req.session_id.take(), req.session_root.take())?;
    Ok(PreparedRequest {
        air: req.air,
        args: req.args,
        session_id,
        session_dir,
        admit: req.admit_capabilities.into_iter().collect(),
    })
}

fn resolve_session_request(
    session_id: Option<String>,
    session_root: Option<String>,
) -> Result<(Option<String>, Option<String>), ApiError> {
    if session_root.is_some() {
        return Err(ApiError::bad_request(
            "session_root is server-controlled and cannot be set by clients",
        ));
    }

    if session_id.is_none() {
        return Ok((None, None));
    }

    let resolved_session_id = session_id
        .map(validate_session_id)
        .transpose()?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let base_dir = ApxmPaths::discover()
        .map_err(|e| ApiError::internal_message(format!("failed to discover APXM paths: {e}")))?
        .sessions_dir()
        .map_err(|e| ApiError::internal_message(format!("failed to resolve sessions dir: {e}")))?;

    std::fs::create_dir_all(&base_dir).map_err(|e| {
        ApiError::internal_message(format!(
            "failed to create sessions root '{}': {e}",
            base_dir.display()
        ))
    })?;

    let session_dir = base_dir.join(&resolved_session_id);
    std::fs::create_dir_all(&session_dir).map_err(|e| {
        ApiError::internal_message(format!(
            "failed to create session dir '{}': {e}",
            session_dir.display()
        ))
    })?;

    Ok((
        Some(resolved_session_id),
        Some(session_dir.to_string_lossy().to_string()),
    ))
}

fn validate_session_id(session_id: String) -> Result<String, ApiError> {
    if session_id.is_empty()
        || session_id == "."
        || session_id == ".."
        || !session_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ApiError::bad_request(
            "session_id must contain only ASCII letters, digits, '-', '_', or '.', and must not be '.' or '..'",
        ));
    }
    Ok(session_id)
}

pub(crate) fn air_module_to_artifact(graph: AirModule) -> Result<Artifact, ApiError> {
    let air_text = graph.to_air().map_err(|error| {
        ApiError::bad_request(format!(
            "failed to lower graph '{}' to AIR: {error}",
            graph.name
        ))
    })?;
    air_to_artifact(&air_text)
}

pub(crate) fn air_to_artifact(air_text: &str) -> Result<Artifact, ApiError> {
    air_to_artifact_with_caps(air_text, &std::collections::HashSet::new())
}

/// Compile AIR to an artifact, declaring `known_caps` (the host's runtime-
/// registered provider/pack capabilities) so the tool-binding check accepts
/// them alongside builtins. An empty set keeps the standalone strict behaviour.
pub(crate) fn air_to_artifact_with_caps(
    air_text: &str,
    known_caps: &std::collections::HashSet<String>,
) -> Result<Artifact, ApiError> {
    let context = CompilerContext::new().map_err(|error| {
        ApiError::internal_message(format!(
            "failed to initialize APXM compiler context: {error}"
        ))
    })?;
    let pipeline =
        CompilerPipeline::with_opt_level(&context, apxm_core::types::OptimizationLevel::O1);
    let module = pipeline
        .compile(&air_text)
        .map_err(|error| ApiError::bad_request(format!("failed to compile AIR: {error}")))?;
    let artifact_bytes = module
        .generate_artifact_bytes_with_known_caps(known_caps)
        .map_err(|error| ApiError::internal_message(format!("failed to emit artifact: {error}")))?;
    Artifact::from_bytes(&artifact_bytes)
        .map_err(|error| ApiError::internal_message(format!("failed to decode artifact: {error}")))
}

/// The names of every capability registered in the runtime — handed to the
/// compiler so provider/pack tool nodes pass the tool-binding check.
pub(crate) fn registered_capability_names(state: &AppState) -> std::collections::HashSet<String> {
    state
        .runtime
        .capability_system()
        .list_capabilities()
        .into_iter()
        .map(|c| c.name)
        .collect()
}

fn validate_raw_execute_admission(
    artifact: &Artifact,
    state: &AppState,
    admit: &std::collections::HashSet<String>,
) -> Result<(), ApiError> {
    if artifact
        .sections()
        .iter()
        .any(|section| section.kind == apxm_runtime::python_tools::CAPABILITY_NAME)
    {
        return Err(ApiError::bad_request(ERROR_RAW_PYTHON_TOOL_SECTIONS));
    }

    for dag in artifact.dags() {
        for node in &dag.nodes {
            if node.attributes.contains_key(graph_attrs::PYTHON_HANDLER_ID) {
                return Err(ApiError::bad_request(ERROR_RAW_PYTHON_TOOL_HANDLERS));
            }

            match node.op_type {
                AISOperationType::InvTool => validate_raw_inv_tool_node(node, state, admit)?,
                AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason => {
                    validate_raw_llm_tool_exposure(node, state)?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn validate_raw_inv_tool_node(
    node: &Node,
    state: &AppState,
    admit: &std::collections::HashSet<String>,
) -> Result<(), ApiError> {
    let capability = node
        .attributes
        .get(graph_attrs::CAPABILITY)
        .and_then(|value| value.as_string())
        .ok_or_else(|| ApiError::bad_request(ERROR_INV_TOOL_MISSING_CAPABILITY))?;
    let capability_system = state.runtime.capability_system();

    if !capability_system.has_capability(capability) {
        return Err(ApiError::bad_request(format!(
            "raw execute capability '{capability}' is not registered"
        )));
    }

    if capability_system.is_read_only(capability) {
        return Ok(());
    }

    let args = inv_tool_static_args(node)?;
    match capability_system.sandbox_preflight(capability, &args) {
        Ok(CapabilitySandboxPreflight::Sandboxed { .. }) => Ok(()),
        // A Direct (write) capability is admitted only if the caller explicitly
        // granted it for this execution. The grant is itemized consent: anything
        // not listed stays refused, so the write boundary is preserved.
        Ok(CapabilitySandboxPreflight::Direct) => {
            if admit.contains(capability) {
                Ok(())
            } else {
                Err(ApiError::bad_request(format!(
                    "capability '{capability}' performs writes and was not granted; \
                     add it to admit_capabilities to authorize this execution"
                )))
            }
        }
        Err(error) => Err(ApiError::bad_request(format!(
            "raw execute capability '{capability}' failed sandbox preflight: {error}"
        ))),
    }
}

fn validate_raw_llm_tool_exposure(node: &Node, state: &AppState) -> Result<(), ApiError> {
    let Some(requested_tools) = parse_string_array_attr(node, graph_attrs::TOOLS) else {
        return validate_raw_ask_group_or_all_tools(node, state);
    };
    if requested_tools.is_empty() {
        return validate_raw_ask_group_or_all_tools(node, state);
    }
    validate_read_only_tool_names(&requested_tools, state)
}

fn validate_raw_ask_group_or_all_tools(node: &Node, state: &AppState) -> Result<(), ApiError> {
    let tools_enabled = node
        .attributes
        .get(graph_attrs::TOOLS_ENABLED)
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !tools_enabled {
        return Ok(());
    }

    if let Some(groups) = parse_string_array_attr(node, graph_attrs::TOOL_GROUPS)
        && !groups.is_empty()
    {
        let grouped_tools = state
            .runtime
            .capability_system()
            .list_capabilities_by_groups(&groups);
        for metadata in grouped_tools {
            if !metadata.read_only {
                return Err(ApiError::bad_request(format!(
                    "{ERROR_ASK_REQUIRES_READ_ONLY_TOOLS}; capability '{}' is not read-only",
                    metadata.name
                )));
            }
        }
        return Ok(());
    }

    for metadata in state.runtime.capability_system().list_capabilities() {
        if !metadata.read_only {
            return Err(ApiError::bad_request(format!(
                "ASK tools_enabled=true would expose non-read-only capability '{}'",
                metadata.name
            )));
        }
    }
    Ok(())
}

fn validate_read_only_tool_names(tool_names: &[String], state: &AppState) -> Result<(), ApiError> {
    let capability_system = state.runtime.capability_system();
    for tool_name in tool_names {
        let Some(metadata) = capability_system.get_metadata(tool_name) else {
            return Err(ApiError::bad_request(format!(
                "raw execute capability '{tool_name}' is not registered"
            )));
        };
        if !metadata.read_only {
            return Err(ApiError::bad_request(format!(
                "{ERROR_ASK_REQUIRES_READ_ONLY_TOOLS}; capability '{tool_name}' is not read-only"
            )));
        }
    }
    Ok(())
}

fn parse_string_array_attr(node: &Node, attr_name: &str) -> Option<Vec<String>> {
    node.attributes
        .get(attr_name)
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_string().map(ToString::to_string))
                .collect()
        })
}

/// Dispatch-time credential resolution (apxm-auth M9, F12/F13). Opt-in via
/// `APXM_RESOLVE_CREDENTIALS`: walk `inv_tool` nodes whose `params_json` carries
/// a `credential` connection id, resolve it through apxm-auth, and inject
/// `headers.Authorization = "Bearer <token>"` (dropping the bare id) so the
/// dispatched HTTP capability authenticates. Off by default → a stack without
/// apxm-auth is unaffected. The resolved token is never logged.
async fn inject_resolved_credentials(artifact: &mut Artifact) -> Result<(), ApiError> {
    let enabled = std::env::var("APXM_RESOLVE_CREDENTIALS")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    if !enabled {
        return Ok(());
    }
    let mut resolver: Option<crate::credentials::CredentialResolver> = None;
    for dag in artifact.dags_mut() {
        for node in &mut dag.nodes {
            if node.op_type != AISOperationType::InvTool {
                continue;
            }
            let Some(pj) = node
                .attributes
                .get(graph_attrs::PARAMS_JSON)
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
            else {
                continue;
            };
            let Ok(mut params) = serde_json::from_str::<JsonValue>(&pj) else {
                continue;
            };
            let Some(conn_id) = params
                .get("credential")
                .and_then(JsonValue::as_str)
                .map(|s| s.to_string())
            else {
                continue;
            };
            let r = resolver.get_or_insert_with(crate::credentials::CredentialResolver::from_env);
            let token = r.resolve(&conn_id).await.map_err(|e| {
                ApiError::internal_message(format!("credential resolve failed for `{conn_id}`: {e}"))
            })?;
            if let Some(obj) = params.as_object_mut() {
                obj.remove("credential");
                let headers = obj
                    .entry("headers")
                    .or_insert_with(|| JsonValue::Object(serde_json::Map::new()));
                if let Some(h) = headers.as_object_mut() {
                    h.insert(
                        "Authorization".to_string(),
                        JsonValue::String(format!("Bearer {token}")),
                    );
                }
            }
            node.set_attribute(
                graph_attrs::PARAMS_JSON.to_string(),
                RuntimeValue::String(params.to_string()),
            );
        }
    }
    Ok(())
}

fn inv_tool_static_args(node: &Node) -> Result<HashMap<String, RuntimeValue>, ApiError> {
    let mut args = HashMap::new();
    let Some(params_json) = node
        .attributes
        .get(graph_attrs::PARAMS_JSON)
        .and_then(|value| value.as_string())
    else {
        return Ok(args);
    };

    let parsed: serde_json::Value = serde_json::from_str(params_json)
        .map_err(|error| ApiError::bad_request(format!("invalid INV_TOOL params_json: {error}")))?;
    let Some(object) = parsed.as_object() else {
        return Err(ApiError::bad_request(ERROR_INV_TOOL_PARAMS_NOT_OBJECT));
    };

    for (key, value) in object {
        let value = RuntimeValue::try_from(value.clone()).map_err(|error| {
            ApiError::bad_request(format!("invalid INV_TOOL arg '{key}': {error}"))
        })?;
        args.insert(key.clone(), value);
    }

    Ok(args)
}

pub(crate) fn to_execute_response(
    result: apxm_runtime::RuntimeExecutionResult,
    session_dir: Option<String>,
) -> ExecuteResponse {
    let mut mapped = HashMap::new();
    let mut content = None;
    for (token, value) in &result.results {
        let json = value
            .to_json()
            .unwrap_or_else(|_| JsonValue::String(value.to_string()));
        if content.is_none()
            && let Some(text) = json.as_str()
        {
            content = Some(text.to_string());
        }
        mapped.insert(token.to_string(), json);
    }

    ExecuteResponse {
        results: mapped,
        content,
        session_dir,
        stats: ExecutionStats {
            executed_nodes: result.stats.executed_nodes,
            failed_nodes: result.stats.failed_nodes,
            duration_ms: result.stats.duration_ms,
        },
        llm_usage: LlmUsageSummary {
            input_tokens: result.llm_metrics.total_input_tokens,
            output_tokens: result.llm_metrics.total_output_tokens,
            total_requests: result.llm_metrics.total_requests,
        },
    }
}
