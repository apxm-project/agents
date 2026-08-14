//! Built-in LLM provider and model catalog.
//!
//! This catalog belongs to the backend layer. Core artifacts may carry backend
//! and model identifiers, while endpoint and model selection remain explicit
//! registration concerns.

use super::{ProviderProtocol, ProviderSpec};

/// Static provider spec (const-friendly, no heap allocations).
///
/// A provider is named by exactly one identifier, `id`. There is no alternative
/// spelling for a provider and no alias table to resolve one.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinProviderSpec {
    pub id: &'static str,
    pub api_key_env_var: Option<&'static str>,
    pub default_base_url: Option<&'static str>,
    pub requires_api_key: bool,
    pub protocol: ProviderProtocol,
}

impl BuiltinProviderSpec {
    /// Convert to an owned `ProviderSpec`.
    pub fn to_provider_spec(&self) -> ProviderSpec {
        ProviderSpec {
            id: self.id.to_string(),
            api_key_env_var: self.api_key_env_var.map(str::to_string),
            default_base_url: self.default_base_url.map(str::to_string),
            requires_api_key: self.requires_api_key,
            protocol: self.protocol,
        }
    }
}

/// Built-in provider specifications.
pub const BUILTIN_PROVIDERS: &[BuiltinProviderSpec] = &[
    BuiltinProviderSpec {
        id: "ollama",
        api_key_env_var: Some("OLLAMA_API_KEY"),
        default_base_url: None,
        requires_api_key: false,
        protocol: ProviderProtocol::Ollama,
    },
    BuiltinProviderSpec {
        id: "openai",
        api_key_env_var: Some("OPENAI_API_KEY"),
        default_base_url: None,
        requires_api_key: true,
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinProviderSpec {
        id: "anthropic",
        api_key_env_var: Some("ANTHROPIC_API_KEY"),
        default_base_url: None,
        requires_api_key: true,
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinProviderSpec {
        id: "google",
        api_key_env_var: Some("GOOGLE_API_KEY"),
        default_base_url: None,
        requires_api_key: true,
        protocol: ProviderProtocol::Google,
    },
    BuiltinProviderSpec {
        id: "vllm",
        api_key_env_var: None,
        default_base_url: None,
        requires_api_key: false,
        protocol: ProviderProtocol::Vllm,
    },
    BuiltinProviderSpec {
        id: "llamacpp",
        api_key_env_var: None,
        default_base_url: None,
        requires_api_key: false,
        protocol: ProviderProtocol::LlamaCpp,
    },
    BuiltinProviderSpec {
        id: "openrouter",
        api_key_env_var: Some("OPENROUTER_API_KEY"),
        default_base_url: None,
        requires_api_key: true,
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinProviderSpec {
        id: "mock",
        api_key_env_var: None,
        default_base_url: None,
        requires_api_key: false,
        protocol: ProviderProtocol::Mock,
    },
];

/// Look up a built-in provider by its exact id.
pub fn resolve_builtin_provider(name: &str) -> Option<&'static BuiltinProviderSpec> {
    let lower = name.to_lowercase();
    BUILTIN_PROVIDERS.iter().find(|spec| spec.id == lower)
}

/// Resolve a provider spec from name, checking builtins first.
pub fn resolve_provider_spec(name: &str) -> Option<ProviderSpec> {
    resolve_builtin_provider(name).map(BuiltinProviderSpec::to_provider_spec)
}

/// Static model spec (const-friendly, no heap allocations).
#[derive(Debug, Clone, Copy)]
pub struct BuiltinModelSpec {
    /// Canonical model identifier.
    pub id: &'static str,
    /// Protocol family this model belongs to.
    pub protocol: ProviderProtocol,
}

/// All built-in models, grouped by provider.
pub const BUILTIN_MODELS: &[BuiltinModelSpec] = &[
    BuiltinModelSpec {
        id: "claude-opus-4-6",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "claude-sonnet-4-6",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "claude-haiku-4-5",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "claude-opus-4-5",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "claude-sonnet-4-5",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "claude-3-7-sonnet",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "claude-3-5-sonnet-20241022",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "claude-3-opus-20240229",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "claude-3-sonnet-20240229",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "claude-3-haiku-20240307",
        protocol: ProviderProtocol::Anthropic,
    },
    BuiltinModelSpec {
        id: "gpt-4o",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gpt-4o-mini",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gpt-4-turbo",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gpt-4",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gpt-3.5-turbo",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gpt-5",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gpt-5-mini",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gpt-5-nano",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gpt-5.1",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gpt-5.2",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "o1",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "o1-mini",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "o1-preview",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "o3",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "o4-mini",
        protocol: ProviderProtocol::OpenAI,
    },
    BuiltinModelSpec {
        id: "gemini-2.5-flash",
        protocol: ProviderProtocol::Google,
    },
    BuiltinModelSpec {
        id: "gemini-2.0-pro",
        protocol: ProviderProtocol::Google,
    },
    BuiltinModelSpec {
        id: "gemini-1.5-pro",
        protocol: ProviderProtocol::Google,
    },
    BuiltinModelSpec {
        id: "gemini-1.5-flash",
        protocol: ProviderProtocol::Google,
    },
];

fn protocol_for_provider(provider: &str) -> Option<ProviderProtocol> {
    resolve_builtin_provider(provider)
        .map(|spec| spec.protocol)
        .or_else(|| provider.parse::<ProviderProtocol>().ok())
}

/// All builtin models for a given provider.
pub fn models_for_provider(provider: &str) -> impl Iterator<Item = &'static BuiltinModelSpec> {
    let protocol = protocol_for_provider(provider);
    BUILTIN_MODELS
        .iter()
        .filter(move |model| protocol.is_some_and(|protocol| model.protocol == protocol))
}

/// All builtin models for a typed provider protocol.
pub fn models_for_protocol(
    protocol: ProviderProtocol,
) -> impl Iterator<Item = &'static BuiltinModelSpec> {
    BUILTIN_MODELS
        .iter()
        .filter(move |model| model.protocol == protocol)
}

/// Look up a builtin model by exact id.
pub fn resolve_builtin_model(id: &str) -> Option<&'static BuiltinModelSpec> {
    BUILTIN_MODELS.iter().find(|model| model.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_resolves_only_under_its_own_id() {
        for spec in BUILTIN_PROVIDERS {
            assert_eq!(
                resolve_builtin_provider(spec.id).map(|resolved| resolved.id),
                Some(spec.id),
                "every built-in provider resolves under its own id"
            );
        }

        for other_name in ["gemini", "vllm-graph-aware", "claude", "gpt"] {
            assert!(
                resolve_builtin_provider(other_name).is_none(),
                "'{other_name}' is not a provider id, so it resolves to no provider"
            );
        }
    }

    #[test]
    fn provider_ids_are_unique_across_the_catalog() {
        let mut ids: Vec<&str> = BUILTIN_PROVIDERS.iter().map(|spec| spec.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(count, ids.len(), "each provider is named exactly once");
    }

    #[test]
    fn models_resolve_for_a_provider_only_under_its_own_id() {
        assert!(
            models_for_provider("google").any(|model| model.id == "gemini-2.5-flash"),
            "'google' is the provider id serving the Gemini models"
        );
        assert_eq!(
            models_for_provider("gemini").count(),
            0,
            "'gemini' names models, not a provider, so it selects no provider catalogue"
        );
    }
}
