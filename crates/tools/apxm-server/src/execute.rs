use std::collections::HashMap;
use std::sync::Arc;

use apxm_artifact::Artifact;
use apxm_compiler::AirModule;
use apxm_compiler::{Context as CompilerContext, Pipeline as CompilerPipeline};
use apxm_core::events::payload::ErrorPayload;
use apxm_core::events::{ApxmEvent, EventSource};
use apxm_core::paths::ApxmPaths;
use apxm_runtime::EmitterAdapter;
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

#[derive(Debug, Deserialize)]
pub(crate) struct ExecuteRequest {
    pub(crate) air: String,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) session_root: Option<String>,
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
    let (air, args, session_id, session_dir) = prepare_request(req)?;
    let artifact = air_to_artifact(&air)?;
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
    let (air, args, session_id, session_dir) = prepare_request(req)?;
    let artifact = air_to_artifact(&air)?;
    let (tx, mut rx) = mpsc::channel::<ApxmEvent>(128);
    let runtime = Arc::clone(&state.runtime);
    let trace_id = session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    tokio::spawn(async move {
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
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub(crate) fn prepare_request(
    mut req: ExecuteRequest,
) -> Result<(String, Vec<String>, Option<String>, Option<String>), ApiError> {
    if req.air.trim().is_empty() {
        return Err(ApiError::bad_request("air must not be empty"));
    }
    let (session_id, session_dir) =
        resolve_session_request(req.session_id.take(), req.session_root.take())?;
    Ok((req.air, req.args, session_id, session_dir))
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
        .generate_artifact_bytes()
        .map_err(|error| ApiError::internal_message(format!("failed to emit artifact: {error}")))?;
    Artifact::from_bytes(&artifact_bytes)
        .map_err(|error| ApiError::internal_message(format!("failed to decode artifact: {error}")))
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
