use apxm_backends::llm::ProviderProtocol;
use apxm_backends::llm::catalog::BUILTIN_PROVIDERS;
use apxm_core::constants;
use apxm_core::types::operations::{
    OperationCategory, OperationField, SemanticOpKind as AISOperationType, get_all_operations,
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
pub struct FrontendProviderSpec {
    pub id: &'static str,
    pub protocol: &'static str,
    pub default_base_url: Option<&'static str>,
    pub requires_api_key: bool,
    pub api_key_env_var: Option<&'static str>,
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
            name: "GRAPH_METRIC_PROMPTS".to_string(),
            value: graph_metric_keys::PROMPTS,
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
        })
        .collect()
}

pub fn provider_protocols() -> Vec<&'static str> {
    ProviderProtocol::all_variants()
        .iter()
        .map(|p| p.as_str())
        .collect()
}

fn category_label(category: OperationCategory) -> &'static str {
    crate::commands::implementations::category_str(category)
}
