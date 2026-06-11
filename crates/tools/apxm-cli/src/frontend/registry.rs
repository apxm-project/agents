use apxm_acp::AgentRegistry;
use apxm_backends::llm::ProviderProtocol;
use apxm_backends::llm::catalog::{BUILTIN_MODELS, BUILTIN_PROVIDERS};
use apxm_core::constants;
use apxm_core::types::operations::{
    AISOperationType, ContextStyle, MlirResultType, OperationCategory, OperationField,
    get_all_operations,
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
pub struct FrontendEmissionSpec {
    pub op: AISOperationType,
    pub mlir_mnemonic: String,
    pub primary_attr: Option<String>,
    pub context_style: String,
    pub result_type: String,
    pub positional_attrs: Vec<String>,
    pub keywords: Vec<String>,
    /// Syntactic-keyword attributes emitted as `<keyword> "<value>"` between
    /// the primary attribute and the operand list. Pairs are
    /// `(literal_keyword, attr_name)`.
    pub syntactic_keywords: Vec<(String, String)>,
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
    AgentRegistry::builtin_templates()
        .into_iter()
        .map(|(name, profile)| FrontendAgentTemplate {
            name,
            command: profile.command,
            description: profile.description,
            route_capabilities: profile.route_capabilities,
            source: "template".to_string(),
            default_mode: profile.default_mode,
            default_model: profile.default_model,
        })
        .collect()
}

pub fn emission_specs() -> Vec<FrontendEmissionSpec> {
    get_all_operations()
        .map(|spec| {
            let context_style = match spec.emission.context_style {
                ContextStyle::Bracketed => "Bracketed",
                ContextStyle::Parenthesized => "Parenthesized",
                ContextStyle::Direct => "Direct",
                ContextStyle::None => "None",
            };
            let result_type = match spec.emission.result_type {
                MlirResultType::Token => "Token",
                MlirResultType::Handle => "Handle",
                MlirResultType::Void => "Void",
            };
            FrontendEmissionSpec {
                op: spec.op_type,
                mlir_mnemonic: spec.op_type.mlir_mnemonic().to_string(),
                primary_attr: spec.emission.primary_attr.map(|s| s.to_string()),
                context_style: context_style.to_string(),
                result_type: result_type.to_string(),
                positional_attrs: spec
                    .emission
                    .positional_attrs
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                keywords: spec
                    .emission
                    .keywords
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                syntactic_keywords: spec
                    .emission
                    .syntactic_keywords
                    .iter()
                    .map(|(kw, attr)| (kw.to_string(), attr.to_string()))
                    .collect(),
            }
        })
        .collect()
}

fn category_label(category: OperationCategory) -> &'static str {
    crate::commands::implementations::category_str(category)
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
    fn graph_attr_constants_derived_from_all_attr_names() {
        let constants = graph_attr_constants();
        assert_eq!(
            constants.len(),
            constants::graph::attrs::ALL_ATTR_NAMES.len()
        );
    }

    #[test]
    fn graph_attr_constant_names_are_unique() {
        let constants = graph_attr_constants();
        let mut names = std::collections::BTreeSet::new();
        let duplicates = constants
            .iter()
            .filter_map(|item| {
                if names.insert(item.name.as_str()) {
                    None
                } else {
                    Some(item.name.as_str())
                }
            })
            .collect::<Vec<_>>();

        assert!(
            duplicates.is_empty(),
            "duplicate frontend attr constants: {duplicates:?}"
        );
    }

    #[test]
    fn operation_specs_include_spawn_agent() {
        assert!(
            operation_specs()
                .iter()
                .any(|item| item.op == AISOperationType::SpawnAgent)
        );
    }

    #[test]
    fn operation_specs_have_full_projection() {
        let ask = operation_specs()
            .into_iter()
            .find(|s| s.op == AISOperationType::Ask)
            .unwrap();
        assert!(!ask.long_description.is_empty());
        assert_eq!(ask.latency, "medium");
        assert!(!ask.fields.is_empty());
    }

    #[test]
    fn builtin_agent_templates_include_claude() {
        assert!(agent_templates().iter().any(|item| item.name == "claude"));
    }

    #[test]
    fn builtin_providers_include_openai() {
        assert!(builtin_providers().iter().any(|p| p.id == "openai"));
    }

    #[test]
    fn provider_protocols_all_present() {
        let protocols = provider_protocols();
        assert!(protocols.contains(&"openai"));
        assert!(protocols.contains(&"mock"));
        assert_eq!(protocols.len(), 6);
    }

    #[test]
    fn builtin_models_include_claude() {
        assert!(builtin_models().iter().any(|m| m.id == "claude-sonnet-4-5"));
    }

    #[test]
    fn builtin_models_include_openai_default() {
        let default = builtin_models()
            .iter()
            .find(|m| m.provider == ProviderProtocol::OpenAI.as_str() && m.is_default)
            .map(|m| m.id.to_string());
        assert_eq!(default, Some("gpt-4o-mini".to_string()));
    }

    #[test]
    fn valid_param_types_includes_all() {
        let types = valid_param_types();
        assert_eq!(types, &["str", "int", "float", "bool", "json"]);
    }
}
