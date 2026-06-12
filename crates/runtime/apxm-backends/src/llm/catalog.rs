//! Built-in LLM provider and model catalog.
//!
//! This catalog belongs to the backend layer. Core artifacts may carry backend
//! and model identifiers, but concrete provider ids, model ids, default
//! endpoints, and provider-specific environment variable names are backend
//! concerns.

use super::{ProviderProtocol, ProviderSpec};

pub const DEFAULT_VLLM_BASE_URL: &str = "http://localhost:8916/v1";

/// Static provider spec (const-friendly, no heap allocations).
#[derive(Debug, Clone, Copy)]
pub struct BuiltinProviderSpec {
    pub id: &'static str,
    pub api_key_env_var: Option<&'static str>,
    pub default_base_url: Option<&'static str>,
    pub requires_api_key: bool,
    pub protocol: ProviderProtocol,
    pub aliases: &'static [&'static str],
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
            aliases: self
                .aliases
                .iter()
                .map(|alias| (*alias).to_string())
                .collect(),
        }
    }
}

/// Built-in provider specifications.
pub const BUILTIN_PROVIDERS: &[BuiltinProviderSpec] = &[
    BuiltinProviderSpec {
        id: "ollama",
        api_key_env_var: Some("OLLAMA_API_KEY"),
        default_base_url: Some("http://localhost:11434"),
        requires_api_key: false,
        protocol: ProviderProtocol::Ollama,
        aliases: &[],
    },
    BuiltinProviderSpec {
        id: "openai",
        api_key_env_var: Some("OPENAI_API_KEY"),
        default_base_url: None,
        requires_api_key: true,
        protocol: ProviderProtocol::OpenAI,
        aliases: &[],
    },
    BuiltinProviderSpec {
        id: "anthropic",
        api_key_env_var: Some("ANTHROPIC_API_KEY"),
        default_base_url: None,
        requires_api_key: true,
        protocol: ProviderProtocol::Anthropic,
        aliases: &[],
    },
    BuiltinProviderSpec {
        id: "google",
        api_key_env_var: Some("GOOGLE_API_KEY"),
        default_base_url: None,
        requires_api_key: true,
        protocol: ProviderProtocol::Google,
        aliases: &["gemini"],
    },
    BuiltinProviderSpec {
        id: "vllm",
        api_key_env_var: None,
        default_base_url: Some(DEFAULT_VLLM_BASE_URL),
        requires_api_key: false,
        protocol: ProviderProtocol::Vllm,
        aliases: &["vllm-graph-aware"],
    },
    BuiltinProviderSpec {
        id: "openrouter",
        api_key_env_var: Some("OPENROUTER_API_KEY"),
        default_base_url: Some("https://openrouter.ai/api/v1"),
        requires_api_key: true,
        protocol: ProviderProtocol::OpenAI,
        aliases: &[],
    },
    BuiltinProviderSpec {
        id: "mock",
        api_key_env_var: None,
        default_base_url: None,
        requires_api_key: false,
        protocol: ProviderProtocol::Mock,
        aliases: &[],
    },
];

/// Look up a built-in provider by id or alias.
pub fn resolve_builtin_provider(name: &str) -> Option<&'static BuiltinProviderSpec> {
    let lower = name.to_lowercase();
    BUILTIN_PROVIDERS
        .iter()
        .find(|spec| spec.id == lower || spec.aliases.iter().any(|alias| *alias == lower))
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
    /// Whether this is the default model for its provider.
    pub is_default: bool,
}

/// All built-in models, grouped by provider.
pub const BUILTIN_MODELS: &[BuiltinModelSpec] = &[
    BuiltinModelSpec {
        id: "claude-opus-4-6",
        protocol: ProviderProtocol::Anthropic,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-sonnet-4-6",
        protocol: ProviderProtocol::Anthropic,
        is_default: true,
    },
    BuiltinModelSpec {
        id: "claude-haiku-4-5",
        protocol: ProviderProtocol::Anthropic,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-opus-4-5",
        protocol: ProviderProtocol::Anthropic,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-sonnet-4-5",
        protocol: ProviderProtocol::Anthropic,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-7-sonnet",
        protocol: ProviderProtocol::Anthropic,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-5-sonnet-20241022",
        protocol: ProviderProtocol::Anthropic,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-opus-20240229",
        protocol: ProviderProtocol::Anthropic,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-sonnet-20240229",
        protocol: ProviderProtocol::Anthropic,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "claude-3-haiku-20240307",
        protocol: ProviderProtocol::Anthropic,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-4o",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-4o-mini",
        protocol: ProviderProtocol::OpenAI,
        is_default: true,
    },
    BuiltinModelSpec {
        id: "gpt-4-turbo",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-4",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-3.5-turbo",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5-mini",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5-nano",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5.1",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gpt-5.2",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o1",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o1-mini",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o1-preview",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o3",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "o4-mini",
        protocol: ProviderProtocol::OpenAI,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gemini-2.5-flash",
        protocol: ProviderProtocol::Google,
        is_default: true,
    },
    BuiltinModelSpec {
        id: "gemini-2.0-pro",
        protocol: ProviderProtocol::Google,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gemini-1.5-pro",
        protocol: ProviderProtocol::Google,
        is_default: false,
    },
    BuiltinModelSpec {
        id: "gemini-1.5-flash",
        protocol: ProviderProtocol::Google,
        is_default: false,
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

/// The default model for a given provider, if one is marked.
pub fn default_model_for_provider(provider: &str) -> Option<&'static str> {
    let protocol = protocol_for_provider(provider)?;
    default_model_for_protocol(protocol)
}

/// The default model for a typed provider protocol, if one is marked.
pub fn default_model_for_protocol(protocol: ProviderProtocol) -> Option<&'static str> {
    BUILTIN_MODELS
        .iter()
        .find(|model| model.protocol == protocol && model.is_default)
        .map(|model| model.id)
}

/// Look up a builtin model by exact id.
pub fn resolve_builtin_model(id: &str) -> Option<&'static BuiltinModelSpec> {
    BUILTIN_MODELS.iter().find(|model| model.id == id)
}

