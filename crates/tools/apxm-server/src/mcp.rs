use axum::Json;
use axum::extract::State;
use serde_json::Value as JsonValue;

use crate::helpers::{jsonrpc_err, jsonrpc_ok, mcp_tool_result};
use crate::mcp_protocol::{fields, server_name};
use crate::state::AppState;
use crate::types::responses::{
    McpInitializeCapabilities, McpInitializeResult, ResourcesCapability, ServerInfo, ToolEntry,
    ToolsCapability,
};

mod compiler;
mod dispatch;
mod goal;
mod schema;
mod workflow;

#[allow(unused_imports)]
pub(crate) use goal::{
    MCP_TOOL_APXM_GOAL_CANCEL, MCP_TOOL_APXM_GOAL_EVENTS, MCP_TOOL_APXM_GOAL_START,
    MCP_TOOL_APXM_GOAL_STATUS, post_goal,
};
#[allow(unused_imports)]
pub(crate) use schema::{
    MCP_METHOD_INITIALIZE, MCP_METHOD_RESOURCES_LIST, MCP_METHOD_RESOURCES_READ,
    MCP_METHOD_TOOLS_CALL, MCP_METHOD_TOOLS_LIST, MCP_RESOURCE_PARAM_URI, MCP_TOOL_APXM_AAM_RECALL,
    MCP_TOOL_APXM_CAPABILITY_DISCOVERY, MCP_TOOL_APXM_EVIDENCE_LOOKUP,
    MCP_TOOL_APXM_PROMPT_AS_WORKFLOW, MCP_TOOL_APXM_SKILL_CALL, MCP_TOOL_APXM_SKILL_GET,
    MCP_TOOL_APXM_SKILL_VALIDATE, MCP_TOOL_APXM_SKILLS_LIST, MCP_TOOL_APXM_TRACE_FETCH,
    MCP_TOOL_PARAM_ARGUMENTS, MCP_TOOL_PARAM_NAME, McpRequest,
};
#[allow(unused_imports)]
pub(crate) use workflow::{
    MCP_TOOL_APXM_WORKFLOW_CANCEL, MCP_TOOL_APXM_WORKFLOW_EVENTS, MCP_TOOL_APXM_WORKFLOW_START,
    MCP_TOOL_APXM_WORKFLOW_STATUS,
};

const MCP_ERROR_UNKNOWN_TOOL_PREFIX: &str = "unknown tool";

// ─── MCP 2025-11-25 JSON-RPC endpoint (/v1/mcp) ─────────────────────────────

/// MCP 2025-11-25 compatible JSON-RPC handler.
///
/// Supports:
/// - `tools/list`  — enumerate APXM server tools and capability discovery
/// - `tools/call`  — call APXM server tools; raw capability-name calls are not authority
/// - `resources/list` — enumerate bundled and configured APXM skill resources
/// - `resources/read` — read `skill://...` skill resources
///
pub(crate) async fn mcp_jsonrpc(
    State(state): State<AppState>,
    Json(req): Json<McpRequest>,
) -> Json<JsonValue> {
    let id = req.id.clone();
    match req.method.as_str() {
        MCP_METHOD_RESOURCES_LIST => jsonrpc_ok(
            id,
            serde_json::json!({ (fields::RESOURCES): state.skill_library.list_skill_resources() }),
        ),
        MCP_METHOD_RESOURCES_READ => {
            let Some(uri) = req
                .params
                .get(MCP_RESOURCE_PARAM_URI)
                .and_then(JsonValue::as_str)
            else {
                return jsonrpc_err(id, -32602, "resources/read missing params.uri");
            };
            match state.skill_library.resolve_skill_uri(uri) {
                Ok(content) => jsonrpc_ok(id, serde_json::json!({ (fields::CONTENTS): [content] })),
                Err(error) => jsonrpc_err(id, -32602, error.to_string()),
            }
        }
        MCP_METHOD_TOOLS_LIST => {
            let mut tools: Vec<ToolEntry> = schema::skill_tool_entries();
            // PURE compiler tools (side-effect-free): compile / validate / ops.
            tools.push(ToolEntry {
                name: compiler::MCP_TOOL_APXM_COMPILE.to_string(),
                description:
                    "Compile APXM AIR to an artifact; returns ok + diagnostics (no execution)"
                        .to_string(),
                input_schema: compiler::air_input_schema(),
            });
            tools.push(ToolEntry {
                name: compiler::MCP_TOOL_APXM_VALIDATE.to_string(),
                description:
                    "Validate APXM AIR (compile-check) without executing; returns ok + diagnostics"
                        .to_string(),
                input_schema: compiler::air_input_schema(),
            });
            tools.push(ToolEntry {
                name: compiler::MCP_TOOL_APXM_OPS_LIST.to_string(),
                description:
                    "List the AIS operation vocabulary (name, mnemonic, category, description)"
                        .to_string(),
                input_schema: serde_json::json!({
                    "type": "object", "additionalProperties": false, "properties": {}
                }),
            });
            tools.push(ToolEntry {
                name: compiler::MCP_TOOL_APXM_RUN.to_string(),
                description: "Compile and run an APXM AIR graph; writes require runtime-minted delegated_capability_ids".to_string(),
                input_schema: compiler::run_input_schema(),
            });
            tools.push(ToolEntry {
                name: workflow::MCP_TOOL_APXM_WORKFLOW_START.to_string(),
                description: "Start a server-managed APXM .apxmw workflow in the background; returns execution_id/session handles".to_string(),
                input_schema: workflow::workflow_start_input_schema(),
            });
            tools.push(ToolEntry {
                name: workflow::MCP_TOOL_APXM_WORKFLOW_STATUS.to_string(),
                description: "Fetch status for a server-managed APXM workflow run by execution_id"
                    .to_string(),
                input_schema: workflow::workflow_status_input_schema(),
            });
            tools.push(ToolEntry {
                name: workflow::MCP_TOOL_APXM_WORKFLOW_EVENTS.to_string(),
                description:
                    "Fetch retained events for a server-managed APXM workflow run by execution_id"
                        .to_string(),
                input_schema: workflow::workflow_events_input_schema(),
            });
            tools.push(ToolEntry {
                name: workflow::MCP_TOOL_APXM_WORKFLOW_CANCEL.to_string(),
                description: "Cancel an in-flight server-managed APXM workflow run by execution_id"
                    .to_string(),
                input_schema: workflow::workflow_cancel_input_schema(),
            });
            tools.push(ToolEntry {
                name: goal::MCP_TOOL_APXM_GOAL_START.to_string(),
                description: "Start a server-owned goal run, auto-planning bounded workflow passes when workers are omitted, and returning a stable goal_id for status/events/cancel".to_string(),
                input_schema: goal::goal_start_input_schema(),
            });
            tools.push(ToolEntry {
                name: goal::MCP_TOOL_APXM_GOAL_STATUS.to_string(),
                description: "Fetch aggregate state for a server-owned goal run by goal_id"
                    .to_string(),
                input_schema: goal::goal_status_input_schema(),
            });
            tools.push(ToolEntry {
                name: goal::MCP_TOOL_APXM_GOAL_EVENTS.to_string(),
                description:
                    "Fetch retained aggregate events for a server-owned goal run by goal_id"
                        .to_string(),
                input_schema: goal::goal_events_input_schema(),
            });
            tools.push(ToolEntry {
                name: goal::MCP_TOOL_APXM_GOAL_CANCEL.to_string(),
                description: "Cancel an in-flight server-owned goal run by goal_id".to_string(),
                input_schema: goal::goal_cancel_input_schema(),
            });
            jsonrpc_ok(
                id,
                serde_json::json!({ (fields::TOOLS): serde_json::to_value(&tools).unwrap_or(JsonValue::Null) }),
            )
        }
        MCP_METHOD_TOOLS_CALL => {
            let tool_name = req
                .params
                .get(MCP_TOOL_PARAM_NAME)
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tool_args = req
                .params
                .get(MCP_TOOL_PARAM_ARGUMENTS)
                .cloned()
                .unwrap_or(JsonValue::Object(Default::default()));

            // PURE compiler tools (compile/validate/ops_list) — no execution,
            // no capability invoke, so they bypass the admission gating below.
            if let Some(response) = compiler::call_compiler_tool(&state, &id, tool_name, &tool_args)
            {
                return response;
            }

            // run (side-effecting): compile + run; writes gated by
            // delegated_capability_ids + the runtime invoke-site write boundary.
            if let Some(response) =
                compiler::call_run_tool(&state, &id, tool_name, &tool_args).await
            {
                return response;
            }

            if let Some(response) =
                workflow::call_workflow_tool(&state, &id, tool_name, &tool_args).await
            {
                return response;
            }

            if let Some(response) = goal::call_goal_tool(&state, &id, tool_name, &tool_args).await {
                return response;
            }

            if let Some(response) =
                dispatch::call_skill_tool(&state, &id, tool_name, &tool_args).await
            {
                return response;
            }

            mcp_tool_result(
                id,
                format!(
                    "{MCP_ERROR_UNKNOWN_TOOL_PREFIX}: {tool_name}; use capability_discovery for templates and present runtime-minted delegated_capability_ids when executing"
                ),
                true,
            )
        }
        MCP_METHOD_INITIALIZE => {
            let result = McpInitializeResult {
                protocol_version: apxm_core::constants::protocols::MCP_VERSION,
                server_info: ServerInfo {
                    name: server_name::HTTP,
                    version: env!("CARGO_PKG_VERSION"),
                },
                capabilities: McpInitializeCapabilities {
                    tools: ToolsCapability {
                        list_changed: false,
                    },
                    resources: ResourcesCapability {
                        list_changed: false,
                        subscribe: false,
                    },
                },
            };
            jsonrpc_ok(id, serde_json::to_value(&result).unwrap_or(JsonValue::Null))
        }
        unknown => jsonrpc_err(id, -32601, format!("Method not found: {}", unknown)),
    }
}
