use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use apxm_artifact::Artifact;
use apxm_core::events::payload::ErrorPayload;
use apxm_core::events::{ApxmEvent, EventEmitter, EventSource, SkillEventProvenance};
use apxm_runtime::EmitterAdapter;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tokio::sync::Notify;

use crate::error::ApiError;
use crate::execute::{
    ExecuteRequest, PreparedRequest, acquire_admission, admit_grant_metadata,
    air_to_artifact_with_caps, inject_resolved_credentials, prepare_request,
    registered_capability_names, to_execute_response, validate_raw_execute_admission,
};
use crate::executions::{ExecutionRecord, ExecutionStatus};
use crate::helpers::mcp_tool_result;
use crate::runs::{events_for_run, events_for_run_since};
use crate::state::{AppState, ExecuteCompletePayload, ExecutionStartedPayload, TurnAbortedPayload};

pub(crate) const MCP_TOOL_APXM_WORKFLOW_START: &str = "apxm_workflow_start";
pub(crate) const MCP_TOOL_APXM_WORKFLOW_STATUS: &str = "apxm_workflow_status";
pub(crate) const MCP_TOOL_APXM_WORKFLOW_EVENTS: &str = "apxm_workflow_events";
pub(crate) const MCP_TOOL_APXM_WORKFLOW_CANCEL: &str = "apxm_workflow_cancel";

const WORKFLOW_RECORD_ID: &str = "apxm.workflow";
const WORKFLOW_RECORD_ENTRY: &str = "workflow_start";
const DEFAULT_EVENTS_LIMIT: usize = 100;
const MAX_EVENTS_LIMIT: usize = 1000;

#[derive(Debug, Deserialize)]
pub(crate) struct WorkflowStartArgs {
    pub(crate) workflow_path: String,
    #[serde(default)]
    pub(crate) args: JsonMap<String, JsonValue>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) admit_capabilities: Vec<String>,
    #[serde(default)]
    pub(crate) imports: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExecutionIdArgs {
    execution_id: String,
}

#[derive(Debug, Deserialize)]
struct WorkflowEventsArgs {
    execution_id: String,
    #[serde(default)]
    since: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorkflowStartResponse {
    pub(crate) status: ExecutionStatus,
    pub(crate) execution_id: String,
    pub(crate) session_id: String,
    pub(crate) session_dir: String,
    pub(crate) workflow_path: String,
}

#[derive(Debug, Serialize)]
struct WorkflowStatusResponse {
    execution_id: String,
    status: ExecutionStatus,
    session_id: String,
    session_dir: String,
    started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<crate::execute::ExecuteResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    totals: WorkflowTotals,
}

#[derive(Debug, Serialize)]
struct WorkflowEventsResponse {
    execution_id: String,
    events: Vec<JsonValue>,
    next_seq: u64,
    done: bool,
}

#[derive(Debug, Serialize)]
struct WorkflowCancelResponse {
    execution_id: String,
    cancelled: bool,
}

#[derive(Debug, Default, Serialize)]
struct WorkflowTotals {
    events: usize,
    node_outputs: usize,
    node_metrics: usize,
}

struct PreparedWorkflowRun {
    artifact: Artifact,
    execution_id: String,
    session_id: String,
    session_dir: String,
    workflow_path: String,
    metadata: std::collections::HashMap<String, String>,
    admission_id: String,
}

struct RuntimeEventGate {
    closed: Arc<AtomicBool>,
    inner: Arc<dyn EventEmitter>,
}

impl EventEmitter for RuntimeEventGate {
    fn emit(&self, event: ApxmEvent) {
        if !self.closed.load(Ordering::SeqCst) {
            self.inner.emit(event);
        }
    }
}

pub(crate) fn workflow_start_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["workflow_path"],
        "properties": {
            "workflow_path": {
                "type": "string",
                "description": "Server-local .apxmw workflow file to start"
            },
            "args": {
                "type": "object",
                "description": "Named JSON arguments passed to the workflow"
            },
            "session_id": { "type": "string" },
            "admit_capabilities": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Write capabilities the caller grants this workflow run"
            },
            "imports": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Visible skill imports for nested skill calls"
            }
        }
    })
}

pub(crate) fn workflow_status_input_schema() -> JsonValue {
    execution_id_input_schema()
}

pub(crate) fn workflow_cancel_input_schema() -> JsonValue {
    execution_id_input_schema()
}

pub(crate) fn workflow_events_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["execution_id"],
        "properties": {
            "execution_id": { "type": "string" },
            "since": {
                "type": "integer",
                "minimum": 0,
                "description": "Return events whose run-local sequence is >= since"
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_EVENTS_LIMIT,
                "description": "Maximum events to return"
            }
        }
    })
}

pub(crate) async fn call_workflow_tool(
    state: &AppState,
    id: &JsonValue,
    tool_name: &str,
    tool_args: &JsonValue,
) -> Option<Json<JsonValue>> {
    match tool_name {
        MCP_TOOL_APXM_WORKFLOW_START => Some(match start_workflow(state, tool_args).await {
            Ok(response) => mcp_json_tool_result(id.clone(), response),
            Err(error) => mcp_tool_result(id.clone(), error.message, true),
        }),
        MCP_TOOL_APXM_WORKFLOW_STATUS => Some(match workflow_status(state, tool_args).await {
            Ok(response) => mcp_json_tool_result(id.clone(), response),
            Err(error) => mcp_tool_result(id.clone(), error.message, true),
        }),
        MCP_TOOL_APXM_WORKFLOW_EVENTS => Some(match workflow_events(state, tool_args).await {
            Ok(response) => mcp_json_tool_result(id.clone(), response),
            Err(error) => mcp_tool_result(id.clone(), error.message, true),
        }),
        MCP_TOOL_APXM_WORKFLOW_CANCEL => Some(match workflow_cancel(state, tool_args) {
            Ok(response) => mcp_json_tool_result(id.clone(), response),
            Err(error) => mcp_tool_result(id.clone(), error.message, true),
        }),
        _ => None,
    }
}

async fn start_workflow(
    state: &AppState,
    tool_args: &JsonValue,
) -> Result<WorkflowStartResponse, ApiError> {
    let request: WorkflowStartArgs =
        serde_json::from_value(tool_args.clone()).map_err(|error| {
            ApiError::bad_request(format!("invalid workflow_start arguments: {error}"))
        })?;
    start_workflow_from_args(state, request).await
}

pub(crate) async fn start_workflow_from_args(
    state: &AppState,
    request: WorkflowStartArgs,
) -> Result<WorkflowStartResponse, ApiError> {
    let prepared = prepare_workflow_run(state, request).await?;
    let response = WorkflowStartResponse {
        status: ExecutionStatus::Running,
        execution_id: prepared.execution_id.clone(),
        session_id: prepared.session_id.clone(),
        session_dir: prepared.session_dir.clone(),
        workflow_path: prepared.workflow_path.clone(),
    };
    spawn_workflow_run(state.clone(), prepared);
    Ok(response)
}

async fn workflow_status(
    state: &AppState,
    tool_args: &JsonValue,
) -> Result<WorkflowStatusResponse, ApiError> {
    let args: ExecutionIdArgs = serde_json::from_value(tool_args.clone()).map_err(|error| {
        ApiError::bad_request(format!("invalid workflow_status arguments: {error}"))
    })?;
    let record = state
        .execution_store
        .get(&args.execution_id)
        .ok_or_else(|| {
            ApiError::not_found(format!("workflow run not found: {}", args.execution_id))
        })?;
    let events = events_for_run(state, &args.execution_id).await;
    Ok(status_response(record, events.len()))
}

async fn workflow_events(
    state: &AppState,
    tool_args: &JsonValue,
) -> Result<WorkflowEventsResponse, ApiError> {
    let args: WorkflowEventsArgs = serde_json::from_value(tool_args.clone()).map_err(|error| {
        ApiError::bad_request(format!("invalid workflow_events arguments: {error}"))
    })?;
    let since = args.since.unwrap_or(0);
    let events = events_for_run_since(state, &args.execution_id, since).await;
    if events.is_empty() && state.execution_store.get(&args.execution_id).is_none() {
        return Err(ApiError::not_found(format!(
            "workflow run not found: {}",
            args.execution_id
        )));
    }

    let limit = args
        .limit
        .unwrap_or(DEFAULT_EVENTS_LIMIT)
        .clamp(1, MAX_EVENTS_LIMIT);
    let filtered: Vec<&ApxmEvent> = events
        .iter()
        .filter(|event| event.meta.seq >= since)
        .collect();
    let page: Vec<&ApxmEvent> = filtered.iter().take(limit).copied().collect();
    let next_seq = page
        .last()
        .map_or(since, |event| event.meta.seq.saturating_add(1));
    let serialized = page
        .iter()
        .map(|event| serde_json::to_value(event).unwrap_or(JsonValue::Null))
        .collect();
    Ok(WorkflowEventsResponse {
        execution_id: args.execution_id,
        events: serialized,
        next_seq,
        done: filtered.len() <= limit,
    })
}

fn workflow_cancel(
    state: &AppState,
    tool_args: &JsonValue,
) -> Result<WorkflowCancelResponse, ApiError> {
    let args: ExecutionIdArgs = serde_json::from_value(tool_args.clone()).map_err(|error| {
        ApiError::bad_request(format!("invalid workflow_cancel arguments: {error}"))
    })?;
    let notify = state
        .cancel_registry
        .get(&args.execution_id)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "no in-flight workflow run to cancel: {}",
                args.execution_id
            ))
        })?;
    notify.notify_one();
    Ok(WorkflowCancelResponse {
        execution_id: args.execution_id,
        cancelled: true,
    })
}

async fn prepare_workflow_run(
    state: &AppState,
    mut request: WorkflowStartArgs,
) -> Result<PreparedWorkflowRun, ApiError> {
    let workflow_path = resolve_workflow_path(&request.workflow_path)?;
    validate_workflow_args(&request.args)?;
    let air = workflow_spawn_air(&workflow_path, &request.args)?;
    let session_id = request
        .session_id
        .take()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let execute_request = ExecuteRequest {
        air,
        args: Vec::new(),
        session_id: Some(session_id),
        session_root: None,
        admit_capabilities: request.admit_capabilities,
        imports: request.imports,
    };
    let PreparedRequest {
        air,
        args,
        session_id,
        session_dir,
        admit,
        imports,
    } = prepare_request(execute_request)?;
    debug_assert!(args.is_empty(), "workflow wrapper takes no positional args");
    let session_id = session_id.expect("workflow_start always supplies a session_id");
    let session_dir = session_dir.expect("workflow_start always supplies a session_dir");
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

    let execution_id = uuid::Uuid::new_v4().to_string();
    state
        .execution_store
        .start_skill_execution_with_provenance_and_execution_id(
            execution_id.clone(),
            workflow_provenance(),
            &session_id,
            &session_dir,
        );

    Ok(PreparedWorkflowRun {
        artifact,
        execution_id,
        session_id,
        session_dir,
        workflow_path: workflow_path.to_string_lossy().to_string(),
        metadata,
        admission_id,
    })
}

fn spawn_workflow_run(state: AppState, prepared: PreparedWorkflowRun) {
    let cancel = Arc::new(Notify::new());
    state
        .cancel_registry
        .insert(prepared.execution_id.clone(), Arc::clone(&cancel));
    tokio::spawn(async move {
        run_prepared_workflow(state, prepared, cancel).await;
    });
}

async fn run_prepared_workflow(
    state: AppState,
    prepared: PreparedWorkflowRun,
    cancel: Arc<Notify>,
) {
    crate::skills::ensure_rollout_open(
        &state,
        &prepared.execution_id,
        &prepared.session_id,
        WORKFLOW_RECORD_ID,
        env!("CARGO_PKG_VERSION"),
        None,
        None,
        None,
        vec![prepared.workflow_path.clone()],
    )
    .await;
    record_workflow_event(
        &state,
        &prepared.execution_id,
        ApxmEvent::root(
            ExecutionStartedPayload {
                execution_id: prepared.execution_id.clone(),
            },
            EventSource::Server,
            &prepared.execution_id,
        ),
    );

    let runtime_events_closed = Arc::new(AtomicBool::new(false));
    let event_sinks = crate::skills::build_skill_event_sinks(&state, &prepared.execution_id, None)
        .into_iter()
        .map(|inner| {
            Arc::new(RuntimeEventGate {
                closed: Arc::clone(&runtime_events_closed),
                inner,
            }) as Arc<dyn EventEmitter>
        })
        .collect();
    let emitter = Arc::new(
        EmitterAdapter::new(
            Arc::new(apxm_core::events::FanOutEmitter::new(event_sinks)),
            EventSource::Runtime,
            &prepared.execution_id,
        )
        .with_skill_provenance(workflow_event_provenance()),
    );

    let cancellation_token = apxm_runtime::CancellationToken::new();
    let runtime = Arc::clone(&state.runtime);
    let mut execution = tokio::spawn({
        let cancellation_token = cancellation_token.clone();
        let artifact = prepared.artifact;
        let session_id = prepared.session_id.clone();
        let session_dir = prepared.session_dir.clone();
        let metadata = prepared.metadata;
        async move {
            runtime
                .execute_artifact_with_session_emitter_metadata_and_cancellation(
                    artifact,
                    Vec::new(),
                    Some(session_id),
                    Some(emitter),
                    Some(session_dir),
                    metadata,
                    cancellation_token,
                )
                .await
        }
    });

    tokio::select! {
        biased;
        outcome = &mut execution => match outcome {
            Ok(Ok(result)) => {
                runtime_events_closed.store(true, Ordering::SeqCst);
                let response = to_execute_response(result, Some(prepared.session_dir.clone()));
                state.execution_store.complete_success(&prepared.execution_id, response.clone());
                record_workflow_event(
                    &state,
                    &prepared.execution_id,
                    ApxmEvent::root(
                        ExecuteCompletePayload { result: response },
                        EventSource::Server,
                        &prepared.execution_id,
                    ),
                );
            }
            Ok(Err(error)) => {
                runtime_events_closed.store(true, Ordering::SeqCst);
                let message = error.to_string();
                state.execution_store.complete_failure(&prepared.execution_id, message.clone());
                record_workflow_event(
                    &state,
                    &prepared.execution_id,
                    ApxmEvent::root(
                        ErrorPayload {
                            message,
                            status: None,
                            recoverable: false,
                        },
                        EventSource::Server,
                        &prepared.execution_id,
                    ),
                );
            }
            Err(error) => {
                runtime_events_closed.store(true, Ordering::SeqCst);
                let message = format!("workflow runtime task failed: {error}");
                state.execution_store.complete_failure(&prepared.execution_id, message.clone());
                record_workflow_event(
                    &state,
                    &prepared.execution_id,
                    ApxmEvent::root(
                        ErrorPayload {
                            message,
                            status: None,
                            recoverable: false,
                        },
                        EventSource::Server,
                        &prepared.execution_id,
                    ),
                );
            }
        },
        _ = cancel.notified() => {
            runtime_events_closed.store(true, Ordering::SeqCst);
            cancellation_token.cancel();
            let reason = "cancelled via apxm_workflow_cancel".to_string();
            state.execution_store.complete_failure(&prepared.execution_id, reason.clone());
            record_workflow_event(
                &state,
                &prepared.execution_id,
                ApxmEvent::root(
                    TurnAbortedPayload {
                        execution_id: prepared.execution_id.clone(),
                        reason,
                    },
                    EventSource::Server,
                    &prepared.execution_id,
                ),
            );
            tokio::spawn(async move {
                let _ = execution.await;
            });
        }
    }

    apxm_runtime::scheduler::admission_registry::unregister(&prepared.admission_id);
    state.cancel_registry.remove(&prepared.execution_id);
    state.rollout_registry.close(&prepared.execution_id).await;
}

fn status_response(record: ExecutionRecord, event_count: usize) -> WorkflowStatusResponse {
    WorkflowStatusResponse {
        execution_id: record.execution_id,
        status: record.status,
        session_id: record.session_id,
        session_dir: record.session_dir,
        started_at_ms: record.started_at_ms,
        completed_at_ms: record.completed_at_ms,
        result: record.result,
        error: record.error,
        totals: WorkflowTotals {
            events: event_count,
            node_outputs: record.node_outputs.len(),
            node_metrics: record.node_metrics.len(),
        },
    }
}

fn record_workflow_event(state: &AppState, execution_id: &str, event: ApxmEvent) {
    let event = state.run_event_bus.record(execution_id, event);
    state
        .rollout_registry
        .try_record(execution_id, event.clone());
    if let Some(dispatcher) = &state.webhook_dispatcher {
        dispatcher.dispatch(event);
    }
}

fn resolve_workflow_path(raw: &str) -> Result<PathBuf, ApiError> {
    if raw.trim().is_empty() || raw.contains('\0') {
        return Err(ApiError::bad_request("workflow_path must not be empty"));
    }
    let path = Path::new(raw);
    if path.extension().and_then(|ext| ext.to_str()) != Some("apxmw") {
        return Err(ApiError::bad_request(
            "workflow_path must point to a .apxmw file",
        ));
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        ApiError::bad_request(format!("workflow_path is not readable: {raw}: {error}"))
    })?;
    if !canonical.is_file() {
        return Err(ApiError::bad_request(format!(
            "workflow_path is not a file: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn workflow_spawn_air(
    workflow_path: &Path,
    args: &JsonMap<String, JsonValue>,
) -> Result<String, ApiError> {
    let workflow_target = quote_air_string(&workflow_path.to_string_lossy());
    let attrs = workflow_spawn_attrs(args)?;
    Ok(format!(
        r#"module {{
  func.func @apxm_mcp_workflow_start() -> !ais.token attributes {{ais.entry}} {{
    %child = ais.workflow_spawn "workflow_path" {workflow_target}{attrs} : !ais.token
    func.return %child : !ais.token
  }}
}}
"#
    ))
}

fn workflow_spawn_attrs(args: &JsonMap<String, JsonValue>) -> Result<String, ApiError> {
    let mut parts = vec!["await_result = true".to_string()];
    if !args.is_empty() {
        parts.push(format!(
            "args = {}",
            format_json_object_attr(args).map_err(ApiError::bad_request)?
        ));
    }
    Ok(format!(" {{{}}}", parts.join(", ")))
}

fn validate_workflow_args(args: &JsonMap<String, JsonValue>) -> Result<(), ApiError> {
    for key in args.keys() {
        validate_attr_key(key).map_err(ApiError::bad_request)?;
    }
    Ok(())
}

fn format_json_attr(value: &JsonValue) -> Result<String, String> {
    match value {
        JsonValue::Null => Ok(quote_air_string("null")),
        JsonValue::Bool(flag) => Ok(flag.to_string()),
        JsonValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                Ok(format!("{value} : i64"))
            } else if let Some(value) = number.as_u64() {
                let value = i64::try_from(value)
                    .map_err(|_| "workflow args integer exceeds i64 range".to_string())?;
                Ok(format!("{value} : i64"))
            } else {
                let value = number
                    .as_f64()
                    .ok_or_else(|| "workflow args number is not finite".to_string())?;
                if value.fract() == 0.0 {
                    Ok(format!("{value:.1} : f64"))
                } else {
                    Ok(format!("{value} : f64"))
                }
            }
        }
        JsonValue::String(text) => Ok(quote_air_string(text)),
        JsonValue::Array(values) => {
            let rendered = values
                .iter()
                .map(format_json_attr)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            Ok(format!("[{rendered}]"))
        }
        JsonValue::Object(map) => format_json_object_attr(map),
    }
}

fn format_json_object_attr(map: &JsonMap<String, JsonValue>) -> Result<String, String> {
    let mut entries: Vec<(&String, &JsonValue)> = map.iter().collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    let rendered = entries
        .into_iter()
        .map(|(key, value)| {
            validate_attr_key(key)?;
            Ok(format!("{key} = {}", format_json_attr(value)?))
        })
        .collect::<Result<Vec<_>, String>>()?
        .join(", ");
    Ok(format!("{{{rendered}}}"))
}

fn validate_attr_key(key: &str) -> Result<(), String> {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return Err("workflow args keys must not be empty".to_string());
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return Err(format!(
            "workflow arg key '{key}' must start with an ASCII letter, '_', or '$'"
        ));
    }
    if chars.any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '$')) {
        return Err(format!(
            "workflow arg key '{key}' may contain only ASCII letters, digits, '_', '.', or '$'"
        ));
    }
    Ok(())
}

fn quote_air_string(value: &str) -> String {
    format!("\"{}\"", apxm_ais::chat::escape_air_string(value))
}

fn workflow_provenance() -> apxm_skill::SkillExecutionProvenance {
    apxm_skill::SkillExecutionProvenance {
        skill_id: WORKFLOW_RECORD_ID.to_string(),
        skill_version: env!("CARGO_PKG_VERSION").to_string(),
        entry_flow: Some(WORKFLOW_RECORD_ENTRY.to_string()),
        source_hash: None,
        air_hash: None,
        artifact_hash: None,
        parent_execution_id: None,
        parent_skill_id: None,
        parent_skill_version: None,
        scope_id: None,
    }
}

fn workflow_event_provenance() -> SkillEventProvenance {
    SkillEventProvenance {
        skill_id: WORKFLOW_RECORD_ID.to_string(),
        skill_version: env!("CARGO_PKG_VERSION").to_string(),
        parent_skill_id: None,
        parent_execution_id: None,
        flow_name: Some(WORKFLOW_RECORD_ENTRY.to_string()),
    }
}

fn execution_id_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["execution_id"],
        "properties": {
            "execution_id": { "type": "string" }
        }
    })
}

fn mcp_json_tool_result<T: serde::Serialize>(id: JsonValue, value: T) -> Json<JsonValue> {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|error| {
        serde_json::json!({ "error": format!("failed to serialize MCP result: {error}") })
            .to_string()
    });
    mcp_tool_result(id, text, false)
}
