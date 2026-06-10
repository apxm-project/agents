use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::mcp_protocol::{
    McpMethod, SkillTool, Tier3Tool, args, fields, schema_type, tool_description,
};
use crate::types::responses::ToolEntry;

pub(crate) const MCP_METHOD_TOOLS_LIST: &str = McpMethod::ToolsList.as_str();
pub(crate) const MCP_METHOD_TOOLS_CALL: &str = McpMethod::ToolsCall.as_str();
pub(crate) const MCP_METHOD_RESOURCES_LIST: &str = McpMethod::ResourcesList.as_str();
pub(crate) const MCP_METHOD_RESOURCES_READ: &str = McpMethod::ResourcesRead.as_str();
pub(crate) const MCP_METHOD_INITIALIZE: &str = McpMethod::Initialize.as_str();

pub(crate) const MCP_RESOURCE_PARAM_URI: &str = fields::URI;

pub(crate) const MCP_TOOL_APXM_SKILLS_LIST: &str = SkillTool::List.as_str();
pub(crate) const MCP_TOOL_APXM_SKILL_GET: &str = SkillTool::Get.as_str();
pub(crate) const MCP_TOOL_APXM_SKILL_VALIDATE: &str = SkillTool::Validate.as_str();
pub(crate) const MCP_TOOL_APXM_SKILL_CALL: &str = SkillTool::Call.as_str();
pub(crate) const MCP_TOOL_APXM_PROMPT_AS_WORKFLOW: &str = Tier3Tool::PromptAsWorkflow.as_str();
pub(crate) const MCP_TOOL_APXM_TRACE_FETCH: &str = Tier3Tool::TraceFetch.as_str();
pub(crate) const MCP_TOOL_APXM_AAM_RECALL: &str = Tier3Tool::AamRecall.as_str();
pub(crate) const MCP_TOOL_APXM_EVIDENCE_LOOKUP: &str = Tier3Tool::EvidenceLookup.as_str();
pub(crate) const MCP_TOOL_APXM_CAPABILITY_LIST: &str = Tier3Tool::CapabilityList.as_str();

pub(crate) const MCP_TOOL_ARG_ID: &str = args::ID;
pub(crate) const MCP_TOOL_ARG_ARGS: &str = args::ARGS;
pub(crate) const MCP_TOOL_ARG_SESSION_ID: &str = args::SESSION_ID;
pub(crate) const MCP_TOOL_PARAM_NAME: &str = fields::NAME;
pub(crate) const MCP_TOOL_PARAM_ARGUMENTS: &str = fields::ARGUMENTS;

/// MCP 2025-11-25 compatible JSON-RPC request.
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
    let mut tools = vec![
        ToolEntry {
            name: MCP_TOOL_APXM_SKILLS_LIST.to_string(),
            description: "List APXM skills installed in the server-owned skill library".to_string(),
            input_schema: serde_json::json!({
                (fields::TYPE): schema_type::OBJECT,
                (fields::ADDITIONAL_PROPERTIES): false,
                (fields::PROPERTIES): {}
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
    ];
    tools.extend(tier3_tool_entries());
    tools
}

pub(crate) fn tier3_tool_entries() -> Vec<ToolEntry> {
    vec![
        ToolEntry {
            name: MCP_TOOL_APXM_PROMPT_AS_WORKFLOW.to_string(),
            description: tool_description::tier3(Tier3Tool::PromptAsWorkflow).to_string(),
            input_schema: prompt_as_workflow_input_schema(),
        },
        ToolEntry {
            name: MCP_TOOL_APXM_TRACE_FETCH.to_string(),
            description: tool_description::tier3(Tier3Tool::TraceFetch).to_string(),
            input_schema: trace_fetch_input_schema(),
        },
        ToolEntry {
            name: MCP_TOOL_APXM_AAM_RECALL.to_string(),
            description: tool_description::tier3(Tier3Tool::AamRecall).to_string(),
            input_schema: query_input_schema(),
        },
        ToolEntry {
            name: MCP_TOOL_APXM_EVIDENCE_LOOKUP.to_string(),
            description: tool_description::tier3(Tier3Tool::EvidenceLookup).to_string(),
            input_schema: evidence_lookup_input_schema(),
        },
        ToolEntry {
            name: MCP_TOOL_APXM_CAPABILITY_LIST.to_string(),
            description: tool_description::tier3(Tier3Tool::CapabilityList).to_string(),
            input_schema: query_input_schema(),
        },
    ]
}

fn skill_id_input_schema() -> JsonValue {
    let mut properties = serde_json::Map::new();
    properties.insert(
        MCP_TOOL_ARG_ID.to_string(),
        serde_json::json!({
            (fields::TYPE): schema_type::STRING,
            (fields::DESCRIPTION): "Skill id, or skill id plus @version when multiple versions are installed"
        }),
    );
    serde_json::json!({
        (fields::TYPE): schema_type::OBJECT,
        (fields::ADDITIONAL_PROPERTIES): false,
        (fields::REQUIRED): [MCP_TOOL_ARG_ID],
        (fields::PROPERTIES): properties,
    })
}

fn skill_call_input_schema() -> JsonValue {
    let mut properties = serde_json::Map::new();
    properties.insert(
        MCP_TOOL_ARG_ID.to_string(),
        serde_json::json!({
            (fields::TYPE): schema_type::STRING,
            (fields::DESCRIPTION): "Skill id, or skill id plus @version when multiple versions are installed"
        }),
    );
    properties.insert(
        MCP_TOOL_ARG_ARGS.to_string(),
        serde_json::json!({
            (fields::TYPE): schema_type::ARRAY,
            (fields::ITEMS): { (fields::TYPE): schema_type::STRING },
            (fields::DESCRIPTION): "Positional skill arguments"
        }),
    );
    properties.insert(
        MCP_TOOL_ARG_SESSION_ID.to_string(),
        serde_json::json!({
            (fields::TYPE): schema_type::STRING,
            (fields::DESCRIPTION): "Optional simple session identifier. Path separators and dot-only components are rejected."
        }),
    );
    serde_json::json!({
        (fields::TYPE): schema_type::OBJECT,
        (fields::ADDITIONAL_PROPERTIES): false,
        (fields::REQUIRED): [MCP_TOOL_ARG_ID],
        (fields::PROPERTIES): properties,
    })
}

fn prompt_as_workflow_input_schema() -> JsonValue {
    serde_json::json!({
        (fields::TYPE): schema_type::OBJECT,
        (fields::ADDITIONAL_PROPERTIES): false,
        (fields::REQUIRED): [args::TASK],
        (fields::PROPERTIES): {
            (args::TASK): {
                (fields::TYPE): schema_type::STRING,
                (fields::DESCRIPTION): "Natural-language task to convert into an APXM execution workflow"
            },
            (args::CONTEXT): {
                (fields::TYPE): schema_type::STRING,
                (fields::DESCRIPTION): "Optional context that should shape the workflow"
            },
            (args::CONSTRAINTS): {
                (fields::TYPE): schema_type::OBJECT,
                (fields::DESCRIPTION): "Optional structured constraints for the workflow emitter"
            },
            (args::PARAMETERS): {
                (fields::TYPE): schema_type::OBJECT,
                (fields::DESCRIPTION): "Optional runtime parameter values keyed by emitted parameter name"
            },
            (args::EXECUTE): {
                (fields::TYPE): schema_type::BOOLEAN,
                (fields::DESCRIPTION): "Whether to execute after successful compile. Default: true"
            },
            (args::TRACE_ID): {
                (fields::TYPE): schema_type::STRING,
                (fields::DESCRIPTION): "Optional caller-provided trace id"
            }
        }
    })
}

fn trace_fetch_input_schema() -> JsonValue {
    serde_json::json!({
        (fields::TYPE): schema_type::OBJECT,
        (fields::ADDITIONAL_PROPERTIES): false,
        (fields::REQUIRED): [args::TRACE_ID],
        (fields::PROPERTIES): {
            (args::TRACE_ID): {
                (fields::TYPE): schema_type::STRING,
                (fields::DESCRIPTION): "Execution trace id returned by prompt_as_workflow or skill execution"
            },
            (args::NODE_ID): {
                (fields::TYPE): schema_type::INTEGER,
                (fields::MINIMUM): 1,
                (fields::DESCRIPTION): "Optional node id to focus the trace response"
            },
            (args::FULL): {
                (fields::TYPE): schema_type::BOOLEAN,
                (fields::DESCRIPTION): "Return the full execution record instead of a compact summary"
            }
        }
    })
}

fn query_input_schema() -> JsonValue {
    serde_json::json!({
        (fields::TYPE): schema_type::OBJECT,
        (fields::ADDITIONAL_PROPERTIES): false,
        (fields::PROPERTIES): {
            (args::QUERY): {
                (fields::TYPE): schema_type::STRING,
                (fields::DESCRIPTION): "Optional substring query"
            },
            (args::TOP_K): {
                (fields::TYPE): schema_type::INTEGER,
                (fields::MINIMUM): 1,
                (fields::MAXIMUM): 100,
                (fields::DESCRIPTION): "Maximum number of entries to return. Default: 10"
            }
        }
    })
}

fn evidence_lookup_input_schema() -> JsonValue {
    serde_json::json!({
        (fields::TYPE): schema_type::OBJECT,
        (fields::ADDITIONAL_PROPERTIES): false,
        (fields::PROPERTIES): {
            (args::QUERY): {
                (fields::TYPE): schema_type::STRING,
                (fields::DESCRIPTION): "Evidence text or filename query"
            },
            (args::CLAIM_ID): {
                (fields::TYPE): schema_type::STRING,
                (fields::DESCRIPTION): "Claim id to search for"
            },
            (args::PATH): {
                (fields::TYPE): schema_type::STRING,
                (fields::DESCRIPTION): "Optional evidence path under allowed APXM evidence roots"
            },
            (args::LIMIT): {
                (fields::TYPE): schema_type::INTEGER,
                (fields::MINIMUM): 1,
                (fields::MAXIMUM): 100,
                (fields::DESCRIPTION): "Maximum matches to return. Default: 10"
            }
        }
    })
}
