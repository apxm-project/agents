use apxm_acp::AgentRegistry;
use apxm_ais::{OperationCategory, OperationField, get_all_operations};
use apxm_core::constants;
use apxm_core::types::model_spec::BUILTIN_MODELS;
use apxm_core::types::provider_spec::{BUILTIN_PROVIDERS, ProviderProtocol};

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
    pub op: String,
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

impl FrontendOperationSpec {
    pub fn required_fields(&self) -> impl Iterator<Item = &FrontendFieldSpec> {
        self.fields.iter().filter(|f| f.required)
    }

    pub fn optional_fields(&self) -> impl Iterator<Item = &FrontendFieldSpec> {
        self.fields.iter().filter(|f| !f.required)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendAgentTemplate {
    pub name: String,
    pub command: String,
    pub default_mode: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendEmissionSpec {
    pub op: String,
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
            provider: m.provider,
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
            name: "GRAPH_PAYLOAD".to_string(),
            value: constants::inner_plan::GRAPH_PAYLOAD,
        },
    ]
}

/// Derives attribute constants directly from `ALL_ATTR_NAMES` — no hand-maintained list.
pub fn graph_attr_constants() -> Vec<FrontendConstant> {
    apxm_ais::attrs::ALL_ATTR_NAMES
        .iter()
        .map(|&value| FrontendConstant {
            name: to_const_name(value),
            value,
        })
        .collect()
}

fn to_const_name(s: &str) -> String {
    s.replace('.', "_").to_ascii_uppercase()
}

pub fn operation_specs() -> Vec<FrontendOperationSpec> {
    get_all_operations()
        .map(|spec| FrontendOperationSpec {
            op: spec.op_type.to_string(),
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
            default_mode: profile.default_mode,
            default_model: profile.default_model,
        })
        .collect()
}

pub fn emission_specs() -> Vec<FrontendEmissionSpec> {
    get_all_operations()
        .map(|spec| {
            let context_style = match spec.emission.context_style {
                apxm_ais::ContextStyle::Bracketed => "Bracketed",
                apxm_ais::ContextStyle::Parenthesized => "Parenthesized",
                apxm_ais::ContextStyle::Direct => "Direct",
                apxm_ais::ContextStyle::None => "None",
            };
            let result_type = match spec.emission.result_type {
                apxm_ais::MlirResultType::Token => "Token",
                apxm_ais::MlirResultType::Handle => "Handle",
                apxm_ais::MlirResultType::Void => "Void",
            };
            FrontendEmissionSpec {
                op: spec.op_type.to_string(),
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
    fn graph_attr_constants_derived_from_all_attr_names() {
        let constants = graph_attr_constants();
        assert_eq!(constants.len(), apxm_ais::attrs::ALL_ATTR_NAMES.len());
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
    fn operation_specs_have_full_projection() {
        let ask = operation_specs()
            .into_iter()
            .find(|s| s.op == "ASK")
            .unwrap();
        assert!(!ask.long_description.is_empty());
        assert_eq!(ask.latency, "low");
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
            .find(|m| m.provider == "openai" && m.is_default)
            .map(|m| m.id.to_string());
        assert_eq!(default, Some("gpt-4o-mini".to_string()));
    }

    #[test]
    fn valid_param_types_includes_all() {
        let types = valid_param_types();
        assert_eq!(types, &["str", "int", "float", "bool", "json"]);
    }
}
