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
use tokio::sync::{Notify, mpsc};

use crate::error::ApiError;
use crate::state::{
    AppState, ExecuteCompletePayload, ExecutionStartedPayload, TokioChannelEmitter,
    TurnAbortedPayload,
};
use crate::types::responses::{ExecutionStats, LlmUsageSummary};

const ERROR_RAW_PYTHON_TOOL_SECTIONS: &str = "raw execute does not support python tool sections";
const ERROR_RAW_PYTHON_TOOL_HANDLERS: &str =
    "raw execute does not support python-backed tool handlers";
const ERROR_INV_TOOL_MISSING_CAPABILITY: &str = "INV_TOOL missing capability attribute";
const ERROR_INV_TOOL_PARAMS_NOT_OBJECT: &str = "INV_TOOL params_json must be a JSON object";
const ERROR_ASK_REQUIRES_READ_ONLY_TOOLS: &str = "ASK tool exposure requires read-only tools";
const ADMIT_SPAWN_AGENT: &str = "SPAWN_AGENT";
const ADMIT_SPAWN_TEAM: &str = "SPAWN_TEAM";

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
    /// Visible skill set (lib / lib::skill / skill ids). Empty = unrestricted (back-compat).
    #[serde(default)]
    pub(crate) imports: Vec<String>,
}

/// A caller-supplied PlanGraph plus the same execution controls as
/// [`ExecuteRequest`]. The `graph` is an `apxm_ais::plan::PlanGraph` envelope
/// (`{ name, entry, parameters, nodes }`, or wrapped as `{ graph: { ... } }`);
/// the server lowers it to AIR server-side — bypassing the LLM emission path —
/// then routes it through the identical admission gate and runtime as
/// `/v1/execute`.
#[derive(Debug, Deserialize)]
pub(crate) struct CompileRequest {
    pub(crate) graph: JsonValue,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) session_root: Option<String>,
    #[serde(default)]
    pub(crate) admit_capabilities: Vec<String>,
    #[serde(default)]
    pub(crate) imports: Vec<String>,
}

impl CompileRequest {
    /// Lower the caller-supplied PlanGraph to AIR and fold it into an
    /// [`ExecuteRequest`] so the compile route shares the execute path verbatim
    /// (admission gate, credential injection, runtime, session handling).
    fn into_execute_request(self) -> Result<ExecuteRequest, ApiError> {
        let air = crate::mcp_tools::lower_plan_graph_to_air(self.graph)
            .map_err(|error| ApiError::bad_request(format!("plan graph lowering failed: {error}")))?;
        Ok(ExecuteRequest {
            air,
            args: self.args,
            session_id: self.session_id,
            session_root: self.session_root,
            admit_capabilities: self.admit_capabilities,
            imports: self.imports,
        })
    }
}

pub(crate) async fn compile_graph(
    state: State<AppState>,
    Json(req): Json<CompileRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    execute(state, Json(req.into_execute_request()?)).await
}

pub(crate) async fn compile_graph_stream(
    state: State<AppState>,
    Json(req): Json<CompileRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    execute_stream(state, Json(req.into_execute_request()?)).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExecuteResponse {
    pub(crate) results: HashMap<String, JsonValue>,
    pub(crate) content: Option<String>,
    pub(crate) session_dir: Option<String>,
    pub(crate) stats: ExecutionStats,
    pub(crate) llm_usage: LlmUsageSummary,
}

/// Build the top-level execution metadata seeding the effective capability grant
/// so nested CALL_SKILL admission enforces `child ⊆ parent` (no-widen).
pub(crate) fn admit_grant_metadata(
    admit: &std::collections::HashSet<String>,
    imports: &[String],
) -> HashMap<String, String> {
    let policy = if admit.is_empty() {
        apxm_skill::CapabilityPolicy::ReadOnly
    } else {
        apxm_skill::CapabilityPolicy::Broader {
            admits: admit.iter().cloned().collect(),
        }
    };
    let mut metadata = HashMap::new();
    metadata.insert(
        apxm_runtime::metadata_keys::SIDE_EFFECT_POLICY.to_string(),
        policy.name(),
    );
    // Absent = unrestricted CALL_SKILL (back-compat).
    if !imports.is_empty() {
        metadata.insert(
            apxm_runtime::metadata_keys::VISIBLE_SKILLS.to_string(),
            imports.join(","),
        );
    }
    metadata
}

pub(crate) async fn execute(
    State(state): State<AppState>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    Ok(Json(run_air_inner(&state, req).await?))
}

/// Transport-neutral core: compile + admit + execute one AIR request, returning
/// the `ExecuteResponse`. Both the REST `/v1/execute` handler and the MCP
/// `apxm_run` tool call this, so the compile path, the static write-boundary
/// pre-flight, the credential injection, and the no-widen grant seed are shared
/// (DRY) rather than duplicated per transport.
/// Acquire an admission slot and register a park-aware handle, returning the
/// admission id to stamp into execution metadata. While the execution is parked
/// (waiting on an event) it releases the slot; on wake it best-effort reacquires.
/// The caller MUST `admission_registry::unregister(&id)` on completion (this drops
/// the handle and finalizes the slot).
pub(crate) async fn acquire_admission(state: &AppState) -> Result<String, ApiError> {
    let permit = state.inference_limiter.acquire().await?.into_inner();
    let admission_id = format!("adm-{}", uuid::Uuid::new_v4());
    let handle = Arc::new(crate::state::AdmissionHandle::new(
        permit,
        state.inference_limiter.semaphore(),
    ));
    apxm_runtime::scheduler::admission_registry::register(admission_id.clone(), handle);
    Ok(admission_id)
}

pub(crate) async fn run_air_inner(
    state: &AppState,
    req: ExecuteRequest,
) -> Result<ExecuteResponse, ApiError> {
    let PreparedRequest {
        air,
        args,
        session_id,
        session_dir,
        admit,
        imports,
    } = prepare_request(req)?;
    let known_caps = registered_capability_names(state);
    let mut artifact = air_to_artifact_with_caps(&air, &known_caps)?;
    validate_raw_execute_admission(&artifact, state, &admit)?;
    inject_resolved_credentials(&mut artifact).await?;
    let admission_id = acquire_admission(state).await?;
    let mut metadata = admit_grant_metadata(&admit, &imports);
    metadata.insert(
        apxm_runtime::metadata_keys::ADMISSION_ID.to_string(),
        admission_id.clone(),
    );
    let execution = state
        .runtime
        .execute_artifact_with_session_emitter_and_metadata(
            artifact,
            args,
            session_id,
            None,
            session_dir.clone(),
            metadata,
        )
        .await;
    apxm_runtime::scheduler::admission_registry::unregister(&admission_id);
    let execution = execution.map_err(ApiError::runtime)?;
    Ok(to_execute_response(execution, session_dir))
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
        imports,
    } = prepare_request(req)?;
    let known_caps = registered_capability_names(&state);
    let mut artifact = air_to_artifact_with_caps(&air, &known_caps)?;
    validate_raw_execute_admission(&artifact, &state, &admit)?;
    inject_resolved_credentials(&mut artifact).await?;
    let admission_id = acquire_admission(&state).await?;
    let stream_config = state.server_config.execution_stream;
    let (tx, mut rx) = mpsc::channel::<ApxmEvent>(stream_config.channel_capacity.max(1));
    let runtime = Arc::clone(&state.runtime);
    let mut grant_metadata = admit_grant_metadata(&admit, &imports);
    grant_metadata.insert(
        apxm_runtime::metadata_keys::ADMISSION_ID.to_string(),
        admission_id.clone(),
    );
    let trace_id = session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // A unique per-execution handle: `session_id` is reused across turns, so it
    // cannot key cancellation. Registering a `Notify` lets
    // `POST /v1/runs/{execution_id}/cancel` abort this run at its next await
    // boundary; the entry is removed once the run settles either way.
    let execution_id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(Notify::new());
    state
        .cancel_registry
        .insert(execution_id.clone(), Arc::clone(&cancel));
    let cancel_registry = Arc::clone(&state.cancel_registry);
    tokio::spawn(async move {
        // The admission slot is owned by the registered handle (released while
        // parked, reacquired on wake); unregister after the run settles.
        // Frame 0 hands the client the id it needs to address the cancel route.
        let _ = tx
            .send(ApxmEvent::root(
                ExecutionStartedPayload {
                    execution_id: execution_id.clone(),
                },
                EventSource::Server,
                &trace_id,
            ))
            .await;
        let emitter = Arc::new(EmitterAdapter::new(
            Arc::new(TokioChannelEmitter(tx.clone())),
            EventSource::Runtime,
            &trace_id,
        ));
        let execution = runtime.execute_artifact_with_session_emitter_and_metadata(
            artifact,
            args,
            session_id,
            Some(emitter),
            session_dir.clone(),
            grant_metadata,
        );
        tokio::select! {
            outcome = execution => match outcome {
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
            },
            // Cancellation wins: dropping `execution` aborts the in-flight
            // model/tool call at its await point. Emit `turn_aborted` in place
            // of `execute_complete`.
            _ = cancel.notified() => {
                let _ = tx
                    .send(ApxmEvent::root(
                        TurnAbortedPayload {
                            execution_id: execution_id.clone(),
                            reason: "cancelled via /v1/runs/{id}/cancel".to_string(),
                        },
                        EventSource::Server,
                        &trace_id,
                    ))
                    .await;
            }
        }
        apxm_runtime::scheduler::admission_registry::unregister(&admission_id);
        cancel_registry.remove(&execution_id);
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
    pub(crate) admit: std::collections::HashSet<String>,
    pub(crate) imports: Vec<String>,
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
        imports: req.imports,
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

pub(crate) fn validate_raw_execute_admission(
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
                AISOperationType::SpawnAgent => {
                    validate_raw_spawn_op_admission(ADMIT_SPAWN_AGENT, node, admit)?;
                }
                AISOperationType::SpawnTeam => {
                    validate_raw_spawn_op_admission(ADMIT_SPAWN_TEAM, node, admit)?;
                }
                AISOperationType::WorkflowSpawn => {
                    validate_workflow_spawn_node(node).map_err(ApiError::bad_request)?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn validate_raw_spawn_op_admission(
    admit_name: &str,
    node: &Node,
    admit: &std::collections::HashSet<String>,
) -> Result<(), ApiError> {
    if admit.contains(admit_name) || admit.contains(&admit_name.to_ascii_lowercase()) {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "{:?} performs process spawning and was not granted; add '{}' to admit_capabilities to authorize this execution",
        node.op_type, admit_name
    )))
}

fn validate_workflow_spawn_node(node: &Node) -> Result<(), String> {
    if node.attributes.contains_key(graph_attrs::SESSION_ROOT) {
        return Err(
            "WORKFLOW_SPAWN session_root is server-controlled and may not be supplied by a graph"
                .to_string(),
        );
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
pub(crate) async fn inject_resolved_credentials(artifact: &mut Artifact) -> Result<(), ApiError> {
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
            let token = r.resolve(&conn_id, None).await.map_err(|e| {
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
