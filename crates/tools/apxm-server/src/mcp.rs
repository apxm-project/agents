use std::collections::HashMap;

use apxm_core::types::values::Value;
use apxm_runtime::capability::CapabilitySandboxPreflight;
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
mod dispatch_spec;
mod schema;
mod workflow;

#[allow(unused_imports)]
pub(crate) use schema::{
    MCP_METHOD_INITIALIZE, MCP_METHOD_RESOURCES_LIST, MCP_METHOD_RESOURCES_READ,
    MCP_METHOD_TOOLS_CALL, MCP_METHOD_TOOLS_LIST, MCP_RESOURCE_PARAM_URI, MCP_TOOL_APXM_AAM_RECALL,
    MCP_TOOL_APXM_CAPABILITY_LIST, MCP_TOOL_APXM_EVIDENCE_LOOKUP, MCP_TOOL_APXM_PLAN_AS_GRAPH,
    MCP_TOOL_APXM_SKILL_CALL, MCP_TOOL_APXM_SKILL_GET, MCP_TOOL_APXM_SKILL_VALIDATE,
    MCP_TOOL_APXM_SKILLS_LIST, MCP_TOOL_APXM_TRACE_FETCH, MCP_TOOL_PARAM_ARGUMENTS,
    MCP_TOOL_PARAM_NAME, McpRequest,
};
#[allow(unused_imports)]
pub(crate) use workflow::{
    MCP_TOOL_APXM_WORKFLOW_CANCEL, MCP_TOOL_APXM_WORKFLOW_EVENTS, MCP_TOOL_APXM_WORKFLOW_START,
    MCP_TOOL_APXM_WORKFLOW_STATUS,
};

const MCP_ERROR_ARGUMENTS_OBJECT: &str = "arguments must be an object";
const MCP_ERROR_UNKNOWN_TOOL_PREFIX: &str = "unknown tool";
const MCP_ERROR_CAPABILITY_NOT_AGENT_SAFE: &str =
    "capability is not read-only and does not declare sandbox execution";
const MCP_ERROR_SANDBOX_PREFLIGHT_PREFIX: &str = "capability failed sandbox preflight";

// ─── MCP 2025-11-25 JSON-RPC endpoint (/v1/mcp) ─────────────────────────────

/// MCP 2025-11-25 compatible JSON-RPC handler.
///
/// Supports:
/// - `tools/list`  — enumerate APXM capabilities as MCP tools
/// - `tools/call`  — invoke an APXM capability by name
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
                description: "Compile and run an APXM AIR graph; writes require admit_capabilities (gated at the invoke site)".to_string(),
                input_schema: compiler::run_input_schema(),
            });
            tools.push(ToolEntry {
                name: dispatch_spec::MCP_TOOL_APXM_DISPATCH.to_string(),
                description: "Dynamically fan out to sub-agents from a constrained spec (validated + templated to a graph, then run)".to_string(),
                input_schema: dispatch_spec::dispatch_input_schema(),
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
            tools.extend(
                state
                    .runtime
                    .capability_system()
                    .list_capabilities()
                    .iter()
                    .filter(|m| m.read_only)
                    .map(|m| ToolEntry {
                        name: m.name.clone(),
                        description: m.description.clone(),
                        input_schema: m.parameters_schema.clone(),
                    }),
            );
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

            // apxm_run (side-effecting): compile + run; writes gated by
            // admit_capabilities + the runtime invoke-site write boundary.
            if let Some(response) =
                compiler::call_run_tool(&state, &id, tool_name, &tool_args).await
            {
                return response;
            }

            // apxm_dispatch (Tier 2): constrained sub-agent spec -> templated
            // graph -> run (same gating as apxm_run).
            if let Some(response) =
                dispatch_spec::call_dispatch_tool(&state, &id, tool_name, &tool_args).await
            {
                return response;
            }

            if let Some(response) =
                workflow::call_workflow_tool(&state, &id, tool_name, &tool_args).await
            {
                return response;
            }

            if let Some(response) =
                dispatch::call_skill_tool(&state, &id, tool_name, &tool_args).await
            {
                return response;
            }

            // Convert JSON args to Value map. MCP tool calls always carry an
            // argument object; silently treating other shapes as `{}` bypasses
            // required-argument validation for registered capabilities.
            let JsonValue::Object(map) = &tool_args else {
                return mcp_tool_result(id, MCP_ERROR_ARGUMENTS_OBJECT.to_string(), true);
            };
            let mut args = HashMap::new();
            for (key, value) in map {
                let value = match Value::try_from(value.clone()) {
                    Ok(value) => value,
                    Err(error) => {
                        return mcp_tool_result(
                            id,
                            format!("invalid argument '{key}': {error}"),
                            true,
                        );
                    }
                };
                args.insert(key.clone(), value);
            }

            let cap_sys = state.runtime.capability_system();
            if !cap_sys.has_capability(tool_name) {
                return mcp_tool_result(
                    id,
                    format!("{MCP_ERROR_UNKNOWN_TOOL_PREFIX}: {tool_name}"),
                    true,
                );
            }
            if !cap_sys.is_read_only(tool_name) {
                match cap_sys.sandbox_preflight(tool_name, &args) {
                    Ok(CapabilitySandboxPreflight::Sandboxed { .. }) => {}
                    Ok(CapabilitySandboxPreflight::Direct) => {
                        return mcp_tool_result(
                            id,
                            format!("{MCP_ERROR_CAPABILITY_NOT_AGENT_SAFE}: {tool_name}"),
                            true,
                        );
                    }
                    Err(error) => {
                        return mcp_tool_result(
                            id,
                            format!("{MCP_ERROR_SANDBOX_PREFLIGHT_PREFIX}: {tool_name}: {error}"),
                            true,
                        );
                    }
                }
            }
            match cap_sys.invoke(tool_name, args).await {
                Ok(result) => {
                    let result_json = result
                        .to_json()
                        .unwrap_or_else(|_| JsonValue::String(result.to_string()));
                    mcp_tool_result(id, result_json.to_string(), false)
                }
                Err(e) => mcp_tool_result(id, e.to_string(), true),
            }
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
