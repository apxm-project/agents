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
use apxm_runtime::capability::CapabilitySandboxPreflight;
use apxm_runtime::{EmitterAdapter, ExecutionEventEmitter};
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, Sse};
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::sync::{Notify, mpsc};

use crate::error::ApiError;
use crate::executions::{ExecutionRecord, ExecutionRecordingEmitter};
use crate::runs::{RunBusFanOutEmitter, record_run_lifecycle_event};
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
    /// The verbatim user prompt to record in the durable transcript. The
    /// runtime emits the model-facing prompt REDACTED (a blake3 hash + char
    /// summary), so on the streaming path we additionally write one typed
    /// `UserMessage` rollout line carrying this text — that is what
    /// `GET /v1/sessions/{id}/history` returns as the user turn. Absent or
    /// empty ⇒ no extra line is written and history falls back to the redacted
    /// summary (prior behavior). Only honored when `session_id` is present and
    /// rollout recording is active.
    #[serde(default)]
    pub(crate) user_text: Option<String>,
    /// Workspace workflow identity. The basename of
    /// `workspace/workflows/<id>/`; present when the caller supplies it so
    /// run history can be grouped by workflow and queried via
    /// `GET /v1/workflows/{id}/runs`.
    #[serde(default)]
    pub(crate) workflow_id: Option<String>,
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
    #[serde(default)]
    pub(crate) user_text: Option<String>,
    #[serde(default)]
    pub(crate) workflow_id: Option<String>,
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
            user_text: self.user_text,
            workflow_id: self.workflow_id,
        })
    }
}

pub(crate) async fn compile_workflow(
    state: State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CompileRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    execute(state, headers, Json(req.into_execute_request()?)).await
}

pub(crate) async fn compile_workflow_stream(
    state: State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CompileRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    execute_stream(state, headers, Json(req.into_execute_request()?)).await
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompileArtifactRequest {
    /// Canonical AIR (`.air` MLIR text) to compile.
    pub(crate) air: String,
}

/// `POST /v1/compile-artifact` — compile AIR to an installable `.apxmobj`
/// artifact and return its raw bytes (`application/octet-stream`). Unlike
/// `/v1/compile` (which compiles+executes), this only emits the artifact, and it
/// compiles WITH the runtime's registered capabilities so provider/pack tool
/// nodes pass the tool-binding check (E712). This is the registry-aware compile
/// the studio's compile-on-deploy posts to so a deployed skill can run.
pub(crate) async fn compile_artifact(
    State(state): State<AppState>,
    Json(req): Json<CompileArtifactRequest>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let caps = registered_capability_names(&state);
    let bytes = air_to_artifact_bytes_with_caps(&req.air, &caps)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        bytes,
    )
        .into_response())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExecuteResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) run_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) trace_id: Option<String>,
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
    headers: HeaderMap,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    Ok(Json(
        run_air_inner(&state, req, extract_trace_id(&headers)).await?,
    ))
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
    trace_id: Option<String>,
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
        // Only the streaming path records a durable transcript; the raw
        // execute path has no rollout, so the verbatim prompt is unused here.
        user_text: _,
        python_tools_sidecar,
        workflow_id,
    } = prepare_request(req)?;
    let known_caps = registered_capability_names(state);
    let mut artifact = air_to_artifact_with_caps(&air, &known_caps)?;
    attach_trusted_python_section(&mut artifact, python_tools_sidecar);
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
    let execution_id = uuid::Uuid::new_v4().to_string();
    let trace_id = trace_id.unwrap_or_else(|| execution_id.clone());
    let raw_record = start_raw_execution_record(
        state,
        &execution_id,
        workflow_id.clone(),
        trace_id.clone(),
        session_id.as_deref(),
        session_dir.as_deref(),
    );
    if let Some(record) = raw_record.as_ref() {
        attach_runtime_run_metadata(&mut metadata, record, &trace_id)?;
    }
    if raw_record.is_some() {
        state.run_event_bus.record(
            &execution_id,
            ApxmEvent::root(
                ExecutionStartedPayload {
                    execution_id: execution_id.clone(),
                },
                EventSource::Server,
                &trace_id,
            ),
        );
    }
    let emitter: Option<Arc<dyn ExecutionEventEmitter>> = raw_record.as_ref().map(|_| {
        Arc::new(EmitterAdapter::new(
            Arc::new(apxm_core::events::FanOutEmitter::new(vec![
                Arc::new(ExecutionRecordingEmitter::new(
                    state.execution_store.clone(),
                    execution_id.clone(),
                )),
                Arc::new(RunBusFanOutEmitter::new(
                    state.run_event_bus.clone(),
                    execution_id.clone(),
                    Vec::new(),
                )),
            ])),
            EventSource::Runtime,
            &trace_id,
        )) as Arc<dyn ExecutionEventEmitter>
    });
    let execution = state
        .runtime
        .execute_artifact_with_session_emitter_metadata_and_credentials(
            artifact,
            args,
            session_id,
            emitter,
            session_dir.clone(),
            metadata,
            resolved_credentials,
        )
        .await;
    apxm_runtime::scheduler::admission_registry::unregister(&admission_id);
    match execution {
        Ok(execution) => {
            let mut response = to_execute_response(execution, session_dir);
            if let Some(record) = raw_record {
                attach_run_metadata(&mut response, &record, &trace_id);
                state
                    .execution_store
                    .complete_success(&execution_id, response.clone());
                state.run_event_bus.record(
                    &execution_id,
                    ApxmEvent::root(
                        ExecuteCompletePayload {
                            result: serde_json::to_value(&response).unwrap_or(JsonValue::Null),
                        },
                        EventSource::Server,
                        &trace_id,
                    ),
                );
            }
            Ok(response)
        }
        Err(error) => {
            if raw_record.is_some() {
                let message = error.to_string();
                state
                    .execution_store
                    .complete_failure(&execution_id, message.clone());
                state.run_event_bus.record(
                    &execution_id,
                    ApxmEvent::root(
                        ErrorPayload {
                            message,
                            status: None,
                            recoverable: false,
                        },
                        EventSource::Server,
                        &trace_id,
                    ),
                );
            }
            Err(ApiError::runtime(error))
        }
    }
}

pub(crate) async fn execute_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
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
        user_text,
        python_tools_sidecar,
        workflow_id,
    } = prepare_request(req)?;
    let known_caps = registered_capability_names(&state);
    let mut artifact = air_to_artifact_with_caps(&air, &known_caps)?;
    attach_trusted_python_section(&mut artifact, python_tools_sidecar);
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
    // Prefer the caller-supplied X-Trace-Id header for end-to-end correlation.
    // Fall back to session_id (for multi-turn sessions) then a fresh UUID.
    let trace_id = extract_trace_id(&headers)
        .or_else(|| session_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // A unique per-execution handle: `session_id` is reused across turns, so it
    // cannot key cancellation. Registering a `Notify` lets
    // `POST /v1/runs/{execution_id}/cancel` abort this run at its next await
    // boundary; the entry is deleted once the run settles either way.
    let execution_id = uuid::Uuid::new_v4().to_string();
    let raw_record = start_raw_execution_record(
        &state,
        &execution_id,
        workflow_id.clone(),
        trace_id.clone(),
        session_id.as_deref(),
        session_dir.as_deref(),
    );
    if let Some(record) = raw_record.as_ref() {
        attach_runtime_run_metadata(&mut grant_metadata, record, &trace_id)?;
    }
    let tx_task = tx.clone();
    // Seed the per-session runtime ledger (turn caps / tool budgets / grants)
    // keyed by session_id and register the session→execution mapping, so the
    // runtime owns per-session limits and the turn-input endpoint can find this
    // session's long-lived execution. Idempotent across re-armed turns of the
    // same session.
    if let Some(sid) = &session_id {
        apxm_runtime::executor::session_ledger::seed(
            sid,
            apxm_runtime::executor::session_ledger::SessionLedger::new(
                None,
                tool_call_budgets.clone(),
                admit.iter().cloned().collect(),
            ),
        );
        state
            .session_registry
            .register(sid.clone(), execution_id.clone());
    }
    let cancel = Arc::new(Notify::new());
    state
        .cancel_registry
        .insert(execution_id.clone(), Arc::clone(&cancel));
    let cancel_registry = Arc::clone(&state.cancel_registry);

    // Durably record this turn ONLY when a session_id is present: the
    // session_id is the cross-turn key that lets `GET /v1/sessions/{id}/history`
    // reassemble the visible conversation that both `apxm chat` and the studio
    // Chat stream through this endpoint. Absent a session_id there is nothing
    // to retrieve by, so we preserve the prior no-rollout behavior.
    //
    // The recorder is opened BEFORE the spawn so the JSONL file (and its
    // SessionMeta seq=0 line) exists before the first runtime event lands.
    // `thread_id` is the per-turn execution_id; `session_id` ties the turns
    // together. Mirrors the skill path's `ensure_rollout_open`.
    let rollout_session_id = session_id.clone();
    let rollout_recording = if let Some(sid) = rollout_session_id.as_deref() {
        let session_meta = crate::rollout::session_meta_from_chat(&execution_id, sid, args.clone());
        let recorder = state
            .rollout_registry
            .open_for_run(
                state.rollout_paths.clone(),
                Some(state.rollout_index.clone()),
                &execution_id,
                sid,
                session_meta,
            )
            .await;
        // Record the verbatim user prompt as ONE typed UserMessage line at turn
        // start — before any runtime event lands — so the transcript reads
        // user-then-assistant. The runtime separately emits the model-facing
        // prompt as a REDACTED `llm_prompt` event; the history mapper skips that
        // redacted user line once a typed UserMessage is present for the turn,
        // so this faithful text replaces (not duplicates) the redacted summary.
        if let (Some(recorder), Some(text)) = (recorder.as_ref(), user_text.as_deref())
            && !text.is_empty()
        {
            let payload =
                apxm_rollout::RolloutPayload::UserMessage(apxm_rollout::UserMessagePayload {
                    content: vec![apxm_rollout::ContentBlock::Text {
                        text: text.to_string(),
                    }],
                });
            if let Err(error) = recorder
                .write_line(payload, apxm_rollout::PartialMeta::default())
                .await
            {
                tracing::warn!(%error, execution_id, "failed to record user_text rollout line");
            }
        }
        recorder.is_some()
    } else {
        false
    };
    let rollout_registry = state.rollout_registry.clone();
    let run_event_bus = state.run_event_bus.clone();
    let execution_store = state.execution_store.clone();
    let cancellation_token = apxm_runtime::CancellationToken::new();
    // Drop this session's registry record when its execution settles (only if it
    // still points at this execution — a newer turn may have re-registered).
    let session_registry = state.session_registry.clone();
    let session_cleanup = session_id.clone();

    tokio::spawn(async move {
        let send_lifecycle = |event: ApxmEvent| async {
            let event = record_run_lifecycle_event(
                &run_event_bus,
                &rollout_registry,
                &execution_id,
                event,
                rollout_recording,
            );
            let _ = tx_task.send(event).await;
        };
        // The admission slot is owned by the registered handle (released while
        // parked, reacquired on wake); unregister after the run settles.
        // Frame 0 hands the client the id it needs to address the cancel route.
        send_lifecycle(ApxmEvent::root(
            ExecutionStartedPayload {
                execution_id: execution_id.clone(),
            },
            EventSource::Server,
            &trace_id,
        ))
        .await;
        let mut downstream: Vec<Arc<dyn apxm_core::events::EventEmitter>> =
            vec![Arc::new(TokioChannelEmitter::new(tx_task.clone()))];
        if raw_record.is_some() {
            downstream.push(Arc::new(ExecutionRecordingEmitter::new(
                execution_store.clone(),
                execution_id.clone(),
            )));
        }
        if rollout_recording {
            downstream.push(Arc::new(crate::rollout::RolloutEmitter::new(
                rollout_registry.clone(),
                execution_id.clone(),
            )));
        }
        let sink: Arc<dyn apxm_core::events::EventEmitter> = Arc::new(RunBusFanOutEmitter::new(
            run_event_bus.clone(),
            execution_id.clone(),
            downstream,
        ));
        let emitter = Arc::new(EmitterAdapter::new(sink, EventSource::Runtime, &trace_id));
        let execution = runtime
            .execute_artifact_with_session_emitter_metadata_credentials_and_cancellation(
                artifact,
                args,
                session_id,
                Some(emitter),
                session_dir.clone(),
                grant_metadata,
                resolved_credentials,
                cancellation_token.clone(),
            );
        tokio::select! {
            outcome = execution => match outcome {
                Ok(result) => {
                    let mut response = to_execute_response(result, session_dir);
                    if let Some(record) = raw_record.as_ref() {
                        attach_run_metadata(&mut response, record, &trace_id);
                        execution_store.complete_success(&execution_id, response.clone());
                    }
                    send_lifecycle(ApxmEvent::root(
                        ExecuteCompletePayload {
                            result: serde_json::to_value(response).unwrap_or(JsonValue::Null),
                        },
                        EventSource::Server,
                        &trace_id,
                    ))
                    .await;
                }
                Err(err) => {
                    if raw_record.is_some() {
                        execution_store.complete_failure(&execution_id, err.to_string());
                    }
                    send_lifecycle(ApxmEvent::root(
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
                cancellation_token.cancel();
                if raw_record.is_some() {
                    execution_store.complete_failure(&execution_id, "cancelled".to_string());
                }
                send_lifecycle(ApxmEvent::root(
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
        // Flush + close the rollout recorder so the turn's tail (last tokens,
        // llm_done) is durable before the index row is finalized. Idempotent
        // and a no-op when no recorder was opened.
        if rollout_recording {
            rollout_registry.close(&execution_id).await;
        }
        apxm_runtime::scheduler::admission_registry::unregister(&admission_id);
        cancel_registry.remove(&execution_id);
        if let Some(sid) = &session_cleanup {
            session_registry.remove_if_execution(sid, &execution_id);
        }
    });
    drop(tx);

    let stream = async_stream::stream! {
        while let Some(item) = rx.recv().await {
            let id = item.meta.seq.to_string();
            let data = serde_json::to_string(&item).unwrap_or_else(|_| "{}".to_string());
            yield Ok(Event::default().event(item.kind().name()).id(id).data(data));
        }
    };
    Ok(
        Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::new().interval(
            std::time::Duration::from_secs(stream_config.keep_alive_secs.max(1)),
        )),
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
    /// Verbatim user prompt for the durable transcript (streaming path only).
    pub(crate) user_text: Option<String>,
    /// Captured `; __apxm_python_tools__` sidecar (stripped from the AIR text).
    /// Injected as an artifact section only on the operator-trusted python path.
    pub(crate) python_tools_sidecar: Option<Vec<u8>>,
    /// Workspace workflow id forwarded from the caller.
    pub(crate) workflow_id: Option<String>,
}

pub(crate) fn prepare_request(mut req: ExecuteRequest) -> Result<PreparedRequest, ApiError> {
    if req.air.trim().is_empty() {
        return Err(ApiError::bad_request("air must not be empty"));
    }
    let workflow_id = req
        .workflow_id
        .take()
        .filter(|workflow_id| !workflow_id.trim().is_empty())
        .map(validate_workflow_id)
        .transpose()?;
    let session_id_request = match req.session_id.take() {
        Some(session_id) => Some(session_id),
        None if workflow_id.is_some() => {
            Some(format!("apxm-workflow-{}", uuid::Uuid::new_v4().simple()))
        }
        None => None,
    };
    let (session_id, session_dir) =
        resolve_session_request(session_id_request, req.session_root.take())?;
    // Strip + capture the python-tools sidecar comment so the MLIR parser never
    // sees a `;` line, and (on the trusted path) it can be re-attached to the
    // compiled artifact as a section the runtime builds the python bridge from.
    let (air, python_tools_sidecar) = crate::workflow_source::strip_python_tools_sidecar(&req.air);
    Ok(PreparedRequest {
        air,
        args: req.args,
        session_id,
        session_dir,
        admit: req.admit_capabilities.into_iter().collect(),
        imports: req.imports,
        tool_call_budgets: clamp_tool_call_budgets(req.tool_call_budgets),
        tool_credentials: req.tool_credentials,
        owner: req.owner,
        user_text: req.user_text,
        python_tools_sidecar,
        workflow_id,
    })
}

fn start_raw_execution_record(
    state: &AppState,
    execution_id: &str,
    workflow_id: Option<String>,
    trace_id: String,
    session_id: Option<&str>,
    session_dir: Option<&str>,
) -> Option<ExecutionRecord> {
    let workflow_id = workflow_id?;
    let session_id = session_id?;
    let session_dir = session_dir?;
    Some(state.execution_store.start_skill_execution_with_all(
        execution_id.to_string(),
        apxm_skill::SkillExecutionProvenance {
            skill_id: workflow_id.clone(),
            skill_version: "raw-workflow".to_string(),
            entry_flow: None,
            source_hash: None,
            air_hash: None,
            artifact_hash: None,
            parent_execution_id: None,
            parent_skill_id: None,
            parent_skill_version: None,
            scope_id: None,
        },
        session_id,
        session_dir,
        None,
        None,
        Some(workflow_id),
        Some(trace_id),
    ))
}

fn attach_run_metadata(response: &mut ExecuteResponse, record: &ExecutionRecord, trace_id: &str) {
    response.execution_id = Some(record.execution_id.clone());
    response.workflow_id = record.workflow_id.clone();
    response.run_root = record.run_root.clone();
    response.trace_id = Some(trace_id.to_string());
}

fn attach_runtime_run_metadata(
    metadata: &mut HashMap<String, String>,
    record: &ExecutionRecord,
    trace_id: &str,
) -> Result<(), ApiError> {
    metadata.insert(
        apxm_runtime::metadata_keys::EXECUTION_ID.to_string(),
        record.execution_id.clone(),
    );
    metadata.insert(
        apxm_runtime::metadata_keys::WORKFLOW_ID.to_string(),
        record.workflow_id.clone().unwrap_or_default(),
    );
    metadata.insert(
        apxm_runtime::metadata_keys::TRACE_ID.to_string(),
        trace_id.to_string(),
    );
    if let Some(run_root) = record.run_root.as_ref() {
        std::fs::create_dir_all(run_root).map_err(|error| {
            ApiError::internal_message(format!(
                "failed to create run artifact root '{run_root}': {error}"
            ))
        })?;
        metadata.insert(
            apxm_runtime::metadata_keys::RUN_ROOT.to_string(),
            run_root.clone(),
        );
    }
    Ok(())
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

pub(crate) fn validate_session_id(session_id: String) -> Result<String, ApiError> {
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

pub(crate) fn validate_workflow_id(workflow_id: String) -> Result<String, ApiError> {
    let workflow_id = workflow_id.trim().to_string();
    if workflow_id.is_empty()
        || workflow_id == "."
        || workflow_id == ".."
        || !workflow_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ApiError::bad_request(
            "workflow_id must contain only ASCII letters, digits, '-', '_', or '.', and must not be '.' or '..'",
        ));
    }
    Ok(workflow_id)
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
    let artifact_bytes = air_to_artifact_bytes_with_caps(air_text, known_caps)?;
    Artifact::from_bytes(&artifact_bytes)
        .map_err(|error| ApiError::internal_message(format!("failed to decode artifact: {error}")))
}

/// Compile AIR to the raw `.apxmobj` artifact bytes, declaring `known_caps` so
/// the tool-binding check accepts runtime-registered provider/pack capabilities
/// (not just compiler builtins). This is what the studio's compile-on-deploy
/// needs: a registry-aware compile that yields an installable artifact.
pub(crate) fn air_to_artifact_bytes_with_caps(
    air_text: &str,
    known_caps: &std::collections::HashSet<String>,
) -> Result<Vec<u8>, ApiError> {
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
    module
        .generate_artifact_bytes_with_known_caps(known_caps)
        .map_err(|error| ApiError::internal_message(format!("failed to emit artifact: {error}")))
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

/// Single-tenant operator trust gate for running author python on the server.
/// Per ADR 0001 the server stays python-free by default; python is admitted ONLY
/// when the operator explicitly asserts trust AND the worker is sandboxed — both
/// `APXM_TRUST_PYTHON_ARTIFACTS` and `APXM_SANDBOX_PYTHON` must be set. (Full
/// multi-tenant provenance = cryptographic artifact signing, still future work.)
pub(crate) fn python_artifacts_trusted() -> bool {
    std::env::var_os("APXM_TRUST_PYTHON_ARTIFACTS").is_some()
        && std::env::var_os("APXM_SANDBOX_PYTHON").is_some()
}

/// On the operator-trusted python path, re-attach the captured python-tools
/// sidecar to the compiled artifact as a `python_tools` section so the runtime
/// builds the (sandboxed) python bridge from it. No-op when untrusted or absent
/// — the server then stays python-free and the admission guard rejects handlers.
pub(crate) fn attach_trusted_python_section(artifact: &mut Artifact, sidecar: Option<Vec<u8>>) {
    if let Some(data) = sidecar
        && python_artifacts_trusted()
    {
        artifact.add_section(apxm_artifact::ArtifactSection {
            kind: apxm_runtime::python_tools::CAPABILITY_NAME.to_string(),
            data,
        });
    }
}

pub(crate) fn validate_raw_execute_admission(
    artifact: &Artifact,
    state: &AppState,
    admit: &std::collections::HashSet<String>,
) -> Result<(), ApiError> {
    let python_trusted = python_artifacts_trusted();
    if !python_trusted
        && artifact
            .sections()
            .iter()
            .any(|section| section.kind == apxm_runtime::python_tools::CAPABILITY_NAME)
    {
        return Err(ApiError::bad_request(ERROR_RAW_PYTHON_TOOL_SECTIONS));
    }

    // Agents declared as sibling flows in THIS artifact (`<name>.main` /
    // `<name>.delegate`). Spawning them is self-contained — the program carries
    // its own sub-agents — so it needs no external `--admit spawn_agent` grant
    // (server-api admission; constitution #3). External/process spawns still do.
    let in_artifact_agents: std::collections::HashSet<String> = artifact
        .dags()
        .iter()
        .filter_map(|dag| dag.metadata.name.as_deref())
        .filter_map(|name| name.split_once('.').map(|(agent, _flow)| agent.to_string()))
        .collect();

    // Capabilities the artifact provides itself (author @tools, python-backed),
    // read from the python-tools section's manifest `name` fields. On the trusted
    // python path an INV_TOOL of one of these is admitted (the sandboxed bridge
    // serves it) even though it is not in the server registry. Section-based so it
    // is independent of how the capability name is encoded on the node.
    let in_artifact_caps: std::collections::HashSet<String> = artifact
        .section_data(apxm_runtime::python_tools::CAPABILITY_NAME)
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(data).ok())
        .and_then(|v| v.as_array().cloned())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    for dag in artifact.dags() {
        for node in &dag.nodes {
            if !python_trusted && node.attributes.contains_key(graph_attrs::PYTHON_HANDLER_ID) {
                return Err(ApiError::bad_request(ERROR_RAW_PYTHON_TOOL_HANDLERS));
            }

            match node.op_type {
                AISOperationType::InvTool => {
                    validate_raw_inv_tool_node(node, state, admit, &in_artifact_caps)?
                }
                AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason => {
                    validate_raw_llm_tool_exposure(node, state, &in_artifact_caps)?;
                }
                AISOperationType::SpawnAgent => {
                    let self_contained = node
                        .attributes
                        .get(graph_attrs::AGENT_NAME)
                        .and_then(|v| v.as_string())
                        .map(|name| in_artifact_agents.contains(name.as_str()))
                        .unwrap_or(false);
                    if !self_contained {
                        validate_raw_spawn_op_admission(ADMIT_SPAWN_AGENT, node, admit)?;
                    }
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
    in_artifact_caps: &std::collections::HashSet<String>,
) -> Result<(), ApiError> {
    let capability = node
        .attributes
        .get(graph_attrs::CAPABILITY)
        .and_then(|value| value.as_string())
        .ok_or_else(|| ApiError::bad_request(ERROR_INV_TOOL_MISSING_CAPABILITY))?;
    let capability_system = state.runtime.capability_system();

    // On the operator-trusted python path, a capability the artifact declares
    // itself via REGISTER_CAPABILITY (a python @tool) is provided by the
    // sandboxed bridge, not the server registry — admit it.
    if python_artifacts_trusted() && in_artifact_caps.contains(capability) {
        return Ok(());
    }

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

fn validate_raw_llm_tool_exposure(
    node: &Node,
    state: &AppState,
    in_artifact_caps: &std::collections::HashSet<String>,
) -> Result<(), ApiError> {
    let Some(requested_tools) = parse_string_array_attr(node, graph_attrs::TOOLS) else {
        return validate_raw_ask_group_or_all_tools(node, state);
    };
    if requested_tools.is_empty() {
        return validate_raw_ask_group_or_all_tools(node, state);
    }
    validate_read_only_tool_names(&requested_tools, state, in_artifact_caps)
}

/// An ASK node may expose a capability if it is read-only OR an admit-gated
/// authoring capability.
fn ask_exposable_groups(groups: &[String]) -> bool {
    groups
        .iter()
        .any(|g| g == apxm_core::constants::capabilities::groups::AUTHORING)
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

fn validate_read_only_tool_names(
    tool_names: &[String],
    state: &AppState,
    in_artifact_caps: &std::collections::HashSet<String>,
) -> Result<(), ApiError> {
    let capability_system = state.runtime.capability_system();
    let python_trusted = python_artifacts_trusted();
    for tool_name in tool_names {
        // An author @tool the artifact provides itself (python-backed) is served
        // by the sandboxed bridge on the trusted path — exposing it on the ASK is
        // fine even though it is not in the server registry.
        if python_trusted && in_artifact_caps.contains(tool_name) {
            continue;
        }
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

/// Dispatch-time credential resolution. Opt-in via
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

/// Extract the `X-Trace-Id` header value from an HTTP request's header map.
/// Returns `None` when the header is absent or not valid ASCII. The value is
/// trimmed but not otherwise validated; callers are responsible for logging or
/// forwarding it as-is.
pub(crate) fn extract_trace_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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
        execution_id: None,
        workflow_id: None,
        run_root: None,
        trace_id: None,
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
