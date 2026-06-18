//! PURE (side-effect-free) MCP compiler tools: `compile`, `validate`,
//! `ops_list`.
//!
//! These expose the APXM compiler over the existing `/v1/mcp` JSON-RPC facade so
//! any MCP client (the `apxm chat` agent, apxm-studio, Claude Code) can compile
//! and validate APXM AIR and discover the op vocabulary. They never execute a
//! graph and never invoke a capability, so they need no admission/no-widen
//! gating — that boundary applies to side-effecting execution tools such as
//! `run`, native workflow control, and goal starts.

use axum::Json;
use serde_json::Value as JsonValue;

use crate::execute::{air_to_artifact_with_caps, registered_capability_names};
use crate::helpers::mcp_tool_result;
use crate::state::AppState;
use crate::workflow_source;

pub(crate) const MCP_TOOL_APXM_COMPILE: &str = "compile";
pub(crate) const MCP_TOOL_APXM_VALIDATE: &str = "validate";
pub(crate) const MCP_TOOL_APXM_OPS_LIST: &str = "ops_list";
pub(crate) const MCP_TOOL_APXM_RUN: &str = "run";

/// Input schema for `run`.
pub(crate) fn run_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "air": { "type": "string", "description": "APXM AIR (MLIR) source to run" },
            "path": { "type": "string", "description": "Path to a .air file or Python frontend file that emits AIR" },
            "args": { "type": "array", "items": { "type": "string" }, "description": "positional string args bound to workflow parameters" },
            "session_id": { "type": "string" },
            "admit_capabilities": { "type": "array", "items": { "type": "string" }, "description": "write capabilities the caller grants this run" }
        }
    })
}

/// Side-effecting MCP tool: compile + run AIR, returning the
/// `ExecuteResponse`. This is the safe "agent writes IR -> dispatch" path —
/// writes are gated by `admit_capabilities` (static pre-flight) AND the runtime
/// invoke-site write boundary. Returns `Some(response)` if `tool_name` is it.
pub(crate) async fn call_run_tool(
    state: &AppState,
    id: &JsonValue,
    tool_name: &str,
    args: &JsonValue,
) -> Option<Json<JsonValue>> {
    if tool_name != MCP_TOOL_APXM_RUN {
        return None;
    }
    let args = match args_with_air_source(args) {
        Ok(args) => args,
        Err(error) => return Some(mcp_tool_result(id.clone(), error, true)),
    };
    // Parse the arguments object directly into ExecuteRequest (air/args/
    // session_id/admit_capabilities) — not the Value->HashMap coercion path.
    let req: crate::execute::ExecuteRequest = match serde_json::from_value(args) {
        Ok(req) => req,
        Err(error) => {
            return Some(mcp_tool_result(
                id.clone(),
                format!("invalid run arguments: {error}"),
                true,
            ));
        }
    };
    match crate::execute::run_air_inner(state, req, None).await {
        Ok(response) => {
            let text = serde_json::to_string(&response)
                .unwrap_or_else(|error| format!("{{\"error\":\"serialize failed: {error}\"}}"));
            Some(mcp_tool_result(id.clone(), text, false))
        }
        Err(error) => Some(mcp_tool_result(id.clone(), error.message, true)),
    }
}

/// Input schema shared by compile + validate.
pub(crate) fn air_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "air": { "type": "string", "description": "APXM AIR (MLIR) source text" },
            "path": { "type": "string", "description": "Path to a .air file or Python frontend file that emits AIR" }
        }
    })
}

/// Intercept the PURE compiler tools. Returns `Some(response)` if `tool_name`
/// is one of them, else `None` so the caller falls through to other dispatch.
pub(crate) fn call_compiler_tool(
    state: &AppState,
    id: &JsonValue,
    tool_name: &str,
    args: &JsonValue,
) -> Option<Json<JsonValue>> {
    match tool_name {
        MCP_TOOL_APXM_COMPILE => Some(compile_tool(state, id.clone(), args, false)),
        MCP_TOOL_APXM_VALIDATE => Some(compile_tool(state, id.clone(), args, true)),
        MCP_TOOL_APXM_OPS_LIST => Some(ops_list_tool(id.clone())),
        _ => None,
    }
}

fn compile_tool(
    state: &AppState,
    id: JsonValue,
    args: &JsonValue,
    validate_only: bool,
) -> Json<JsonValue> {
    let air = match air_source_from_args(args) {
        Ok(air) => air,
        Err(error) => return mcp_tool_result(id, error, true),
    };
    let known = registered_capability_names(state);
    // A compile failure is a valid tool RESULT (ok:false + diagnostics), not a
    // JSON-RPC protocol error — so isError stays false and the agent can read
    // the diagnostics and repair the AIR in its loop.
    let payload = match air_to_artifact_with_caps(&air, &known) {
        Ok(_artifact) => serde_json::json!({
            "ok": true,
            "diagnostics": [],
            "mode": if validate_only { "validate" } else { "compile" },
        }),
        Err(error) => serde_json::json!({
            "ok": false,
            "diagnostics": [error.message],
            "mode": if validate_only { "validate" } else { "compile" },
        }),
    };
    mcp_tool_result(id, payload.to_string(), false)
}

fn args_with_air_source(args: &JsonValue) -> Result<JsonValue, String> {
    let mut args = args.clone();
    let air = air_source_from_args(&args)?;
    let Some(object) = args.as_object_mut() else {
        return Err("tool arguments must be an object".to_string());
    };
    object.insert("air".to_string(), JsonValue::String(air));
    object.remove("path");
    Ok(args)
}

fn air_source_from_args(args: &JsonValue) -> Result<String, String> {
    workflow_source::air_from_args(args)
}

fn ops_list_tool(id: JsonValue) -> Json<JsonValue> {
    let operations: Vec<JsonValue> = apxm_ais::get_all_operations()
        .map(|spec| {
            serde_json::json!({
                "name": spec.name,
                "mnemonic": spec.op_type.mlir_mnemonic(),
                "category": serde_json::to_value(spec.category).unwrap_or(JsonValue::Null),
                "description": spec.description,
            })
        })
        .collect();
    mcp_tool_result(
        id,
        serde_json::json!({ "operations": operations }).to_string(),
        false,
    )
}
