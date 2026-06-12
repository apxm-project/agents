use std::collections::HashMap;
use std::sync::Arc;

use apxm_artifact::Artifact;
use apxm_compiler::AirModule;
use apxm_compiler::{Context as CompilerContext, Pipeline as CompilerPipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::orchestration::admission as orchestration_admission;
use apxm_core::events::payload::{
    ErrorPayload, ExecuteCompletePayload, ExecutionStartedPayload, TurnAbortedPayload,
};
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
use crate::state::{AppState, TokioChannelEmitter};
use crate::types::responses::{ExecutionStats, LlmUsageSummary};

const ERROR_RAW_PYTHON_TOOL_SECTIONS: &str = "raw execute does not support python tool sections";
const ERROR_RAW_PYTHON_TOOL_HANDLERS: &str =
    "raw execute does not support python-backed tool handlers";
const ERROR_INV_TOOL_MISSING_CAPABILITY: &str = "INV_TOOL missing capability attribute";
const ERROR_INV_TOOL_PARAMS_NOT_OBJECT: &str = "INV_TOOL params_json must be a JSON object";
const ERROR_ASK_REQUIRES_READ_ONLY_TOOLS: &str = "ASK tool exposure requires read-only tools";
const ADMIT_SPAWN_AGENT: &str = orchestration_admission::SPAWN_AGENT;
const ADMIT_SPAWN_TEAM: &str = orchestration_admission::SPAWN_TEAM;

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
    /// Visible skill set (lib / lib::skill / skill ids). Empty means only shared skills.
    #[serde(default)]
    pub(crate) imports: Vec<String>,
    /// Per-tool call-count budget: `{capability_name: max_calls}`.
    /// Declared by the caller; enforced by the runtime's trusted `invoke_tool`
    /// seam, shared across the execution tree. Each value is clamped to the
    /// operator ceiling `$APXM_TOOL_CALL_BUDGET_CEILING` when set (a request can
    /// only lower it).
    #[serde(default)]
    pub(crate) tool_call_budgets: HashMap<String, usize>,
    /// Per-tool auth binding: `{capability_name: connection_id}`. The
    /// server resolves each connection id (owner-scoped) to a bearer token and
    /// the runtime injects it at the tool's invoke seam — the secret never enters
    /// the AIR or the prompt; only the connection id travels on the wire.
    #[serde(default)]
    pub(crate) tool_credentials: HashMap<String, String>,
    /// Tenant/owner scope for credential resolution. Passed to the
    /// credential resolver so a tool's token is scoped to this owner.
    #[serde(default)]
    pub(crate) owner: Option<String>,
}

/// A caller-supplied workflow source plus the same execution controls as
/// [`ExecuteRequest`]. The source must be canonical AIR text or a server-local
/// `.air` / Python frontend path that emits AIR.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompileRequest {
    #[serde(default)]
    pub(crate) air: Option<String>,
    #[serde(default)]
    pub(crate) path: Option<String>,
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
    #[serde(default)]
    pub(crate) tool_call_budgets: HashMap<String, usize>,
    #[serde(default)]
    pub(crate) tool_credentials: HashMap<String, String>,
    #[serde(default)]
    pub(crate) owner: Option<String>,
}

impl CompileRequest {
    /// Resolve the caller-supplied workflow source to AIR and fold it into an
    /// [`ExecuteRequest`] so this route shares the execute path verbatim.
    fn into_execute_request(self) -> Result<ExecuteRequest, ApiError> {
        let air = crate::workflow_source::air_from_parts(self.air.as_deref(), self.path.as_deref())
            .map_err(ApiError::bad_request)?;
        Ok(ExecuteRequest {
            air,
            args: self.args,
            session_id: self.session_id,
            session_root: self.session_root,
            admit_capabilities: self.admit_capabilities,
            imports: self.imports,
            tool_call_budgets: self.tool_call_budgets,
            tool_credentials: self.tool_credentials,
            owner: self.owner,
        })
    }
}

pub(crate) async fn compile_workflow(
    state: State<AppState>,
    Json(req): Json<CompileRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    execute(state, Json(req.into_execute_request()?)).await
}

pub(crate) async fn compile_workflow_stream(
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
    /// Consumed per-tool call counts, so a host can maintain a
    /// cross-turn session budget. Empty when no per-tool budget was set.
    #[serde(default)]
    pub(crate) tool_call_counts: HashMap<String, usize>,
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
    metadata.insert(
        apxm_runtime::metadata_keys::VISIBLE_SKILLS.to_string(),
        imports.join(","),
    );
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
/// `run` tool call this, so the compile path, the static write-boundary
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
        tool_call_budgets,
        tool_credentials,
        owner,
    } = prepare_request(req)?;
    let known_caps = registered_capability_names(state);
    let mut artifact = air_to_artifact_with_caps(&air, &known_caps)?;
    validate_raw_execute_admission(&artifact, state, &admit)?;
    inject_resolved_credentials(&mut artifact, owner.as_deref()).await?;
    let resolved_credentials =
        resolve_tool_credentials(&tool_credentials, owner.as_deref()).await?;
    let tool_call_budgets =
        merge_tool_budgets(tool_call_budgets, air_declared_tool_budgets(&artifact));
    let admission_id = acquire_admission(state).await?;
    let mut metadata = admit_grant_metadata(&admit, &imports);
    metadata.insert(
        apxm_runtime::metadata_keys::ADMISSION_ID.to_string(),
        admission_id.clone(),
    );
    if let Some((key, value)) = tool_call_budgets_metadata(&tool_call_budgets) {
        metadata.insert(key, value);
    }
    let execution = state
        .runtime
        .execute_artifact_with_session_emitter_metadata_and_credentials(
            artifact,
            args,
            session_id,
            None,
            session_dir.clone(),
            metadata,
            resolved_credentials,
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
        tool_call_budgets,
        tool_credentials,
        owner,
    } = prepare_request(req)?;
    let known_caps = registered_capability_names(&state);
    let mut artifact = air_to_artifact_with_caps(&air, &known_caps)?;
    validate_raw_execute_admission(&artifact, &state, &admit)?;
    inject_resolved_credentials(&mut artifact, owner.as_deref()).await?;
    let resolved_credentials =
        resolve_tool_credentials(&tool_credentials, owner.as_deref()).await?;
    let tool_call_budgets =
        merge_tool_budgets(tool_call_budgets, air_declared_tool_budgets(&artifact));
    let admission_id = acquire_admission(&state).await?;
    let stream_config = state.server_config.execution_stream;
    let (tx, mut rx) = mpsc::channel::<ApxmEvent>(stream_config.channel_capacity.max(1));
    let runtime = Arc::clone(&state.runtime);
    let mut grant_metadata = admit_grant_metadata(&admit, &imports);
    grant_metadata.insert(
        apxm_runtime::metadata_keys::ADMISSION_ID.to_string(),
        admission_id.clone(),
    );
    if let Some((key, value)) = tool_call_budgets_metadata(&tool_call_budgets) {
        grant_metadata.insert(key, value);
    }
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
        let execution = runtime.execute_artifact_with_session_emitter_metadata_and_credentials(
            artifact,
            args,
            session_id,
            Some(emitter),
            session_dir.clone(),
            grant_metadata,
            resolved_credentials,
        );
        tokio::select! {
            outcome = execution => match outcome {
                Ok(result) => {
                    let _ = tx
                        .send(ApxmEvent::root(
                            ExecuteCompletePayload {
                                result: serde_json::to_value(to_execute_response(result, session_dir))
                                    .unwrap_or(JsonValue::Null),
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
                            duration_ms: 0,
                            reason: "cancelled".to_string(),
                            error_message_safe: Some(
                                "cancelled via /v1/runs/{id}/cancel".to_string(),
                            ),
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
    pub(crate) tool_call_budgets: HashMap<String, usize>,
    pub(crate) tool_credentials: HashMap<String, String>,
    pub(crate) owner: Option<String>,
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
        tool_call_budgets: clamp_tool_call_budgets(req.tool_call_budgets),
        tool_credentials: req.tool_credentials,
        owner: req.owner,
    })
}

/// Resolve a per-tool auth binding — `{capability: connection_id}` —
/// into `{capability: "Bearer <token>"}`, scoped to `owner`. The resolved bearer
/// is handed to the runtime out-of-band (a context field, not the AIR), so the
/// secret never enters the program. Returns `None` when nothing is bound.
async fn resolve_tool_credentials(
    tool_credentials: &HashMap<String, String>,
    owner: Option<&str>,
) -> Result<Option<HashMap<String, String>>, ApiError> {
    if tool_credentials.is_empty() {
        return Ok(None);
    }
    let resolver = crate::credentials::CredentialResolver::from_env();
    let mut resolved = HashMap::new();
    for (capability, connection_id) in tool_credentials {
        let token = resolver.resolve(connection_id, owner).await.map_err(|e| {
            ApiError::internal_message(format!(
                "credential resolve failed for `{connection_id}`: {e}"
            ))
        })?;
        resolved.insert(capability.clone(), format!("Bearer {token}"));
    }
    Ok(Some(resolved))
}

/// Operator ceiling for per-tool call budgets. When
/// `$APXM_TOOL_CALL_BUDGET_CEILING` is set, every requested budget is clamped to
/// at most that value — a caller can only *lower* the operator bound, never raise
/// it. Unset = no ceiling.
fn clamp_tool_call_budgets(mut budgets: HashMap<String, usize>) -> HashMap<String, usize> {
    if let Some(ceiling) = std::env::var("APXM_TOOL_CALL_BUDGET_CEILING")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        for value in budgets.values_mut() {
            *value = (*value).min(ceiling);
        }
    }
    budgets
}

/// Read per-tool call budgets DECLARED in the AIR (any node's `tool_call_budgets`
/// attribute, a JSON object string). The trusted host reads the program's
/// declaration; the program never self-enforces. The most restrictive value wins
/// across nodes.
fn air_declared_tool_budgets(artifact: &Artifact) -> HashMap<String, usize> {
    let mut declared: HashMap<String, usize> = HashMap::new();
    for dag in artifact.dags() {
        for node in &dag.nodes {
            let Some(raw) = node
                .attributes
                .get(graph_attrs::TOOL_CALL_BUDGETS)
                .and_then(|v| v.as_string())
            else {
                continue;
            };
            if let Ok(map) = serde_json::from_str::<HashMap<String, usize>>(raw) {
                for (cap, limit) in map {
                    declared
                        .entry(cap)
                        .and_modify(|e| *e = (*e).min(limit))
                        .or_insert(limit);
                }
            }
        }
    }
    declared
}

/// Merge request budgets with AIR-declared budgets — most restrictive (min) per
/// tool wins — then clamp the union to the operator ceiling.
fn merge_tool_budgets(
    request: HashMap<String, usize>,
    declared: HashMap<String, usize>,
) -> HashMap<String, usize> {
    let mut merged = request;
    for (cap, limit) in declared {
        merged
            .entry(cap)
            .and_modify(|e| *e = (*e).min(limit))
            .or_insert(limit);
    }
    clamp_tool_call_budgets(merged)
}

/// Serialize the per-tool call budget into the execution metadata channel so the
/// runtime can seed `ExecutionContext::tool_call_budgets`. Returns `None` for an
/// empty budget so the metadata key is omitted.
pub(crate) fn tool_call_budgets_metadata(
    budgets: &HashMap<String, usize>,
) -> Option<(String, String)> {
    if budgets.is_empty() {
        return None;
    }
    serde_json::to_string(budgets).ok().map(|json| {
        (
            apxm_runtime::metadata_keys::TOOL_CALL_BUDGETS.to_string(),
            json,
        )
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

pub(crate) fn air_module_to_artifact(module: AirModule) -> Result<Artifact, ApiError> {
    let air_text = module.to_air().map_err(|error| {
        ApiError::bad_request(format!(
            "failed to lower workflow '{}' to AIR: {error}",
            module.name
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
            "WORKFLOW_SPAWN session_root is server-controlled and may not be supplied by a workflow"
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

/// The authoring tool group (Goal 1): write-class capabilities SAFE to *expose*
/// on an ASK node because their *execution* is still gated by the write boundary
/// (admit_capabilities) and confined to a staging area — workflow-scoped
/// admission. Exposing them lets the conversational agent propose authoring and
/// running a workflow; doing so still needs an explicit grant.
const AUTHORING_GROUP: &str = "authoring";

/// An ASK node may expose a capability if it is read-only OR an admit-gated
/// authoring capability.
fn ask_exposable_groups(groups: &[String]) -> bool {
    groups.iter().any(|g| g == AUTHORING_GROUP)
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
            if !metadata.read_only && !ask_exposable_groups(&metadata.groups) {
                return Err(ApiError::bad_request(format!(
                    "{ERROR_ASK_REQUIRES_READ_ONLY_TOOLS}; capability '{}' is not read-only",
                    metadata.name
                )));
            }
        }
        return Ok(());
    }

    for metadata in state.runtime.capability_system().list_capabilities() {
        if !metadata.read_only && !ask_exposable_groups(&metadata.groups) {
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
        if !metadata.read_only && !ask_exposable_groups(&metadata.groups) {
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
pub(crate) async fn inject_resolved_credentials(
    artifact: &mut Artifact,
    owner: Option<&str>,
) -> Result<(), ApiError> {
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
            let token = r.resolve(&conn_id, owner).await.map_err(|e| {
                ApiError::internal_message(format!(
                    "credential resolve failed for `{conn_id}`: {e}"
                ))
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
        tool_call_counts: result.tool_call_counts,
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn merge_tool_budgets_takes_most_restrictive_per_tool() {
        let request = HashMap::from([
            ("web.fetch".to_string(), 5usize),
            ("bash".to_string(), 9usize),
        ]);
        // AIR declares a tighter web.fetch and a new tool not in the request.
        let declared = HashMap::from([
            ("web.fetch".to_string(), 3usize),
            ("slack.post".to_string(), 2usize),
        ]);
        let merged = merge_tool_budgets(request, declared);
        assert_eq!(merged.get("web.fetch").copied(), Some(3)); // min(5, 3)
        assert_eq!(merged.get("bash").copied(), Some(9)); // request only
        assert_eq!(merged.get("slack.post").copied(), Some(2)); // declared only
    }
}
