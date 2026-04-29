use axum::Json;
use serde_json::Value as JsonValue;

use crate::helpers::mcp_tool_result;
use crate::skills::{SkillExecuteRequest, SkillLookupError, execute_skill_by_id};
use crate::state::AppState;

use super::schema::{
    MCP_TOOL_APXM_SKILL_CALL, MCP_TOOL_APXM_SKILL_GET, MCP_TOOL_APXM_SKILL_VALIDATE,
    MCP_TOOL_APXM_SKILLS_LIST, MCP_TOOL_ARG_ARGS, MCP_TOOL_ARG_ID, MCP_TOOL_ARG_SESSION_ID,
};

pub(crate) async fn call_skill_tool(
    state: &AppState,
    id: &JsonValue,
    tool_name: &str,
    tool_args: &JsonValue,
) -> Option<Json<JsonValue>> {
    match tool_name {
        MCP_TOOL_APXM_SKILLS_LIST => {
            Some(mcp_json_tool_result(id.clone(), state.skill_library.scan()))
        }
        MCP_TOOL_APXM_SKILL_GET | MCP_TOOL_APXM_SKILL_VALIDATE => {
            let Some(skill_id) = tool_args.get(MCP_TOOL_ARG_ID).and_then(JsonValue::as_str) else {
                return Some(mcp_tool_result(
                    id.clone(),
                    format!("missing required argument: {MCP_TOOL_ARG_ID}"),
                    true,
                ));
            };
            Some(match state.skill_library.find(skill_id) {
                Ok(record) => mcp_json_tool_result(id.clone(), record),
                Err(error) => mcp_tool_result(id.clone(), skill_lookup_message(error), true),
            })
        }
        MCP_TOOL_APXM_SKILL_CALL => {
            let Some(skill_id) = tool_args.get(MCP_TOOL_ARG_ID).and_then(JsonValue::as_str) else {
                return Some(mcp_tool_result(
                    id.clone(),
                    format!("missing required argument: {MCP_TOOL_ARG_ID}"),
                    true,
                ));
            };
            let args = match parse_string_array_arg(tool_args, MCP_TOOL_ARG_ARGS) {
                Ok(args) => args,
                Err(message) => return Some(mcp_tool_result(id.clone(), message, true)),
            };
            let session_id = match parse_optional_string_arg(tool_args, MCP_TOOL_ARG_SESSION_ID) {
                Ok(session_id) => session_id,
                Err(message) => return Some(mcp_tool_result(id.clone(), message, true)),
            };
            let request = SkillExecuteRequest { args, session_id };
            Some(match execute_skill_by_id(state, skill_id, request).await {
                Ok(response) => mcp_json_tool_result(id.clone(), response),
                Err(error) => mcp_tool_result(id.clone(), error.message, true),
            })
        }
        _ => None,
    }
}

fn parse_string_array_arg(args: &JsonValue, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        return Err(format!("{key} must be an array of strings"));
    };
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| format!("{key} must be an array of strings"))
        })
        .collect()
}

fn parse_optional_string_arg(args: &JsonValue, key: &str) -> Result<Option<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| format!("{key} must be a string"))
}

fn mcp_json_tool_result<T: serde::Serialize>(id: JsonValue, value: T) -> Json<JsonValue> {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|error| {
        serde_json::json!({ "error": format!("failed to serialize MCP result: {error}") })
            .to_string()
    });
    mcp_tool_result(id, text, false)
}

fn skill_lookup_message(error: SkillLookupError) -> String {
    match error {
        SkillLookupError::NotFound(id) => format!("skill not found: {id}"),
        SkillLookupError::Ambiguous(id) => {
            format!("skill id has multiple versions; request {id}@<version>")
        }
    }
}
