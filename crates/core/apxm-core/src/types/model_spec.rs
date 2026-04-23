//! Data-driven model metadata (single source of truth for well-known models).
//!
//! Follows the same pattern as [`super::provider_spec::BuiltinProviderSpec`]:
//! a const-friendly struct with a static registry, lookup helpers, and
//! codegen-consumable accessors.

/// Static model spec (const-friendly, no heap allocations).
#[derive(Debug, Clone, Copy)]
pub struct BuiltinModelSpec {
    /// Canonical model identifier (e.g. "claude-sonnet-4-5").
    pub id: &'static str,
    /// Provider this model belongs to (matches `BuiltinProviderSpec::id`).
    pub provider: &'static str,
    /// Whether this is the default model for its provider.
    pub is_default: bool,
}

/// All built-in models, grouped by provider.
pub const BUILTIN_MODELS: &[BuiltinModelSpec] = &[
    // --- Anthropic ---
    BuiltinModelSpec {
        id: "claude-opus-4-6",
        provider: "anthropic",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-sonnet-4-6",
        provider: "anthropic",
        is_default: true,
    },
    BuiltinModelSpec {
        id: "claude-haiku-4-5",
        provider: "anthropic",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-opus-4-5",
        provider: "anthropic",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-sonnet-4-5",
        provider: "anthropic",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-7-sonnet",
        provider: "anthropic",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-5-sonnet-20241022",
        provider: "anthropic",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-opus-20240229",
        provider: "anthropic",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-sonnet-20240229",
        provider: "anthropic",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-haiku-20240307",
        provider: "anthropic",
        is_default: false,
    },
    // --- OpenAI ---
    BuiltinModelSpec {
        id: "gpt-4o",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-4o-mini",
        provider: "openai",
        is_default: true,
    },
    BuiltinModelSpec {
        id: "gpt-4-turbo",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-4",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-3.5-turbo",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5-mini",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5-nano",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5.1",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5.2",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o1",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o1-mini",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o1-preview",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o3",
        provider: "openai",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o4-mini",
        provider: "openai",
        is_default: false,
    },
    // --- Google ---
    BuiltinModelSpec {
        id: "gemini-2.5-flash",
        provider: "google",
        is_default: true,
    },
    BuiltinModelSpec {
        id: "gemini-2.0-pro",
        provider: "google",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gemini-1.5-pro",
        provider: "google",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gemini-1.5-flash",
        provider: "google",
        is_default: false,
    },
    // --- vLLM ---
    BuiltinModelSpec {
        id: "Qwen/Qwen2.5-7B-Instruct",
        provider: "vllm",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "Qwen/Qwen2.5-14B-Instruct",
        provider: "vllm",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "meta-llama/Llama-3.1-8B-Instruct",
        provider: "vllm",
        is_default: false,
    },
    BuiltinModelSpec {
        id: "meta-llama/Llama-3.1-70B-Instruct",
        provider: "vllm",
        is_default: false,
    },
];

/// All builtin models for a given provider.
pub fn models_for_provider(provider: &str) -> impl Iterator<Item = &'static BuiltinModelSpec> {
    BUILTIN_MODELS
        .iter()
        .filter(move |m| m.provider == provider)
}

/// The default model for a given provider, if one is marked.
///
/// Self-hosted providers such as `vllm` may intentionally have no builtin
/// default because the served model is an operator choice, not a provider
/// invariant.
pub fn default_model_for_provider(provider: &str) -> Option<&'static str> {
    BUILTIN_MODELS
        .iter()
        .find(|m| m.provider == provider && m.is_default)
        .map(|m| m.id)
}

/// Look up a builtin model by exact id.
pub fn resolve_builtin_model(id: &str) -> Option<&'static BuiltinModelSpec> {
    BUILTIN_MODELS.iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_provider_has_at_most_one_default() {
        let providers: Vec<&str> = BUILTIN_MODELS
            .iter()
            .map(|m| m.provider)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        for provider in providers {
            let defaults: Vec<_> = BUILTIN_MODELS
                .iter()
                .filter(|m| m.provider == provider && m.is_default)
                .collect();
            assert!(
                defaults.len() <= 1,
                "Provider '{provider}' should have at most 1 default model, found {}",
                defaults.len()
            );
        }
    }

    #[test]
    fn all_ids_unique() {
        let mut ids: Vec<&str> = BUILTIN_MODELS.iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BUILTIN_MODELS.len());
    }

    #[test]
    fn default_model_for_anthropic() {
        assert_eq!(
            default_model_for_provider("anthropic"),
            Some("claude-sonnet-4-6")
        );
    }

    #[test]
    fn default_model_for_openai() {
        assert_eq!(default_model_for_provider("openai"), Some("gpt-4o-mini"));
    }

    #[test]
    fn default_model_for_google() {
        assert_eq!(
            default_model_for_provider("google"),
            Some("gemini-2.5-flash")
        );
    }

    #[test]
    fn vllm_has_no_builtin_default_model() {
        assert_eq!(default_model_for_provider("vllm"), None);
    }

    #[test]
    fn unknown_provider_returns_none() {
        assert_eq!(default_model_for_provider("unknown"), None);
    }

    #[test]
    fn resolve_known_model() {
        let spec = resolve_builtin_model("gpt-4o").unwrap();
        assert_eq!(spec.provider, "openai");
    }

    #[test]
    fn models_for_provider_count() {
        assert_eq!(models_for_provider("anthropic").count(), 10);
        assert_eq!(models_for_provider("openai").count(), 15);
        assert_eq!(models_for_provider("google").count(), 4);
    }
}
