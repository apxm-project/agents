//! Backend protocol and provider configuration types.
//!
//! Concrete backend protocol identities belong to the backend layer. Core
//! artifacts may carry backend/model identifiers, but they should not own the
//! list of providers APXM can speak to.

use serde::{Deserialize, Serialize};

/// The wire protocol a registered LLM backend speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderProtocol {
    /// OpenAI-compatible API.
    OpenAI,
    /// Anthropic Messages API.
    Anthropic,
    /// Google Gemini API.
    Google,
    /// Ollama local API.
    Ollama,
    /// vLLM-compatible API with APXM graph-awareness extensions.
    Vllm,
    /// Mock backend for tests and benchmarks.
    Mock,
}

impl ProviderProtocol {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::Ollama => "ollama",
            Self::Vllm => "vllm",
            Self::Mock => "mock",
        }
    }

    pub const fn all_variants() -> &'static [Self] {
        &[
            Self::OpenAI,
            Self::Anthropic,
            Self::Google,
            Self::Ollama,
            Self::Vllm,
            Self::Mock,
        ]
    }
}

impl std::fmt::Display for ProviderProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ProviderProtocol {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "anthropic" => Ok(Self::Anthropic),
            "google" => Ok(Self::Google),
            "ollama" => Ok(Self::Ollama),
            "vllm" | "vllm-graph-aware" => Ok(Self::Vllm),
            "mock" => Ok(Self::Mock),
            _ => Err(format!("Unknown provider protocol: '{value}'")),
        }
    }
}

/// Data-driven provider metadata exposed to generated frontends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSpec {
    /// Canonical provider identifier.
    pub id: String,
    /// Environment variable name for the API key.
    pub api_key_env_var: Option<String>,
    /// Default base URL.
    pub default_base_url: Option<String>,
    /// Whether this provider requires an API key to function.
    pub requires_api_key: bool,
    /// The wire protocol this provider speaks.
    pub protocol: ProviderProtocol,
    /// Alternative names for this provider.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// Normalize provider endpoints so versioned APIs store `/v1` exactly once.
///
/// The `/v1` version prefix is only auto-appended for a bare host
/// (e.g. `https://api.openai.com` -> `https://api.openai.com/v1`). When the
/// caller supplies an endpoint that already carries a path — such as a gateway
/// base like `https://host/openai` (Azure APIM) — the path is trusted verbatim
/// and no `/v1` is synthesized, since the gateway routes under that prefix and
/// would 404 on `/openai/v1/...`.
pub fn normalize_endpoint_for_protocol(protocol: ProviderProtocol, endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    let needs_v1 = matches!(
        protocol,
        ProviderProtocol::OpenAI | ProviderProtocol::Anthropic | ProviderProtocol::Vllm
    );
    if !needs_v1 || trimmed.ends_with("/v1") {
        return trimmed.to_string();
    }
    // A path beyond the host means the caller pinned the API base explicitly.
    let after_scheme = trimmed.split_once("://").map_or(trimmed, |(_, rest)| rest);
    if after_scheme.contains('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

/// Some enterprise Anthropic gateways (e.g. APIM) publish a base path like
/// `/Anthropic` while the Messages API still lives under `/v1/messages`.
pub fn normalize_anthropic_gateway_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with("/Anthropic") && !trimmed.ends_with("/v1") {
        format!("{trimmed}/v1")
    } else {
        trimmed.to_string()
    }
}
