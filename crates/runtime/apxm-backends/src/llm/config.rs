//! Backend registration configuration types.
//!
//! These structures describe registered LLM backends, their deployment mode,
//! and the models they serve. They live with backend ownership because their
//! semantics are interpreted by backend adapters and credential tooling.

use super::ProviderProtocol;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Backend deployment type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    /// Cloud-hosted service.
    Cloud,
    /// On-premises enterprise deployment or gateway.
    OnPrem,
    /// Local backend managed outside or alongside APXM.
    Local,
}

impl BackendType {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::OnPrem => "onprem",
            Self::Local => "local",
        }
    }
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for BackendType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "cloud" => Ok(Self::Cloud),
            "onprem" => Ok(Self::OnPrem),
            "local" => Ok(Self::Local),
            _ => Err(format!("Unknown backend type: '{value}'")),
        }
    }
}

/// Complete backend registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Unique backend identifier.
    pub name: String,
    /// Backend deployment type.
    #[serde(rename = "type")]
    pub backend_type: BackendType,
    /// Wire protocol this backend speaks.
    pub protocol: ProviderProtocol,
    /// API endpoint URL.
    pub endpoint: Option<String>,
    /// API key or `env:VAR_NAME` reference.
    pub api_key: Option<String>,
    /// Custom HTTP headers.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Models hosted on this backend.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelConfig>,
    /// Optional local container metadata.
    pub docker: Option<DockerConfig>,
    /// Whether this backend accepts automatic tool selection.
    #[serde(default)]
    pub auto_tool_choice: Option<bool>,
    /// Whether this backend accepts provider-enforced structured outputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_structured_outputs: Option<bool>,
}

/// Model metadata and capability hints for routing and request shaping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Model identifier sent to the API.
    pub id: String,
    /// Alternative names for this model.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Maximum context window in tokens. `0` means unknown.
    #[serde(default)]
    pub context_window: usize,
    /// Cost per 1000 input tokens in USD. `0.0` means free or unknown.
    #[serde(default)]
    pub cost_per_1k_input: f64,
    /// Cost per 1000 output tokens in USD. `0.0` means free or unknown.
    #[serde(default)]
    pub cost_per_1k_output: f64,
    /// Whether the model supports vision/image inputs.
    #[serde(default)]
    pub supports_vision: bool,
    /// Whether the model supports function/tool calling.
    #[serde(default)]
    pub supports_functions: bool,
    /// Whether the model supports extended thinking/reasoning.
    #[serde(default)]
    pub supports_thinking: bool,
    /// Whether the model accepts an explicit custom `temperature` value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_custom_temperature: Option<bool>,
    /// Whether this model accepts provider-enforced structured output schemas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_structured_outputs: Option<bool>,
    /// Maximum output tokens per request.
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
    /// Arbitrary classification tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Optional local container metadata for backend lifecycle tooling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Container image name.
    pub image: String,
    /// Additional command-line arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables set in the container.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Override the container default command.
    #[serde(default)]
    pub command: Vec<String>,
    /// Path to local model weights.
    pub model_path: Option<String>,
    /// Number of GPUs for tensor parallelism.
    pub tensor_parallel: Option<usize>,
}
