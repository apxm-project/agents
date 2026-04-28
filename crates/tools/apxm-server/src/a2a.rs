use std::collections::HashMap;

use apxm_compiler::{AirEdge, AirModule, AirNode};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::values::Value;
use apxm_core::types::{AISOperationType, DependencyType};
use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::execute::{air_module_to_artifact, to_execute_response};
use crate::helpers::{jsonrpc_err, jsonrpc_ok, now_ms};
use crate::mcp::McpRequest;
use crate::state::AppState;
use crate::types::responses::{A2aTaskFailure, A2aTaskSuccess};

/// Lifecycle state of an A2A task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum A2aState {
    Submitted,
    Working,
    Completed,
    Failed,
    Canceled,
}

/// In-memory record for a running or completed A2A task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct A2aTaskRecord {
    pub(crate) id: String,
    pub(crate) state: A2aState,
    #[serde(default)]
    pub(crate) output_text: Option<String>,
    #[serde(default)]
    pub(crate) error_message: Option<String>,
    pub(crate) created_at_ms: u64,
    #[serde(default)]
    pub(crate) completed_at_ms: Option<u64>,
}

/// Inbound A2A message part — text or opaque data.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum A2aPart {
    Text {
        text: String,
    },
    Data {
        #[allow(dead_code)]
        data: JsonValue,
    },
}

/// Inbound A2A message envelope.
#[derive(Debug, Deserialize)]
pub(crate) struct A2aMessage {
    #[allow(dead_code)]
    role: String,
    parts: Vec<A2aPart>,
}

/// Body for `POST /a2a/tasks/send`.
#[derive(Debug, Deserialize)]
pub(crate) struct A2aSendTaskRequest {
    id: String,
    message: A2aMessage,
    #[serde(default)]
    #[allow(dead_code)]
    metadata: Option<JsonValue>,
}

// ─── A2A v0.3 JSON-RPC endpoint (/a2a) ───────────────────────────────────────

/// A2A v0.3 JSON-RPC handler.
///
/// Supports:
/// - `tasks/send`    — submit a task for execution
/// - `tasks/get`     — retrieve task status (stored in memory as fact)
/// - `tasks/cancel`  — cancel a pending task (best-effort)
///
/// Wire format: `{"jsonrpc":"2.0","id":1,"method":"tasks/send","params":{...}}`
pub(crate) async fn a2a_jsonrpc(
    State(state): State<AppState>,
    Json(req): Json<McpRequest>,
) -> Json<JsonValue> {
    let id = req.id.clone();
    match req.method.as_str() {
        "tasks/send" => {
            // Extract task from params
            let task_id = uuid::Uuid::new_v4().to_string();
            let message = req
                .params
                .get("message")
                .cloned()
                .unwrap_or(req.params.clone());
            let text = message.to_string();
            let source = "a2a".to_string();

            // Store the incoming task in memory so agents can retrieve it
            match state
                .runtime
                .memory()
                .store_fact(&text, &["a2a:task".to_string()], &source, None)
                .await
            {
                Ok(fact_id) => jsonrpc_ok(
                    id,
                    serde_json::json!({
                        "id": task_id,
                        "factId": fact_id,
                        "status": { "state": apxm_core::types::SessionStatus::Submitted },
                        "message": message,
                    }),
                ),
                Err(e) => jsonrpc_err(id, -32000, e.to_string()),
            }
        }
        "tasks/get" => {
            let task_id = req.params.get("id").and_then(|v| v.as_str()).unwrap_or("");
            // Tasks are stored as memory facts — search by task ID
            match state.runtime.memory().search_facts(task_id, 1).await {
                Ok(facts) if !facts.is_empty() => jsonrpc_ok(
                    id,
                    serde_json::json!({
                        "id": task_id,
                        "status": { "state": "completed" },
                        "facts": serde_json::to_value(&facts).unwrap_or(JsonValue::Null),
                    }),
                ),
                Ok(_) => jsonrpc_err(id, -32001, format!("Task '{}' not found", task_id)),
                Err(e) => jsonrpc_err(id, -32000, e.to_string()),
            }
        }
        "tasks/cancel" => {
            // Best-effort cancellation — APXM runtime doesn't currently support mid-flight cancel
            jsonrpc_ok(id, serde_json::json!({ "cancelled": true }))
        }
        unknown => jsonrpc_err(id, -32601, format!("A2A method not found: {}", unknown)),
    }
}

// ─── A2A v0.3 REST Handlers ───────────────────────────────────────────────────

/// `POST /a2a/tasks/send`
///
/// Accepts an A2A v0.3 task, constructs a minimal CONST_STR → ASK graph from
/// the text parts, executes it immediately via the APXM runtime, and returns
/// the result in A2A response format.  The task record is stored in
/// `AppState.a2a_tasks` for subsequent `GET /a2a/tasks/{id}` polling.
pub(crate) async fn a2a_send_task(
    State(state): State<AppState>,
    Json(req): Json<A2aSendTaskRequest>,
) -> impl IntoResponse {
    let now = now_ms();
    state.a2a_tasks.insert(
        req.id.clone(),
        A2aTaskRecord {
            id: req.id.clone(),
            state: A2aState::Working,
            output_text: None,
            error_message: None,
            created_at_ms: now,
            completed_at_ms: None,
        },
    );

    // Concatenate all text parts into a single prompt.
    let user_text: String = req
        .message
        .parts
        .iter()
        .filter_map(|p| match p {
            A2aPart::Text { text } => Some(text.clone()),
            A2aPart::Data { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if user_text.is_empty() {
        if let Some(mut record) = state.a2a_tasks.get_mut(&req.id) {
            record.state = A2aState::Failed;
            record.error_message = Some("No text content in message".to_string());
            record.completed_at_ms = Some(now_ms());
        }
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(A2aTaskFailure::new(
                &req.id,
                "No text content in A2A message parts",
            )),
        )
            .into_response();
    }

    // Build a minimal CONST_STR → ASK AirModule from the inbound text.
    let graph = AirModule {
        name: format!("a2a_{}", req.id),
        nodes: vec![
            AirNode {
                id: 1,
                name: "input".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([("value".to_string(), Value::String(user_text))]),
            },
            AirNode {
                id: 2,
                name: "response".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([(
                    graph_attrs::TEMPLATE_STR.to_string(),
                    Value::String("Complete the following task:\n{0}".to_string()),
                )]),
            },
        ],
        edges: vec![AirEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Data,
        }],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let artifact = match air_module_to_artifact(graph) {
        Ok(a) => a,
        Err(e) => {
            if let Some(mut record) = state.a2a_tasks.get_mut(&req.id) {
                record.state = A2aState::Failed;
                record.error_message = Some(e.message.clone());
                record.completed_at_ms = Some(now_ms());
            }
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(A2aTaskFailure::new(&req.id, &e.message)),
            )
                .into_response();
        }
    };

    match state
        .runtime
        .execute_artifact_with_session(artifact, vec![], None)
        .await
    {
        Ok(result) => {
            let resp = to_execute_response(result, None);
            let output = resp
                .content
                .clone()
                .unwrap_or_else(|| serde_json::to_string(&resp.results).unwrap_or_default());
            if let Some(mut record) = state.a2a_tasks.get_mut(&req.id) {
                record.state = A2aState::Completed;
                record.output_text = Some(output.clone());
                record.completed_at_ms = Some(now_ms());
            }
            Json(A2aTaskSuccess::completed(req.id.clone(), output)).into_response()
        }
        Err(e) => {
            if let Some(mut record) = state.a2a_tasks.get_mut(&req.id) {
                record.state = A2aState::Failed;
                record.error_message = Some(e.to_string());
                record.completed_at_ms = Some(now_ms());
            }
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(A2aTaskFailure::new(&req.id, e.to_string())),
            )
                .into_response()
        }
    }
}

/// `GET /a2a/tasks/{id}` — return status and result of a previously submitted A2A task.
pub(crate) async fn a2a_get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.a2a_tasks.get(&id) {
        Some(record) => {
            let mut body = serde_json::json!({"id": record.id, "status": {"state": record.state}});
            if let Some(ref text) = record.output_text {
                body["result"] = serde_json::json!({"message": {"role": "agent", "parts": [{"type": "text", "text": text}]}});
            }
            if let Some(ref err) = record.error_message {
                body["error"] = serde_json::json!({"message": err});
            }
            Json(body).into_response()
        }
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Task '{}' not found", id)})),
        )
            .into_response(),
    }
}
