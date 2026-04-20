use std::collections::HashMap;

use apxm_core::types::values::Value;
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::helpers::{jsonrpc_err, jsonrpc_ok, mcp_tool_result};
use crate::state::AppState;

// ─── MCP 2025-11-05 JSON-RPC endpoint (/v1/mcp) ─────────────────────────────

/// MCP 2025-11-05 compatible JSON-RPC handler.
///
/// Supports:
/// - `tools/list`  — enumerate APXM capabilities as MCP tools
/// - `tools/call`  — invoke an APXM capability by name
///
/// Wire format: `{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}`
#[derive(Debug, Deserialize)]
pub(crate) struct McpRequest {
    #[serde(default)]
    pub(crate) id: JsonValue,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: JsonValue,
}

pub(crate) async fn mcp_jsonrpc(
    State(state): State<AppState>,
    Json(req): Json<McpRequest>,
) -> Json<JsonValue> {
    let id = req.id.clone();
    match req.method.as_str() {
        "tools/list" => {
            let tools: Vec<JsonValue> = state
                .runtime
                .capability_system()
                .list_capabilities()
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "name": m.name,
                        "description": m.description,
                        "inputSchema": m.parameters_schema,
                    })
                })
                .collect();
            jsonrpc_ok(id, serde_json::json!({ "tools": tools }))
        }
        "tools/call" => {
            let tool_name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tool_args = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(JsonValue::Object(Default::default()));

            // Convert JSON args to Value map
            let args: HashMap<String, Value> = if let JsonValue::Object(map) = &tool_args {
                map.iter()
                    .filter_map(|(k, v)| {
                        Value::try_from(v.clone()).ok().map(|val| (k.clone(), val))
                    })
                    .collect()
            } else {
                HashMap::new()
            };

            let cap_sys = state.runtime.capability_system();
            let executor = cap_sys.registry().get(tool_name);
            match executor {
                None => mcp_tool_result(id, format!("Tool '{}' not found", tool_name), true),
                Some(exec) => match exec.execute(args).await {
                    Ok(result) => {
                        let result_json = result
                            .to_json()
                            .unwrap_or_else(|_| JsonValue::String(result.to_string()));
                        mcp_tool_result(id, result_json.to_string(), false)
                    }
                    Err(e) => mcp_tool_result(id, e.to_string(), true),
                },
            }
        }
        "initialize" => jsonrpc_ok(
            id,
            serde_json::json!({
                "protocolVersion": apxm_core::constants::protocols::MCP_VERSION,
                "serverInfo": {
                    "name": "apxm-server",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "tools": { "listChanged": false },
                },
            }),
        ),
        unknown => jsonrpc_err(id, -32601, format!("Method not found: {}", unknown)),
    }
}
