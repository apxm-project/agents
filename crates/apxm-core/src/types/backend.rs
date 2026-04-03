//! Backend configuration types for unified LLM backend management.
//!
//! This module defines the core types for the backend unification system:
//! - [`BackendType`]: Cloud, on-premises, or local backends
//! - [`BackendConfig`]: Complete backend configuration with models
//! - [`ModelConfig`]: Model metadata and capabilities
//! - [`DockerConfig`]: Docker container configuration for local backends

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::provider_spec::ProviderProtocol;

/// Backend deployment type.
///
/// Determines lifecycle management and configuration requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    /// Cloud-hosted service (OpenAI, Anthropic, etc.)
    Cloud,
    /// On-premises enterprise deployment (corporate gateway)
    OnPrem,
    /// Local containerized backend (vLLM, Ollama)
    Local,
}

impl BackendType {
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendType::Cloud => "cloud",
            BackendType::OnPrem => "onprem",
            BackendType::Local => "local",
        }
    }
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for BackendType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cloud" => Ok(BackendType::Cloud),
            "onprem" => Ok(BackendType::OnPrem),
            "local" => Ok(BackendType::Local),
            _ => Err(format!("Unknown backend type: '{}'", s)),
        }
    }
}

/// Complete backend configuration.
///
/// Unifies credential, endpoint, and model information into a single
/// structure. Replaces the fragmented `credentials.toml` + `models.toml`
/// system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Unique backend identifier (e.g., "openai", "corp-gateway", "local-vllm")
    pub name: String,

    /// Backend deployment type
    #[serde(rename = "type")]
    pub backend_type: BackendType,

    /// Wire protocol this backend speaks
    pub protocol: ProviderProtocol,

    /// API endpoint URL (optional for cloud providers with default URLs)
    pub endpoint: Option<String>,

    /// API key or "env:VAR_NAME" to read from environment
    pub api_key: Option<String>,

    /// Custom HTTP headers (e.g., subscription keys, user headers)
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Models hosted on this backend
    #[serde(default)]
    pub models: Vec<ModelConfig>,

    /// Docker configuration for local backends
    pub docker: Option<DockerConfig>,
}

/// Model metadata and capabilities.
///
/// Describes a model's identity, pricing, and feature support.
/// Used for routing, cost estimation, and capability checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Model identifier sent to the API (e.g., "gpt-4", "claude-3-opus")
    pub id: String,

    /// Alternative names for this model (e.g., ["gpt4", "gpt-4-turbo"])
    #[serde(default)]
    pub aliases: Vec<String>,

    /// Maximum context window in tokens (0 = unknown)
    #[serde(default)]
    pub context_window: usize,

    /// Cost per 1000 input tokens in USD (0.0 = free/unknown)
    #[serde(default)]
    pub cost_per_1k_input: f64,

    /// Cost per 1000 output tokens in USD (0.0 = free/unknown)
    #[serde(default)]
    pub cost_per_1k_output: f64,

    /// Whether the model supports vision/image inputs
    #[serde(default)]
    pub supports_vision: bool,

    /// Whether the model supports function/tool calling
    #[serde(default)]
    pub supports_functions: bool,

    /// Whether the model supports extended thinking/reasoning
    #[serde(default)]
    pub supports_thinking: bool,

    /// Arbitrary classification tags (e.g., ["fast", "cheap", "multilingual"])
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Docker container configuration for local backends.
///
/// Used by [`BackendType::Local`] backends to manage containerized
/// inference servers (vLLM, Ollama, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Docker image name (e.g., "vllm/vllm-openai:latest")
    pub image: String,

    /// Additional command-line arguments to pass to the container
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables to set in the container
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Override the container's default command
    #[serde(default)]
    pub command: Vec<String>,

    /// Path to the model weights (host path, mounted into container)
    pub model_path: Option<String>,

    /// Number of GPUs for tensor parallelism
    pub tensor_parallel: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_type_serde() {
        let bt = BackendType::Cloud;
        let json = serde_json::to_string(&bt).unwrap();
        assert_eq!(json, r#""cloud""#);
        let deserialized: BackendType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, BackendType::Cloud);
    }

    #[test]
    fn test_backend_type_from_str() {
        assert_eq!("cloud".parse::<BackendType>().unwrap(), BackendType::Cloud);
        assert_eq!("onprem".parse::<BackendType>().unwrap(), BackendType::OnPrem);
        assert_eq!("local".parse::<BackendType>().unwrap(), BackendType::Local);
        assert!("unknown".parse::<BackendType>().is_err());
    }

    #[test]
    fn test_backend_config_serde() {
        let config = BackendConfig {
            name: "test-backend".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: Some("https://api.example.com".to_string()),
            api_key: Some("env:TEST_KEY".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: BackendConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test-backend");
        assert_eq!(deserialized.backend_type, BackendType::Cloud);
    }

    #[test]
    fn test_model_config_defaults() {
        let json = r#"{"id": "test-model"}"#;
        let config: ModelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.id, "test-model");
        assert_eq!(config.aliases.len(), 0);
        assert_eq!(config.context_window, 0);
        assert_eq!(config.cost_per_1k_input, 0.0);
        assert!(!config.supports_vision);
        assert!(!config.supports_functions);
        assert!(!config.supports_thinking);
    }

    #[test]
    fn test_docker_config_serde() {
        let mut env = HashMap::new();
        env.insert("GPU_MEMORY".to_string(), "24GB".to_string());

        let config = DockerConfig {
            image: "vllm/vllm-openai:latest".to_string(),
            args: vec!["--dtype".to_string(), "float16".to_string()],
            env,
            command: vec![],
            model_path: Some("/models/llama-7b".to_string()),
            tensor_parallel: Some(2),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: DockerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.image, "vllm/vllm-openai:latest");
        assert_eq!(deserialized.tensor_parallel, Some(2));
    }

    #[test]
    fn test_backend_config_with_models() {
        let config = BackendConfig {
            name: "openai".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: None,
            api_key: Some("env:OPENAI_API_KEY".to_string()),
            headers: HashMap::new(),
            models: vec![
                ModelConfig {
                    id: "gpt-4".to_string(),
                    aliases: vec!["gpt4".to_string()],
                    context_window: 8192,
                    cost_per_1k_input: 0.03,
                    cost_per_1k_output: 0.06,
                    supports_vision: false,
                    supports_functions: true,
                    supports_thinking: false,
                    tags: vec!["production".to_string()],
                },
            ],
            docker: None,
        };

        assert_eq!(config.models.len(), 1);
        assert_eq!(config.models[0].id, "gpt-4");
        assert_eq!(config.models[0].context_window, 8192);
    }
}
