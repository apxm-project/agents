use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use serde_json::Value as JsonValue;

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn jsonrpc_ok(id: JsonValue, result: JsonValue) -> Json<JsonValue> {
    Json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
}

pub(crate) fn jsonrpc_err(id: JsonValue, code: i64, message: impl Into<String>) -> Json<JsonValue> {
    Json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    }))
}

pub(crate) fn mcp_tool_result(id: JsonValue, text: String, is_error: bool) -> Json<JsonValue> {
    jsonrpc_ok(
        id,
        serde_json::json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        }),
    )
}
