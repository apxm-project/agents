use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use apxm_artifact::Artifact;
use apxm_compiler::AirModule;
use apxm_compiler::{Context as CompilerContext, Pipeline as CompilerPipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::events::payload::ErrorPayload;
use apxm_core::events::{ApxmEvent, EventSource};
use apxm_core::paths::ApxmPaths;
use apxm_core::types::AISOperationType;
use apxm_core::types::values::Value;
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

#[derive(Debug, Deserialize)]
pub(crate) struct ExecuteRequest {
    pub(crate) graph: JsonValue,
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) session_root: Option<String>,
    #[serde(default)]
    pub(crate) token_budget: Option<u64>,
    #[serde(default)]
    pub(crate) output_schema: Option<JsonValue>,
    #[serde(default)]
    pub(crate) max_schema_retries: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ExecuteResponse {
    pub(crate) results: HashMap<String, JsonValue>,
    pub(crate) content: Option<String>,
    pub(crate) session_dir: Option<String>,
    pub(crate) stats: JsonValue,
    pub(crate) llm_usage: JsonValue,
}

pub(crate) async fn execute(
    State(state): State<AppState>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    let (graph, args, session_id, session_dir) = prepare_request(req)?;
    let artifact = graph_to_artifact(graph)?;
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
    let (graph, args, session_id, session_dir) = prepare_request(req)?;
    let artifact = graph_to_artifact(graph)?;
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
) -> Result<(AirModule, Vec<String>, Option<String>, Option<String>), ApiError> {
    let mut graph: AirModule = serde_json::from_value(req.graph)
        .map_err(|e| ApiError::bad_request(format!("invalid graph: {e}")))?;
    apply_runtime_attributes(
        &mut graph,
        req.token_budget.take(),
        req.output_schema.take(),
        req.max_schema_retries,
    )
    .map_err(ApiError::bad_request)?;
    let (session_id, session_dir) =
        resolve_session_request(req.session_id.take(), req.session_root.take())?;
    Ok((graph, req.args, session_id, session_dir))
}

fn resolve_session_request(
    session_id: Option<String>,
    session_root: Option<String>,
) -> Result<(Option<String>, Option<String>), ApiError> {
    let session_root = session_root
        .map(|root| {
            let trimmed = root.trim();
            if trimmed.is_empty() {
                Err(ApiError::bad_request("session_root must not be empty"))
            } else {
                Ok(PathBuf::from(trimmed))
            }
        })
        .transpose()?;

    if session_root.is_none() && session_id.is_none() {
        return Ok((None, None));
    }

    let resolved_session_id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let base_dir = if let Some(root) = session_root {
        root
    } else {
        ApxmPaths::discover()
            .map_err(|e| ApiError::internal_message(format!("failed to discover APXM paths: {e}")))?
            .sessions_dir()
            .map_err(|e| {
                ApiError::internal_message(format!("failed to resolve sessions dir: {e}"))
            })?
    };

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

pub(crate) fn apply_runtime_attributes(
    graph: &mut AirModule,
    token_budget: Option<u64>,
    output_schema: Option<JsonValue>,
    max_schema_retries: Option<u32>,
) -> Result<(), String> {
    for node in &mut graph.nodes {
        if node.op != AISOperationType::Ask {
            continue;
        }
        if let Some(budget) = token_budget {
            let budget = i64::try_from(budget).unwrap_or(i64::MAX);
            node.attributes.insert(
                graph_attrs::TOKEN_BUDGET.to_string(),
                Value::Number(budget.into()),
            );
        }
        if let Some(schema) = output_schema.as_ref() {
            let schema_value = Value::try_from(schema.clone())
                .map_err(|e| format!("invalid output_schema: {e}"))?;
            node.attributes
                .insert(graph_attrs::OUTPUT_SCHEMA.to_string(), schema_value);
        }
        if let Some(retries) = max_schema_retries {
            let retries = i64::from(retries);
            node.attributes.insert(
                graph_attrs::MAX_SCHEMA_RETRIES.to_string(),
                Value::Number(retries.into()),
            );
        }
    }
    Ok(())
}

pub(crate) fn graph_to_artifact(graph: AirModule) -> Result<Artifact, ApiError> {
    let air_text = graph.to_air().map_err(|error| {
        ApiError::bad_request(format!(
            "failed to lower graph '{}' to AIR: {error}",
            graph.name
        ))
    })?;
    let context = CompilerContext::new().map_err(|error| {
        ApiError::internal_message(format!(
            "failed to initialize APXM compiler context: {error}"
        ))
    })?;
    let pipeline =
        CompilerPipeline::with_opt_level(&context, apxm_core::types::OptimizationLevel::O1);
    let module = pipeline.compile(&air_text).map_err(|error| {
        ApiError::bad_request(format!("failed to compile graph '{}': {error}", graph.name))
    })?;
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
        stats: serde_json::json!({
            "executed_nodes": result.stats.executed_nodes,
            "failed_nodes": result.stats.failed_nodes,
            "duration_ms": result.stats.duration_ms
        }),
        llm_usage: serde_json::json!({
            "input_tokens": result.llm_metrics.total_input_tokens,
            "output_tokens": result.llm_metrics.total_output_tokens,
            "total_requests": result.llm_metrics.total_requests
        }),
    }
}
