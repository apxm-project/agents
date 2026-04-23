use std::collections::HashMap;
use std::sync::Arc;

use apxm_artifact::Artifact;
use apxm_compiler::AirModule;
use apxm_compiler::{Context as CompilerContext, Pipeline as CompilerPipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::events::payload::ErrorPayload;
use apxm_core::events::{ApxmEvent, EventSource};
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
    graph: JsonValue,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    token_budget: Option<u64>,
    #[serde(default)]
    output_schema: Option<JsonValue>,
    #[serde(default)]
    max_schema_retries: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ExecuteResponse {
    pub(crate) results: HashMap<String, JsonValue>,
    pub(crate) content: Option<String>,
    pub(crate) stats: JsonValue,
    pub(crate) llm_usage: JsonValue,
}

pub(crate) async fn execute(
    State(state): State<AppState>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, ApiError> {
    let (graph, args, session_id) = prepare_request(req)?;
    let artifact = graph_to_artifact(graph)?;
    let execution = state
        .runtime
        .execute_artifact_with_session(artifact, args, session_id)
        .await
        .map_err(ApiError::runtime)?;
    Ok(Json(to_execute_response(execution)))
}

pub(crate) async fn execute_stream(
    State(state): State<AppState>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let (graph, args, session_id) = prepare_request(req)?;
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
                None,
            )
            .await
        {
            Ok(result) => {
                let _ = tx
                    .send(ApxmEvent::root(
                        ExecuteCompletePayload {
                            result: to_execute_response(result),
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
) -> Result<(AirModule, Vec<String>, Option<String>), ApiError> {
    let mut graph: AirModule = serde_json::from_value(req.graph)
        .map_err(|e| ApiError::bad_request(format!("invalid graph: {e}")))?;
    apply_runtime_attributes(
        &mut graph,
        req.token_budget.take(),
        req.output_schema.take(),
        req.max_schema_retries,
    )
    .map_err(ApiError::bad_request)?;
    Ok((graph, req.args, req.session_id))
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

pub(crate) fn to_execute_response(result: apxm_runtime::RuntimeExecutionResult) -> ExecuteResponse {
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
