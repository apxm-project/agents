use apxm_backends::llm::ProviderProtocol;
use apxm_backends::llm::catalog::{BUILTIN_MODELS, BUILTIN_PROVIDERS};
use apxm_core::constants;
use apxm_core::types::operations::{
    AISOperationType, OperationCategory, OperationField, get_all_operations,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendConstant {
    pub name: String,
    pub value: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendFieldSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
    pub ref_type: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendOperationSpec {
    pub op: AISOperationType,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub long_description: &'static str,
    pub latency: &'static str,
    pub fields: Vec<FrontendFieldSpec>,
    pub produces_output: bool,
    pub needs_submission: bool,
    pub min_inputs: u32,
    pub example_json: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendAgentTemplate {
    pub name: String,
    pub command: String,
    pub description: Option<String>,
    pub route_capabilities: Vec<String>,
    pub source: String,
    pub default_mode: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendProviderSpec {
    pub id: &'static str,
    pub protocol: &'static str,
    pub default_base_url: Option<&'static str>,
    pub requires_api_key: bool,
    pub api_key_env_var: Option<&'static str>,
    pub aliases: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendModelSpec {
    pub id: &'static str,
    pub provider: &'static str,
    pub is_default: bool,
}

pub fn builtin_models() -> Vec<FrontendModelSpec> {
    BUILTIN_MODELS
        .iter()
        .map(|m| FrontendModelSpec {
            id: m.id,
            provider: m.protocol.as_str(),
            is_default: m.is_default,
        })
        .collect()
}

pub fn graph_metadata_constants() -> Vec<FrontendConstant> {
    vec![
        FrontendConstant {
            name: "IS_ENTRY".to_string(),
            value: constants::graph::metadata::IS_ENTRY,
        },
        FrontendConstant {
            name: "AIR_PAYLOAD".to_string(),
            value: constants::inner_plan::AIR_PAYLOAD,
        },
    ]
}

/// Derives attribute constants directly from `ALL_ATTR_NAMES` — no hand-maintained list.
pub fn graph_attr_constants() -> Vec<FrontendConstant> {
    constants::graph::attrs::ALL_ATTR_NAMES
        .iter()
        .map(|&value| FrontendConstant {
            name: to_const_name(value),
            value,
        })
        .collect()
}

pub fn graph_metric_constants() -> Vec<FrontendConstant> {
    use constants::session::metrics_keys;
    use metrics_keys::graph_metric_keys;

    vec![
        FrontendConstant {
            name: "GRAPH_METRICS".to_string(),
            value: metrics_keys::RUNTIME_GRAPH_METRICS,
        },
        FrontendConstant {
            name: "GRAPH_METRIC_GRAPH".to_string(),
            value: graph_metric_keys::GRAPH,
        },
        FrontendConstant {
            name: "GRAPH_METRIC_NODES".to_string(),
            value: graph_metric_keys::NODES,
        },
        FrontendConstant {
            name: "GRAPH_METRIC_AGGREGATES".to_string(),
            value: graph_metric_keys::AGGREGATES,
        },
        FrontendConstant {
            name: "GRAPH_METRIC_BY_AGENT".to_string(),
            value: graph_metric_keys::BY_AGENT,
        },
        FrontendConstant {
            name: "GRAPH_METRIC_PROCESS_SPAWNS".to_string(),
            value: graph_metric_keys::PROCESS_SPAWNS,
        },
        FrontendConstant {
            name: "GRAPH_METRIC_PROMPT_TURNS".to_string(),
            value: graph_metric_keys::PROMPT_TURNS,
        },
    ]
}

fn to_const_name(s: &str) -> String {
    s.replace('.', "_").to_ascii_uppercase()
}

pub fn operation_specs() -> Vec<FrontendOperationSpec> {
    get_all_operations()
        .map(|spec| FrontendOperationSpec {
            op: spec.op_type,
            name: spec.name,
            category: category_label(spec.category),
            description: spec.description,
            long_description: spec.long_description,
            latency: spec.latency.as_str(),
            fields: spec.fields.iter().map(field_to_frontend).collect(),
            produces_output: spec.produces_output,
            needs_submission: spec.needs_submission,
            min_inputs: spec.min_inputs,
            example_json: spec.example_json,
        })
        .collect()
}

fn field_to_frontend(field: &OperationField) -> FrontendFieldSpec {
    FrontendFieldSpec {
        name: field.name,
        description: field.description,
        required: field.required,
        ref_type: field.ref_type.map(|r| r.label()),
    }
}

pub fn builtin_providers() -> Vec<FrontendProviderSpec> {
    BUILTIN_PROVIDERS
        .iter()
        .map(|p| FrontendProviderSpec {
            id: p.id,
            protocol: p.protocol.as_str(),
            default_base_url: p.default_base_url,
            requires_api_key: p.requires_api_key,
            api_key_env_var: p.api_key_env_var,
            aliases: p.aliases,
        })
        .collect()
}

pub fn provider_protocols() -> Vec<&'static str> {
    ProviderProtocol::all_variants()
        .iter()
        .map(|p| p.as_str())
        .collect()
}

pub fn valid_param_types() -> &'static [&'static str] {
    constants::parameters::VALID_TYPES
}

pub fn agent_templates() -> Vec<FrontendAgentTemplate> {
    Vec::new()
}

fn category_label(category: OperationCategory) -> &'static str {
    crate::commands::implementations::category_str(category)
}
