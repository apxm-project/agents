use std::collections::HashMap;

use apxm_core::types::values::Value;
use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::helpers::{jsonrpc_err, jsonrpc_ok, mcp_tool_result};
use crate::skills::{SkillExecuteRequest, SkillLookupError, execute_skill_by_id};
use crate::state::AppState;
use crate::types::responses::{
    McpInitializeCapabilities, McpInitializeResult, ServerInfo, ToolEntry, ToolsCapability,
};

const MCP_METHOD_TOOLS_LIST: &str = "tools/list";
const MCP_METHOD_TOOLS_CALL: &str = "tools/call";
const MCP_METHOD_INITIALIZE: &str = "initialize";

pub(crate) const MCP_TOOL_APXM_SKILLS_LIST: &str = "apxm_skills_list";
pub(crate) const MCP_TOOL_APXM_SKILL_GET: &str = "apxm_skill_get";
pub(crate) const MCP_TOOL_APXM_SKILL_VALIDATE: &str = "apxm_skill_validate";
pub(crate) const MCP_TOOL_APXM_SKILL_CALL: &str = "apxm_skill_call";

const MCP_TOOL_ARG_ID: &str = "id";
const MCP_TOOL_ARG_ARGS: &str = "args";
const MCP_TOOL_ARG_SESSION_ID: &str = "session_id";
const MCP_TOOL_PARAM_NAME: &str = "name";
const MCP_TOOL_PARAM_ARGUMENTS: &str = "arguments";

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
        MCP_METHOD_TOOLS_LIST => {
            let mut tools: Vec<ToolEntry> = skill_tool_entries();
            tools.extend(
                state
                    .runtime
                    .capability_system()
                    .list_capabilities()
                    .iter()
                    .map(|m| ToolEntry {
                        name: m.name.clone(),
                        description: m.description.clone(),
                        input_schema: m.parameters_schema.clone(),
                    }),
            );
            jsonrpc_ok(
                id,
                serde_json::json!({ "tools": serde_json::to_value(&tools).unwrap_or(JsonValue::Null) }),
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

            if let Some(response) = call_skill_tool(&state, &id, tool_name, &tool_args).await {
                return response;
            }

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
        MCP_METHOD_INITIALIZE => {
            let result = McpInitializeResult {
                protocol_version: apxm_core::constants::protocols::MCP_VERSION,
                server_info: ServerInfo {
                    name: "apxm-server",
                    version: env!("CARGO_PKG_VERSION"),
                },
                capabilities: McpInitializeCapabilities {
                    tools: ToolsCapability {
                        list_changed: false,
                    },
                },
            };
            jsonrpc_ok(id, serde_json::to_value(&result).unwrap_or(JsonValue::Null))
        }
        unknown => jsonrpc_err(id, -32601, format!("Method not found: {}", unknown)),
    }
}

fn skill_tool_entries() -> Vec<ToolEntry> {
    vec![
        ToolEntry {
            name: MCP_TOOL_APXM_SKILLS_LIST.to_string(),
            description: "List APXM skills installed in the server-owned skill library".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            }),
        },
        ToolEntry {
            name: MCP_TOOL_APXM_SKILL_GET.to_string(),
            description: "Get one APXM skill manifest and validation status".to_string(),
            input_schema: skill_id_input_schema(),
        },
        ToolEntry {
            name: MCP_TOOL_APXM_SKILL_VALIDATE.to_string(),
            description: "Re-read and validate one installed APXM skill without executing it"
                .to_string(),
            input_schema: skill_id_input_schema(),
        },
        ToolEntry {
            name: MCP_TOOL_APXM_SKILL_CALL.to_string(),
            description: "Execute a static APXM skill from the server-owned skill library"
                .to_string(),
            input_schema: skill_call_input_schema(),
        },
    ]
}

fn skill_id_input_schema() -> JsonValue {
    let mut properties = serde_json::Map::new();
    properties.insert(
        MCP_TOOL_ARG_ID.to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Skill id, or skill id plus @version when multiple versions are installed"
        }),
    );
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [MCP_TOOL_ARG_ID],
        "properties": properties,
    })
}

fn skill_call_input_schema() -> JsonValue {
    let mut properties = serde_json::Map::new();
    properties.insert(
        MCP_TOOL_ARG_ID.to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Skill id, or skill id plus @version when multiple versions are installed"
        }),
    );
    properties.insert(
        MCP_TOOL_ARG_ARGS.to_string(),
        serde_json::json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "Positional skill arguments"
        }),
    );
    properties.insert(
        MCP_TOOL_ARG_SESSION_ID.to_string(),
        serde_json::json!({
            "type": "string",
            "description": "Optional simple session identifier. Path separators and dot-only components are rejected."
        }),
    );
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [MCP_TOOL_ARG_ID],
        "properties": properties,
    })
}

async fn call_skill_tool(
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
