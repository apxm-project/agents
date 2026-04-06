use apxm_acp::AgentRegistry;
use apxm_ais::{OperationCategory, get_all_operations};
use apxm_core::constants;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendConstant {
    pub name: &'static str,
    pub value: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendOperationSpec {
    pub op: String,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub required_fields: Vec<&'static str>,
    pub optional_fields: Vec<&'static str>,
    pub produces_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendAgentTemplate {
    pub name: String,
    pub command: String,
    pub default_mode: Option<String>,
    pub default_model: Option<String>,
}

pub fn graph_metadata_constants() -> Vec<FrontendConstant> {
    vec![
        FrontendConstant {
            name: "IS_ENTRY",
            value: constants::graph::metadata::IS_ENTRY,
        },
        FrontendConstant {
            name: "GRAPH_PAYLOAD",
            value: constants::inner_plan::GRAPH_PAYLOAD,
        },
    ]
}

pub fn graph_attr_constants() -> Vec<FrontendConstant> {
    vec![
        FrontendConstant {
            name: "AGENT_NAME",
            value: constants::graph::attrs::AGENT_NAME,
        },
        FrontendConstant {
            name: "TEAM_NAME",
            value: constants::graph::attrs::TEAM_NAME,
        },
        FrontendConstant {
            name: "FLOW_NAME",
            value: constants::graph::attrs::FLOW_NAME,
        },
        FrontendConstant {
            name: "MODEL",
            value: constants::graph::attrs::MODEL,
        },
        FrontendConstant {
            name: "PROVIDER",
            value: constants::graph::attrs::PROVIDER,
        },
        FrontendConstant {
            name: "API_KEY",
            value: constants::graph::attrs::API_KEY,
        },
        FrontendConstant {
            name: "BASE_URL",
            value: constants::graph::attrs::BASE_URL,
        },
        FrontendConstant {
            name: "TEMPERATURE",
            value: constants::graph::attrs::TEMPERATURE,
        },
        FrontendConstant {
            name: "SYSTEM_PROMPT",
            value: constants::graph::attrs::SYSTEM_PROMPT,
        },
        FrontendConstant {
            name: "TOOLS_CONFIG",
            value: constants::graph::attrs::TOOLS_CONFIG,
        },
        FrontendConstant {
            name: "TOKEN_BUDGET",
            value: constants::graph::attrs::TOKEN_BUDGET,
        },
        FrontendConstant {
            name: "OUTPUT_SCHEMA",
            value: constants::graph::attrs::OUTPUT_SCHEMA,
        },
        FrontendConstant {
            name: "MAX_SCHEMA_RETRIES",
            value: constants::graph::attrs::MAX_SCHEMA_RETRIES,
        },
        FrontendConstant {
            name: "MAX_ITERATIONS",
            value: constants::graph::attrs::MAX_ITERATIONS,
        },
        FrontendConstant {
            name: "HANDOFF_TARGETS",
            value: constants::graph::attrs::HANDOFF_TARGETS,
        },
        FrontendConstant {
            name: "INNER_PLAN_SUPPORTED",
            value: constants::graph::attrs::INNER_PLAN_SUPPORTED,
        },
        FrontendConstant {
            name: "ENABLE_INNER_PLAN",
            value: constants::graph::attrs::ENABLE_INNER_PLAN,
        },
        FrontendConstant {
            name: "BIND_INNER_PLAN_OUTPUTS",
            value: constants::graph::attrs::BIND_INNER_PLAN_OUTPUTS,
        },
        FrontendConstant {
            name: "TEMPLATE_STR",
            value: constants::graph::attrs::TEMPLATE_STR,
        },
        FrontendConstant {
            name: "PROMPT",
            value: constants::graph::attrs::PROMPT,
        },
        FrontendConstant {
            name: "TEMPLATE",
            value: constants::graph::attrs::TEMPLATE,
        },
        FrontendConstant {
            name: "QUERY",
            value: constants::graph::attrs::QUERY,
        },
        FrontendConstant {
            name: "MEMORY_TIER",
            value: constants::graph::attrs::MEMORY_TIER,
        },
        FrontendConstant {
            name: "SPACE",
            value: constants::graph::attrs::SPACE,
        },
        FrontendConstant {
            name: "CAPABILITY",
            value: constants::graph::attrs::CAPABILITY,
        },
        FrontendConstant {
            name: "PARAMS_JSON",
            value: constants::graph::attrs::PARAMS_JSON,
        },
        FrontendConstant {
            name: "GOAL",
            value: constants::graph::attrs::GOAL,
        },
        FrontendConstant {
            name: "TRACE_ID",
            value: constants::graph::attrs::TRACE_ID,
        },
        FrontendConstant {
            name: "TRACE",
            value: constants::graph::attrs::TRACE,
        },
        FrontendConstant {
            name: "TRACE_QUERY",
            value: constants::graph::attrs::TRACE_QUERY,
        },
        FrontendConstant {
            name: "TRUE_LABEL",
            value: constants::graph::attrs::TRUE_LABEL,
        },
        FrontendConstant {
            name: "FALSE_LABEL",
            value: constants::graph::attrs::FALSE_LABEL,
        },
        FrontendConstant {
            name: "CASE_LABELS",
            value: constants::graph::attrs::CASE_LABELS,
        },
        FrontendConstant {
            name: "LABEL",
            value: constants::graph::attrs::LABEL,
        },
        FrontendConstant {
            name: "TRY_LABEL",
            value: constants::graph::attrs::TRY_LABEL,
        },
        FrontendConstant {
            name: "CATCH_LABEL",
            value: constants::graph::attrs::CATCH_LABEL,
        },
        FrontendConstant {
            name: "RECOVERY_TEMPLATE",
            value: constants::graph::attrs::RECOVERY_TEMPLATE,
        },
        FrontendConstant {
            name: "TOOLS_ENABLED",
            value: constants::graph::attrs::TOOLS_ENABLED,
        },
        FrontendConstant {
            name: "TOOLS",
            value: constants::graph::attrs::TOOLS,
        },
        FrontendConstant {
            name: "MESSAGE",
            value: constants::graph::attrs::MESSAGE,
        },
        FrontendConstant {
            name: "RECIPIENT",
            value: constants::graph::attrs::RECIPIENT,
        },
        FrontendConstant {
            name: "TARGET",
            value: constants::graph::attrs::TARGET,
        },
        FrontendConstant {
            name: "PROTOCOL",
            value: constants::graph::attrs::PROTOCOL,
        },
        FrontendConstant {
            name: "CONDITION",
            value: constants::graph::attrs::CONDITION,
        },
        FrontendConstant {
            name: "EVIDENCE",
            value: constants::graph::attrs::EVIDENCE,
        },
        FrontendConstant {
            name: "VALUE",
            value: constants::graph::attrs::VALUE,
        },
        FrontendConstant {
            name: "KEY",
            value: constants::graph::attrs::KEY,
        },
        FrontendConstant {
            name: "QUEUE",
            value: constants::graph::attrs::QUEUE,
        },
        FrontendConstant {
            name: "CHECKPOINT",
            value: constants::graph::attrs::CHECKPOINT,
        },
        FrontendConstant {
            name: "CHECKPOINT_ID",
            value: constants::graph::attrs::CHECKPOINT_ID,
        },
        FrontendConstant {
            name: "SERVER_URL",
            value: constants::graph::attrs::SERVER_URL,
        },
        FrontendConstant {
            name: "ACTION",
            value: constants::graph::attrs::ACTION,
        },
        FrontendConstant {
            name: "GOAL_ID",
            value: constants::graph::attrs::GOAL_ID,
        },
        FrontendConstant {
            name: "PRIORITY",
            value: constants::graph::attrs::PRIORITY,
        },
        FrontendConstant {
            name: "ON_FAIL",
            value: constants::graph::attrs::ON_FAIL,
        },
        FrontendConstant {
            name: "ERROR_MESSAGE",
            value: constants::graph::attrs::ERROR_MESSAGE,
        },
        FrontendConstant {
            name: "STRATEGY",
            value: constants::graph::attrs::STRATEGY,
        },
        FrontendConstant {
            name: "SEPARATOR",
            value: constants::graph::attrs::SEPARATOR,
        },
        FrontendConstant {
            name: "TIMEOUT_MS",
            value: constants::graph::attrs::TIMEOUT_MS,
        },
        FrontendConstant {
            name: "BUDGET",
            value: constants::graph::attrs::BUDGET,
        },
        FrontendConstant {
            name: "MAX_RETRIES",
            value: constants::graph::attrs::MAX_RETRIES,
        },
        FrontendConstant {
            name: "HISTORY_LIMIT",
            value: constants::graph::attrs::HISTORY_LIMIT,
        },
        FrontendConstant {
            name: "LIMIT",
            value: constants::graph::attrs::LIMIT,
        },
        FrontendConstant {
            name: "STAGING_ID",
            value: constants::graph::attrs::STAGING_ID,
        },
        FrontendConstant {
            name: "CONTEXT_KEY",
            value: constants::graph::attrs::CONTEXT_KEY,
        },
        FrontendConstant {
            name: "LEASE_MS",
            value: constants::graph::attrs::LEASE_MS,
        },
        FrontendConstant {
            name: "MAX_WAIT_MS",
            value: constants::graph::attrs::MAX_WAIT_MS,
        },
        FrontendConstant {
            name: "NOTIFICATION_URL",
            value: constants::graph::attrs::NOTIFICATION_URL,
        },
        FrontendConstant {
            name: "POLL_INTERVAL_MS",
            value: constants::graph::attrs::POLL_INTERVAL_MS,
        },
        FrontendConstant {
            name: "POLL_MAX_ATTEMPTS",
            value: constants::graph::attrs::POLL_MAX_ATTEMPTS,
        },
        FrontendConstant {
            name: "CASE_REGIONS",
            value: constants::graph::attrs::CASE_REGIONS,
        },
        FrontendConstant {
            name: "DEFAULT_REGION",
            value: constants::graph::attrs::DEFAULT_REGION,
        },
        FrontendConstant {
            name: "BACKEND",
            value: constants::graph::attrs::BACKEND,
        },
        FrontendConstant {
            name: "CLAIM_TEXT",
            value: constants::graph::attrs::CLAIM_TEXT,
        },
        FrontendConstant {
            name: "CODE",
            value: constants::graph::attrs::CODE,
        },
        FrontendConstant {
            name: "COUNT",
            value: constants::graph::attrs::COUNT,
        },
        FrontendConstant {
            name: "INTERPRETER",
            value: constants::graph::attrs::INTERPRETER,
        },
        FrontendConstant {
            name: "HANDOFF_FROM",
            value: constants::graph::attrs::HANDOFF_FROM,
        },
        FrontendConstant {
            name: "HANDOFF_TO",
            value: constants::graph::attrs::HANDOFF_TO,
        },
        FrontendConstant {
            name: "MAX_TOOL_ITERATIONS",
            value: constants::graph::attrs::MAX_TOOL_ITERATIONS,
        },
        FrontendConstant {
            name: "PROFILE",
            value: constants::graph::attrs::PROFILE,
        },
        FrontendConstant {
            name: "NODE_NAME",
            value: constants::graph::attrs::NODE_NAME,
        },
        FrontendConstant {
            name: "MODE",
            value: constants::graph::attrs::MODE,
        },
        FrontendConstant {
            name: "CWD",
            value: constants::graph::attrs::CWD,
        },
        FrontendConstant {
            name: "TASK_SPEC",
            value: constants::graph::attrs::TASK_SPEC,
        },
        FrontendConstant {
            name: "TARGET_AGENT",
            value: constants::graph::attrs::TARGET_AGENT,
        },
        FrontendConstant {
            name: "PARTIES",
            value: constants::graph::attrs::PARTIES,
        },
        FrontendConstant {
            name: "PROPOSAL",
            value: constants::graph::attrs::PROPOSAL,
        },
        FrontendConstant {
            name: "TIMEOUT",
            value: constants::graph::attrs::TIMEOUT,
        },
        FrontendConstant {
            name: "MAX_ROUNDS",
            value: constants::graph::attrs::MAX_ROUNDS,
        },
        FrontendConstant {
            name: "CAPABILITY_NAME",
            value: constants::graph::attrs::CAPABILITY_NAME,
        },
        FrontendConstant {
            name: "DESCRIPTION",
            value: constants::graph::attrs::DESCRIPTION,
        },
        FrontendConstant {
            name: "REGION",
            value: constants::graph::attrs::REGION,
        },
        FrontendConstant {
            name: "PARAMETERS_SCHEMA",
            value: constants::graph::attrs::PARAMETERS_SCHEMA,
        },
        FrontendConstant {
            name: "CACHED_SYSTEM_PROMPT",
            value: constants::graph::attrs::CACHED_SYSTEM_PROMPT,
        },
        FrontendConstant {
            name: "MEMOIZABLE",
            value: constants::graph::attrs::MEMOIZABLE,
        },
    ]
}

pub fn operation_specs() -> Vec<FrontendOperationSpec> {
    get_all_operations()
        .map(|spec| FrontendOperationSpec {
            op: spec.op_type.to_string(),
            name: spec.name,
            category: category_label(spec.category),
            description: spec.description,
            required_fields: spec.required_fields().map(|field| field.name).collect(),
            optional_fields: spec
                .fields
                .iter()
                .filter(|field| !field.required)
                .map(|field| field.name)
                .collect(),
            produces_output: spec.produces_output,
        })
        .collect()
}

pub fn agent_templates() -> Vec<FrontendAgentTemplate> {
    AgentRegistry::builtin_templates()
        .into_iter()
        .map(|(name, profile)| FrontendAgentTemplate {
            name,
            command: profile.command,
            default_mode: profile.default_mode,
            default_model: profile.default_model,
        })
        .collect()
}

fn category_label(category: OperationCategory) -> &'static str {
    match category {
        OperationCategory::Metadata => "metadata",
        OperationCategory::Memory => "memory",
        OperationCategory::Reasoning => "reasoning",
        OperationCategory::Tools => "tools",
        OperationCategory::ControlFlow => "control_flow",
        OperationCategory::Synchronization => "synchronization",
        OperationCategory::ErrorHandling => "error_handling",
        OperationCategory::Communication => "communication",
        OperationCategory::Coordination => "coordination",
        OperationCategory::Identity => "identity",
        OperationCategory::Internal => "internal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_attr_constants_include_model() {
        assert!(
            graph_attr_constants()
                .iter()
                .any(|item| item.name == "MODEL")
        );
    }

    #[test]
    fn operation_specs_include_spawn_agent() {
        assert!(
            operation_specs()
                .iter()
                .any(|item| item.op == "SPAWN_AGENT")
        );
    }

    #[test]
    fn builtin_agent_templates_include_claude() {
        assert!(agent_templates().iter().any(|item| item.name == "claude"));
    }
}
