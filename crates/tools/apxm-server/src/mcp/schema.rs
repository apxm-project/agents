use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::types::responses::ToolEntry;

pub(crate) const MCP_METHOD_TOOLS_LIST: &str = "tools/list";
pub(crate) const MCP_METHOD_TOOLS_CALL: &str = "tools/call";
pub(crate) const MCP_METHOD_INITIALIZE: &str = "initialize";

pub(crate) const MCP_TOOL_APXM_SKILLS_LIST: &str = "apxm_skills_list";
pub(crate) const MCP_TOOL_APXM_SKILL_GET: &str = "apxm_skill_get";
pub(crate) const MCP_TOOL_APXM_SKILL_VALIDATE: &str = "apxm_skill_validate";
pub(crate) const MCP_TOOL_APXM_SKILL_CALL: &str = "apxm_skill_call";

pub(crate) const MCP_TOOL_ARG_ID: &str = "id";
pub(crate) const MCP_TOOL_ARG_ARGS: &str = "args";
pub(crate) const MCP_TOOL_ARG_SESSION_ID: &str = "session_id";
pub(crate) const MCP_TOOL_PARAM_NAME: &str = "name";
pub(crate) const MCP_TOOL_PARAM_ARGUMENTS: &str = "arguments";

/// MCP 2025-11-05 compatible JSON-RPC request.
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

pub(crate) fn skill_tool_entries() -> Vec<ToolEntry> {
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
