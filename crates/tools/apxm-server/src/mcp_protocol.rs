#![allow(dead_code)]
// This module is compiled by both the HTTP gateway and the standalone stdio
// binary. Some registry entries are target-specific, but keeping them together
// prevents protocol string drift between the two MCP surfaces.

use apxm_core::types::{OperationCategory, OperationLatency};

pub(crate) mod fields {
    pub(crate) const ADDITIONAL_PROPERTIES: &str = "additionalProperties";
    pub(crate) const ARGUMENTS: &str = apxm_core::constants::mcp::fields::ARGUMENTS;
    pub(crate) const CAPABILITIES: &str = "capabilities";
    pub(crate) const CONTENT: &str = apxm_core::constants::mcp::fields::CONTENT;
    pub(crate) const CONTENTS: &str = "contents";
    pub(crate) const DESCRIPTION: &str = "description";
    pub(crate) const INPUT_SCHEMA: &str = "inputSchema";
    pub(crate) const IS_ERROR: &str = apxm_core::constants::mcp::fields::IS_ERROR;
    pub(crate) const ITEMS: &str = "items";
    pub(crate) const LIST_CHANGED: &str = "listChanged";
    pub(crate) const MAXIMUM: &str = "maximum";
    pub(crate) const MIME_TYPE: &str = "mimeType";
    pub(crate) const MINIMUM: &str = "minimum";
    pub(crate) const NAME: &str = apxm_core::constants::mcp::fields::NAME;
    pub(crate) const PROPERTIES: &str = "properties";
    pub(crate) const PROTOCOL_VERSION: &str = "protocolVersion";
    pub(crate) const REQUIRED: &str = "required";
    pub(crate) const RESOURCES: &str = "resources";
    pub(crate) const SERVER_INFO: &str = "serverInfo";
    pub(crate) const SUBSCRIBE: &str = "subscribe";
    pub(crate) const TEXT: &str = apxm_core::constants::mcp::fields::TEXT;
    pub(crate) const TOOLS: &str = "tools";
    pub(crate) const TYPE: &str = "type";
    pub(crate) const URI: &str = "uri";
    pub(crate) const VERSION: &str = "version";
}

pub(crate) mod args {
    pub(crate) const AIR: &str = "air";
    pub(crate) const ARGS: &str = "args";
    pub(crate) const CLAIM_ID: &str = "claim_id";
    pub(crate) const CONTEXT: &str = "context";
    pub(crate) const CONSTRAINTS: &str = "constraints";
    pub(crate) const EXECUTE: &str = "execute";
    pub(crate) const FULL: &str = "full";
    pub(crate) const ID: &str = "id";
    pub(crate) const LIMIT: &str = "limit";
    pub(crate) const NODE_ID: &str = "node_id";
    pub(crate) const OPT_LEVEL: &str = "opt_level";
    pub(crate) const PARAMETERS: &str = "parameters";
    pub(crate) const PATH: &str = "path";
    pub(crate) const QUERY: &str = "query";
    pub(crate) const SESSION_ID: &str = "session_id";
    pub(crate) const SPACE: &str = "space";
    pub(crate) const TASK: &str = "task";
    pub(crate) const TOP_K: &str = "top_k";
    pub(crate) const TRACE_ID: &str = "trace_id";
}

pub(crate) mod tool_result {
    pub(crate) const AAM: &str = "aam";
    pub(crate) const AIR_CONTRACT: &str = "air_contract";
    pub(crate) const AIR_HASH: &str = "air_hash";
    pub(crate) const AIR_PATH: &str = "air_path";
    pub(crate) const AIR_TEXT: &str = "air";
    pub(crate) const ARTIFACT_BYTES: &str = "artifact_bytes";
    pub(crate) const ARTIFACT_HASH: &str = "artifact_hash";
    pub(crate) const ARTIFACT_PATH: &str = "artifact_path";
    pub(crate) const BACKENDS: &str = "backends";
    pub(crate) const BELIEFS: &str = "beliefs";
    pub(crate) const CAPABILITIES: &str = super::fields::CAPABILITIES;
    pub(crate) const CATEGORY: &str = "category";
    pub(crate) const COMPILE_MS: &str = "compile_ms";
    pub(crate) const CRITICAL_PATH: &str = "critical_path";
    pub(crate) const DEPENDENCY_TYPES: &str = "dependency_types";
    pub(crate) const DEPTH: &str = "depth";
    pub(crate) const DESCRIPTION: &str = "description";
    pub(crate) const DURATION_MS: &str = "duration_ms";
    pub(crate) const EDGE_COUNT: &str = "edge_count";
    pub(crate) const EDGES: &str = "edges";
    pub(crate) const EMISSION_MS: &str = "emission_ms";
    pub(crate) const ENTRY_NODES: &str = "entry_nodes";
    pub(crate) const ERRORS: &str = "errors";
    pub(crate) const EPISODES: &str = "episodes";
    pub(crate) const ESTIMATED_MS: &str = "estimated_ms";
    pub(crate) const ESTIMATED_SPEEDUP: &str = "estimated_speedup";
    pub(crate) const EVIDENCE: &str = "evidence";
    pub(crate) const EXAMPLE: &str = "example";
    pub(crate) const EXECUTE_MS: &str = "execute_ms";
    pub(crate) const EXECUTED_NODES: &str = "executed_nodes";
    pub(crate) const EXECUTION: &str = "execution";
    pub(crate) const EXECUTION_ID: &str = "execution_id";
    pub(crate) const EXECUTION_PHASES: &str = "execution_phases";
    pub(crate) const EXIT_NODES: &str = "exit_nodes";
    pub(crate) const FAILED_NODES: &str = "failed_nodes";
    pub(crate) const FILES: &str = "files";
    pub(crate) const GOALS: &str = "goals";
    pub(crate) const GOAL_CHANGES: &str = "goal_changes";
    pub(crate) const HEALTH: &str = "health";
    pub(crate) const ID: &str = "id";
    pub(crate) const INPUT_TOKENS: &str = "input_tokens";
    pub(crate) const KEY: &str = "key";
    pub(crate) const LABEL: &str = "label";
    pub(crate) const LATENCY: &str = "latency";
    pub(crate) const LATENCY_MS: &str = "latency_ms";
    pub(crate) const LENGTH: &str = "length";
    pub(crate) const LLM_USAGE: &str = "llm_usage";
    pub(crate) const LONG_DESCRIPTION: &str = "long_description";
    pub(crate) const MANIFEST: &str = "manifest";
    pub(crate) const MATCHES: &str = "matches";
    pub(crate) const MAX_PARALLELISM: &str = "max_parallelism";
    pub(crate) const MEMORY: &str = "memory";
    pub(crate) const METRICS: &str = "metrics";
    pub(crate) const MODEL: &str = "model";
    pub(crate) const NAME: &str = "name";
    pub(crate) const NODE_COUNT: &str = "node_count";
    pub(crate) const NODE_ID: &str = super::args::NODE_ID;
    pub(crate) const NODE_METRICS: &str = "node_metrics";
    pub(crate) const NODE_OUTPUTS: &str = "node_outputs";
    pub(crate) const NODES: &str = "nodes";
    pub(crate) const OP: &str = "op";
    pub(crate) const OPERATIONS: &str = "operations";
    pub(crate) const OPT_LEVEL: &str = super::args::OPT_LEVEL;
    pub(crate) const OPTIONAL_ATTRIBUTES: &str = "optional_attributes";
    pub(crate) const OUTPUT_TOKENS: &str = "output_tokens";
    pub(crate) const OUTPUTS: &str = "outputs";
    pub(crate) const PARALLEL: &str = "parallel";
    pub(crate) const PARALLEL_MS: &str = "parallel_ms";
    pub(crate) const PARALLELISM_DEGREE: &str = "parallelism_degree";
    pub(crate) const PARAMETER_TYPES: &str = "parameter_types";
    pub(crate) const WORKFLOW: &str = "workflow";
    pub(crate) const WORKFLOW_NAME: &str = "workflow_name";
    pub(crate) const PHASE: &str = "phase";
    pub(crate) const PREVIEW: &str = "preview";
    pub(crate) const PRODUCES_OUTPUT: &str = "produces_output";
    pub(crate) const QUERY: &str = super::args::QUERY;
    pub(crate) const RECORD: &str = "record";
    pub(crate) const REQUIRED_ARGUMENT: &str = "required_argument";
    pub(crate) const REQUIRED_ATTRIBUTES: &str = "required_attributes";
    pub(crate) const RESULT: &str = "result";
    pub(crate) const RESULTS: &str = "results";
    pub(crate) const SCORE: &str = "score";
    pub(crate) const SEQUENTIAL_MS: &str = "sequential_ms";
    pub(crate) const SESSION_DIR: &str = "session_dir";
    pub(crate) const SESSION_ID: &str = super::args::SESSION_ID;
    pub(crate) const SKILL_ID: &str = "skill_id";
    pub(crate) const SKILL_VERSION: &str = "skill_version";
    pub(crate) const SPEEDUP: &str = "speedup";
    pub(crate) const STATS: &str = "stats";
    pub(crate) const STATUS: &str = "status";
    pub(crate) const SUGGESTIONS: &str = "suggestions";
    pub(crate) const SUMMARY: &str = "summary";
    pub(crate) const TIMESTAMP: &str = "timestamp";
    pub(crate) const TOTAL_REQUESTS: &str = "total_requests";
    pub(crate) const TRACE: &str = "trace";
    pub(crate) const TRACE_ID: &str = super::args::TRACE_ID;
    pub(crate) const TRANSITIONS: &str = "transitions";
    pub(crate) const VALID: &str = "valid";
    pub(crate) const VALUE: &str = "value";
    pub(crate) const WARNINGS: &str = "warnings";
    pub(crate) const BELIEF_CHANGES: &str = "belief_changes";
    pub(crate) const CAPABILITY_CHANGES: &str = "capability_changes";
}

pub(crate) mod status {
    pub(crate) const COMPILED: &str = "compiled";
    pub(crate) const EXECUTED: &str = "executed";
    pub(crate) const FOUND: &str = "found";
    pub(crate) const NOT_FOUND: &str = "not_found";
    pub(crate) const UNAVAILABLE: &str = "unavailable";
}

pub(crate) mod workflow_skill {
    pub(crate) const ID: &str = "prompt-as-workflow";
    pub(crate) const VERSION: &str = "0.1.0";
    pub(crate) const ENTRY_FLOW: &str = "prompt_as_workflow";
    pub(crate) const TRACE_PREFIX: &str = "apxm-workflow";
    pub(crate) const EMISSION_CAPABILITY: &str = "workflow_emission_v1";
    pub(crate) const REQUEST_CAPABILITY_KEY: &str = "capability";
    pub(crate) const SESSION_DIR_KIND: &str = "skills";
}

pub(crate) mod evidence_path {
    pub(crate) const DOCS: &str = "docs";
    pub(crate) const CLAIMS: &str = "claims";
    pub(crate) const EVALUATION: &str = "evaluation";
    pub(crate) const BENCHMARKS: &str = "benchmarks";
    pub(crate) const RESULTS: &str = "results";
    pub(crate) const FILE_EXTENSIONS: &[&str] = &["md", "json", "jsonl", "csv", "txt"];
}

pub(crate) mod defaults {
    pub(crate) const ARTIFACT_WORKFLOW_NAME: &str = "artifact";
}

pub(crate) mod admission_error {
    use apxm_core::constants::mcp::tools as mcp_tool_names;

    pub(crate) const GENERATED_PYTHON_TOOL_SECTIONS: &str =
        "generated workflow execution does not support python tool sections";
    pub(crate) const GENERATED_PYTHON_TOOL_HANDLERS: &str =
        "generated workflow execution does not support python-backed tool handlers";
    pub(crate) const INV_TOOL_MISSING_CAPABILITY: &str = "INV_TOOL missing capability attribute";
    pub(crate) const INV_TOOL_PARAMS_NOT_OBJECT: &str =
        "INV_TOOL params_json must be a JSON object";
    pub(crate) const ASK_REQUIRES_READ_ONLY_TOOLS: &str =
        "ASK/THINK/REASON tool exposure requires read-only tools";
    pub(crate) const TRACE_ID_UNSAFE: &str = "trace_id must contain only ASCII letters, digits, '-', '_', or '.', and must not be '.' or '..'";

    pub(crate) fn generated_capability_not_registered(capability: &str) -> String {
        format!("generated workflow capability '{capability}' is not registered")
    }

    pub(crate) fn generated_capability_direct_side_effect(capability: &str) -> String {
        format!(
            "generated workflow capability '{capability}' is not read-only and does not declare sandbox execution"
        )
    }

    pub(crate) fn generated_capability_sandbox_preflight(capability: &str, error: &str) -> String {
        format!("generated workflow capability '{capability}' failed sandbox preflight: {error}")
    }

    pub(crate) fn ask_group_capability_not_read_only(capability: &str) -> String {
        format!("{ASK_REQUIRES_READ_ONLY_TOOLS}; capability '{capability}' is not read-only")
    }

    pub(crate) fn ask_all_tools_exposes_non_read_only(capability: &str) -> String {
        format!("ASK tools_enabled=true would expose non-read-only capability '{capability}'")
    }

    pub(crate) fn ask_named_tool_not_read_only(tool_name: &str) -> String {
        format!("{ASK_REQUIRES_READ_ONLY_TOOLS}; capability '{tool_name}' is not read-only")
    }

    pub(crate) fn generated_process_spawn_not_allowed(op_type: impl std::fmt::Display) -> String {
        format!(
            "generated workflows may not contain process-spawn operation {op_type}; use {} or dekk apxm goal for admitted worker goal execution",
            mcp_tool_names::APXM_GOAL_START
        )
    }
}

pub(crate) mod schema_type {
    pub(crate) const ARRAY: &str = "array";
    pub(crate) const BOOLEAN: &str = "boolean";
    pub(crate) const INTEGER: &str = "integer";
    pub(crate) const OBJECT: &str = "object";
    pub(crate) const STRING: &str = "string";
}

pub(crate) mod server_name {
    pub(crate) const HTTP: &str = "apxm-server";
    pub(crate) const STDIO: &str = "apxm-mcp-server";
}

pub(crate) mod contract_value {
    pub(crate) const DEPENDENCY_TYPES: [&str; 3] = ["Data", "Control", "Effect"];
    pub(crate) const PARAMETER_TYPES: &[&str] = apxm_core::constants::parameters::VALID_TYPES;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentKind {
    Text,
}

impl ContentKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpMethod {
    Initialize,
    ToolsList,
    ToolsCall,
    ResourcesList,
    ResourcesRead,
    Ping,
}

impl McpMethod {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Initialize => apxm_core::constants::mcp::methods::INITIALIZE,
            Self::ToolsList => apxm_core::constants::mcp::methods::TOOLS_LIST,
            Self::ToolsCall => apxm_core::constants::mcp::methods::TOOLS_CALL,
            Self::ResourcesList => apxm_core::constants::mcp::methods::RESOURCES_LIST,
            Self::ResourcesRead => apxm_core::constants::mcp::methods::RESOURCES_READ,
            Self::Ping => apxm_core::constants::mcp::methods::PING,
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            value if value == Self::Initialize.as_str() => Some(Self::Initialize),
            value if value == Self::ToolsList.as_str() => Some(Self::ToolsList),
            value if value == Self::ToolsCall.as_str() => Some(Self::ToolsCall),
            value if value == Self::ResourcesList.as_str() => Some(Self::ResourcesList),
            value if value == Self::ResourcesRead.as_str() => Some(Self::ResourcesRead),
            value if value == Self::Ping.as_str() => Some(Self::Ping),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StdioTool {
    Validate,
    Compile,
    Execute,
    GetContract,
    Analyze,
}

impl StdioTool {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Compile => "compile",
            Self::Execute => "execute",
            Self::GetContract => "get_contract",
            Self::Analyze => "analyze",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            value if value == Self::Validate.as_str() => Some(Self::Validate),
            value if value == Self::Compile.as_str() => Some(Self::Compile),
            value if value == Self::Execute.as_str() => Some(Self::Execute),
            value if value == Self::GetContract.as_str() => Some(Self::GetContract),
            value if value == Self::Analyze.as_str() => Some(Self::Analyze),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tier3Tool {
    PromptAsWorkflow,
    TraceFetch,
    AamRecall,
    EvidenceLookup,
    CapabilityList,
}

impl Tier3Tool {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PromptAsWorkflow => "prompt_as_workflow",
            Self::TraceFetch => "trace_fetch",
            Self::AamRecall => "aam_recall",
            Self::EvidenceLookup => "evidence_lookup",
            Self::CapabilityList => "capability_list",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            value if value == Self::PromptAsWorkflow.as_str() => Some(Self::PromptAsWorkflow),
            value if value == Self::TraceFetch.as_str() => Some(Self::TraceFetch),
            value if value == Self::AamRecall.as_str() => Some(Self::AamRecall),
            value if value == Self::EvidenceLookup.as_str() => Some(Self::EvidenceLookup),
            value if value == Self::CapabilityList.as_str() => Some(Self::CapabilityList),
            _ => None,
        }
    }
}

pub(crate) mod tool_description {
    use super::Tier3Tool;

    pub(crate) const PROMPT_AS_WORKFLOW: &str = "Emit canonical APXM AIR via the model router, repair it with compiler feedback, compile it, optionally execute it, and return a compact summary with trace_id.";
    pub(crate) const TRACE_FETCH: &str =
        "Fetch a compact execution trace summary by trace_id; pass full=true for detailed records.";
    pub(crate) const AAM_RECALL: &str =
        "Recall matching AAM beliefs, goals, transitions, and memory entries.";
    pub(crate) const EVIDENCE_LOOKUP: &str =
        "Lookup APXM evaluation and claim evidence from repo-local .apxm evidence stores.";
    pub(crate) const CAPABILITY_LIST: &str =
        "List APXM runtime capabilities, LLM backends, and model-router health.";

    pub(crate) const fn tier3(tool: Tier3Tool) -> &'static str {
        match tool {
            Tier3Tool::PromptAsWorkflow => PROMPT_AS_WORKFLOW,
            Tier3Tool::TraceFetch => TRACE_FETCH,
            Tier3Tool::AamRecall => AAM_RECALL,
            Tier3Tool::EvidenceLookup => EVIDENCE_LOOKUP,
            Tier3Tool::CapabilityList => CAPABILITY_LIST,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkillTool {
    List,
    Get,
    Validate,
    Call,
}

impl SkillTool {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::List => "skills_list",
            Self::Get => "skill_get",
            Self::Validate => "skill_validate",
            Self::Call => "skill_call",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationCategoryWire {
    Communication,
    ControlFlow,
    Coordination,
    ErrorHandling,
    Identity,
    Internal,
    Memory,
    Metadata,
    Reasoning,
    Synchronization,
    Tools,
}

impl OperationCategoryWire {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Communication => "communication",
            Self::ControlFlow => "control_flow",
            Self::Coordination => "coordination",
            Self::ErrorHandling => "error_handling",
            Self::Identity => "identity",
            Self::Internal => "internal",
            Self::Memory => "memory",
            Self::Metadata => "metadata",
            Self::Reasoning => "reasoning",
            Self::Synchronization => "synchronization",
            Self::Tools => "tools",
        }
    }
}

impl From<OperationCategory> for OperationCategoryWire {
    fn from(category: OperationCategory) -> Self {
        match category {
            OperationCategory::Communication => Self::Communication,
            OperationCategory::ControlFlow => Self::ControlFlow,
            OperationCategory::Coordination => Self::Coordination,
            OperationCategory::ErrorHandling => Self::ErrorHandling,
            OperationCategory::Identity => Self::Identity,
            OperationCategory::Internal => Self::Internal,
            OperationCategory::Memory => Self::Memory,
            OperationCategory::Metadata => Self::Metadata,
            OperationCategory::Reasoning => Self::Reasoning,
            OperationCategory::Synchronization => Self::Synchronization,
            OperationCategory::Tools => Self::Tools,
        }
    }
}

pub(crate) const fn operation_latency_estimate_ms(latency: OperationLatency) -> u64 {
    match latency {
        OperationLatency::None => 10,
        OperationLatency::Low => 100,
        OperationLatency::Medium => 1000,
        OperationLatency::High => 5000,
    }
}
