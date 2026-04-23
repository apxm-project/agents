//! Backend configuration management (unified replacement for credentials).
//!
//! This module provides the [`BackendStore`] for managing the new unified
//! backend configuration system. Unlike the legacy credentials system, backends
//! can include type information (cloud/onprem/local), model metadata, and Docker
//! configurations for local deployments.

use apxm_core::types::{BackendConfig, ModelConfig};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

const FILE_PERMISSIONS: u32 = 0o600;
const DIR_PERMISSIONS: u32 = 0o700;
const CONFIG_FILENAME: &str = "config.toml";
const FILE_HEADER: &str = "# APXM Configuration - Managed by `apxm backend`\n\
                            # Permissions: 0600 (owner read/write only)\n\n";

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse config file: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("Failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),

    #[error("Backend '{name}' already exists. Remove it first with: apxm backend remove {name}")]
    AlreadyExists { name: String },

    #[error("Backend '{name}' not found")]
    NotFound { name: String },

    #[error("Model '{model}' already exists in backend '{backend}'")]
    ModelAlreadyExists { backend: String, model: String },

    #[error("Unable to determine home directory")]
    HomeDirMissing,

    #[error(
        "Config file at {path} has insecure permissions ({mode:o}). Fix with: chmod 600 {path}"
    )]
    InsecurePermissions { path: String, mode: u32 },
}

/// Simplified config structure for backend management.
///
/// This only handles the `[[backends]]` section. Full ApXmConfig is handled
/// by apxm-driver.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct BackendConfigFile {
    #[serde(default)]
    backends: Vec<BackendConfig>,
}

/// Backend configuration store.
///
/// Manages the unified backend configuration at `~/.apxm/config.toml`.
/// Replaces the legacy credential-based system with a hierarchical
/// Backend → Model → Endpoint structure.
pub struct BackendStore {
    config_path: PathBuf,
    dir: PathBuf,
}

impl BackendStore {
    /// Open the backend store at the default location (~/.apxm/config.toml).
    pub fn open() -> Result<Self, BackendError> {
        let home = dirs::home_dir().ok_or(BackendError::HomeDirMissing)?;
        let dir = home.join(".apxm");
        let config_path = dir.join(CONFIG_FILENAME);
        Ok(Self { config_path, dir })
    }

    /// Ensure the directory exists with correct permissions.
    fn ensure_dir(&self) -> Result<(), BackendError> {
        if !self.dir.exists() {
            fs::create_dir_all(&self.dir)?;
            fs::set_permissions(&self.dir, fs::Permissions::from_mode(DIR_PERMISSIONS))?;
        }
        // Create .gitignore as safety net
        let gitignore = self.dir.join(".gitignore");
        if !gitignore.exists() {
            fs::write(&gitignore, "config.toml\ncredentials.toml\n")?;
        }
        Ok(())
    }

    /// Check file permissions are secure (owner-only read/write).
    fn check_permissions(&self) -> Result<(), BackendError> {
        if !self.config_path.exists() {
            return Ok(());
        }
        let metadata = fs::metadata(&self.config_path)?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(BackendError::InsecurePermissions {
                path: self.config_path.display().to_string(),
                mode,
            });
        }
        Ok(())
    }

    /// Read the config file (backends section only).
    fn read_file(&self) -> Result<BackendConfigFile, BackendError> {
        self.check_permissions()?;
        if !self.config_path.exists() {
            return Ok(BackendConfigFile::default());
        }
        let contents = fs::read_to_string(&self.config_path)?;
        let file: BackendConfigFile = toml::from_str(&contents)?;
        Ok(file)
    }

    /// Write the config file atomically.
    fn write_file(&self, file: &BackendConfigFile) -> Result<(), BackendError> {
        self.ensure_dir()?;
        let serialized = toml::to_string_pretty(file)?;
        let content = format!("{FILE_HEADER}{serialized}");

        // Atomic write via tempfile
        let temp = tempfile::NamedTempFile::new_in(&self.dir)?;
        fs::write(temp.path(), &content)?;
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(FILE_PERMISSIONS))?;
        temp.persist(&self.config_path)
            .map_err(std::io::Error::other)?;
        Ok(())
    }

    /// Add a backend. Fails if name already exists.
    ///
    /// For protocols that use a versioned API path (OpenAI, Anthropic, vLLM),
    /// the endpoint is normalized to include `/v1` so that downstream code
    /// can append resource paths directly (e.g. `/chat/completions`, `/models`).
    pub fn add(&self, mut backend: BackendConfig) -> Result<(), BackendError> {
        let mut file = self.read_file()?;
        if file.backends.iter().any(|b| b.name == backend.name) {
            return Err(BackendError::AlreadyExists {
                name: backend.name.clone(),
            });
        }
        normalize_endpoint(&mut backend);
        file.backends.push(backend);
        self.write_file(&file)
    }

    /// Remove a backend by name.
    pub fn remove(&self, name: &str) -> Result<(), BackendError> {
        let mut file = self.read_file()?;
        let original_len = file.backends.len();
        file.backends.retain(|b| b.name != name);
        if file.backends.len() == original_len {
            return Err(BackendError::NotFound {
                name: name.to_string(),
            });
        }
        self.write_file(&file)
    }

    /// Get a backend by name.
    pub fn get(&self, name: &str) -> Result<Option<BackendConfig>, BackendError> {
        let file = self.read_file()?;
        Ok(file.backends.into_iter().find(|b| b.name == name))
    }

    /// List all backends.
    pub fn list(&self) -> Result<Vec<BackendConfig>, BackendError> {
        let file = self.read_file()?;
        Ok(file.backends)
    }

    /// Update a backend by name.
    pub fn update(&self, name: &str, backend: BackendConfig) -> Result<(), BackendError> {
        let mut file = self.read_file()?;
        let pos = file
            .backends
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| BackendError::NotFound {
                name: name.to_string(),
            })?;
        file.backends[pos] = backend;
        self.write_file(&file)
    }

    /// Add a model to an existing backend. Fails if the model ID already exists.
    pub fn add_model(&self, backend_name: &str, model: ModelConfig) -> Result<(), BackendError> {
        let mut file = self.read_file()?;
        let pos = file
            .backends
            .iter()
            .position(|b| b.name == backend_name)
            .ok_or_else(|| BackendError::NotFound {
                name: backend_name.to_string(),
            })?;
        if file.backends[pos].models.iter().any(|m| m.id == model.id) {
            return Err(BackendError::ModelAlreadyExists {
                backend: backend_name.to_string(),
                model: model.id,
            });
        }
        file.backends[pos].models.push(model);
        self.write_file(&file)
    }

    /// Migrate from legacy credentials.toml to backends.
    ///
    /// Returns the number of credentials migrated.
    pub fn migrate_from_credentials(&self) -> Result<usize, BackendError> {
        use serde::Deserialize;
        use std::collections::BTreeMap;

        // Define minimal credential structure for reading
        #[derive(Debug, Clone, Deserialize)]
        struct Credential {
            provider: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            api_key: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            base_url: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            model: Option<String>,
            #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
            headers: BTreeMap<String, String>,
        }

        #[derive(Debug, Default, Deserialize)]
        struct CredentialsFile {
            #[serde(default)]
            credentials: BTreeMap<String, Credential>,
        }

        // Read credentials.toml directly
        let home = dirs::home_dir().ok_or(BackendError::HomeDirMissing)?;
        let credentials_path = home.join(".apxm").join("credentials.toml");

        if !credentials_path.exists() {
            return Ok(0);
        }

        let contents = fs::read_to_string(&credentials_path)?;
        let creds_file: CredentialsFile = toml::from_str(&contents)?;

        if creds_file.credentials.is_empty() {
            return Ok(0);
        }

        let mut file = self.read_file()?;
        let mut count = 0;

        for (name, cred) in creds_file.credentials {
            // Skip if already migrated
            if file.backends.iter().any(|b| b.name == name) {
                continue;
            }

            let legacy_cred = LegacyCredential {
                provider: cred.provider,
                api_key: cred.api_key,
                base_url: cred.base_url,
                model: cred.model,
                headers: cred.headers,
            };
            let backend = credential_to_backend(&name, legacy_cred);
            file.backends.push(backend);
            count += 1;
        }

        if count > 0 {
            self.write_file(&file)?;
        }

        Ok(count)
    }

    /// Get the path to the config file.
    pub fn path(&self) -> &Path {
        &self.config_path
    }
}

/// Minimal credential structure for migration (matches credentials.toml format).
#[derive(Debug)]
struct LegacyCredential {
    provider: String,
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    headers: std::collections::BTreeMap<String, String>,
}

/// Convert a legacy Credential (from credentials.toml) to a BackendConfig.
fn credential_to_backend(name: &str, cred: LegacyCredential) -> BackendConfig {
    use apxm_core::types::ProviderProtocol;
    use apxm_core::types::{BackendType, ModelConfig};
    use std::collections::HashMap;

    // Infer protocol from provider string
    let protocol = match cred.provider.to_lowercase().as_str() {
        "openai" => ProviderProtocol::OpenAI,
        "anthropic" => ProviderProtocol::Anthropic,
        "google" => ProviderProtocol::Google,
        "ollama" => ProviderProtocol::Ollama,
        "vllm" => ProviderProtocol::Vllm,
        _ => ProviderProtocol::OpenAI, // Default fallback
    };

    // Infer backend type from endpoint
    let backend_type = if let Some(url) = &cred.base_url {
        if url.contains("localhost") || url.contains("127.0.0.1") {
            BackendType::Local
        } else if url.contains("openai.com")
            || url.contains("anthropic.com")
            || url.contains("googleapis.com")
        {
            BackendType::Cloud
        } else {
            BackendType::OnPrem
        }
    } else {
        BackendType::Cloud
    };

    // Convert BTreeMap to HashMap
    let headers: HashMap<String, String> = cred.headers.into_iter().collect();

    // Create model config if model is specified
    let models = if let Some(model_id) = cred.model {
        vec![ModelConfig {
            id: model_id,
            aliases: vec![],
            context_window: 0,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            supports_vision: false,
            supports_functions: false,
            supports_thinking: false,
            supports_custom_temperature: None,
            max_output_tokens: None,
            tags: vec![],
        }]
    } else {
        vec![]
    };

    BackendConfig {
        name: name.to_string(),
        backend_type,
        protocol,
        endpoint: cred.base_url,
        api_key: cred.api_key,
        headers,
        models,
        docker: None,
        auto_tool_choice: None,
    }
}

/// Ensure the stored endpoint includes the `/v1` version prefix for protocols
/// that need it (OpenAI, Anthropic, vLLM). This makes the endpoint consistent
/// with the runtime, which builds paths like `${endpoint}/chat/completions`.
fn normalize_endpoint(backend: &mut BackendConfig) {
    use apxm_core::types::ProviderProtocol;

    let needs_v1 = matches!(
        backend.protocol,
        ProviderProtocol::OpenAI | ProviderProtocol::Anthropic | ProviderProtocol::Vllm
    );
    if !needs_v1 {
        return;
    }

    if let Some(ref mut url) = backend.endpoint {
        let trimmed = url.trim_end_matches('/');
        if !trimmed.ends_with("/v1") {
            *url = format!("{trimmed}/v1");
        } else {
            // Normalize trailing slash: "http://x/v1/" → "http://x/v1"
            *url = trimmed.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::ProviderProtocol;
    use apxm_core::types::{BackendType, DockerConfig, ModelConfig};
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn test_store(dir: &Path) -> BackendStore {
        let config_path = dir.join(CONFIG_FILENAME);
        BackendStore {
            config_path,
            dir: dir.to_path_buf(),
        }
    }

    #[test]
    fn add_and_get_backend() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "test-backend".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-test-key".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend.clone()).unwrap();
        let got = store.get("test-backend").unwrap().unwrap();
        assert_eq!(got.name, "test-backend");
        assert_eq!(got.backend_type, BackendType::Cloud);
        assert_eq!(got.protocol, ProviderProtocol::OpenAI);
    }

    #[test]
    fn add_duplicate_fails() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "dup".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: None,
            api_key: None,
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend.clone()).unwrap();
        let result = store.add(backend);
        assert!(matches!(result, Err(BackendError::AlreadyExists { .. })));
    }

    #[test]
    fn remove_backend() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "to-remove".to_string(),
            backend_type: BackendType::Local,
            protocol: ProviderProtocol::Ollama,
            endpoint: Some("http://localhost:11434".to_string()),
            api_key: None,
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();
        store.remove("to-remove").unwrap();
        assert!(store.get("to-remove").unwrap().is_none());
    }

    #[test]
    fn list_backends() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend1 = BackendConfig {
            name: "backend-1".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: None,
            api_key: Some("key1".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        let backend2 = BackendConfig {
            name: "backend-2".to_string(),
            backend_type: BackendType::OnPrem,
            protocol: ProviderProtocol::Anthropic,
            endpoint: Some("https://internal.example.com".to_string()),
            api_key: Some("key2".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend1).unwrap();
        store.add(backend2).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|b| b.name == "backend-1"));
        assert!(list.iter().any(|b| b.name == "backend-2"));
    }

    #[test]
    fn update_backend() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "update-test".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: None,
            api_key: Some("old-key".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();

        let updated = BackendConfig {
            name: "update-test".to_string(),
            backend_type: BackendType::OnPrem,
            protocol: ProviderProtocol::OpenAI,
            endpoint: Some("https://new-endpoint.com".to_string()),
            api_key: Some("new-key".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.update("update-test", updated).unwrap();
        let got = store.get("update-test").unwrap().unwrap();
        assert_eq!(got.backend_type, BackendType::OnPrem);
        assert_eq!(got.api_key.as_deref(), Some("new-key"));
    }

    #[test]
    fn backend_with_models() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "with-models".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: None,
            api_key: Some("key".to_string()),
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
                    supports_custom_temperature: None,
                    max_output_tokens: None,
                    tags: vec!["production".to_string()],
                },
                ModelConfig {
                    id: "gpt-3.5-turbo".to_string(),
                    aliases: vec!["gpt35".to_string(), "fast".to_string()],
                    context_window: 16384,
                    cost_per_1k_input: 0.001,
                    cost_per_1k_output: 0.002,
                    supports_vision: false,
                    supports_functions: true,
                    supports_thinking: false,
                    supports_custom_temperature: None,
                    max_output_tokens: None,
                    tags: vec!["fast".to_string(), "cheap".to_string()],
                },
            ],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();
        let got = store.get("with-models").unwrap().unwrap();
        assert_eq!(got.models.len(), 2);
        assert_eq!(got.models[0].id, "gpt-4");
        assert_eq!(got.models[1].id, "gpt-3.5-turbo");
    }

    #[test]
    fn backend_with_docker() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let mut env = HashMap::new();
        env.insert("GPU_MEMORY".to_string(), "24GB".to_string());

        let backend = BackendConfig {
            name: "local-vllm".to_string(),
            backend_type: BackendType::Local,
            protocol: ProviderProtocol::Vllm,
            endpoint: Some("http://localhost:8000".to_string()),
            api_key: None,
            headers: HashMap::new(),
            models: vec![],
            docker: Some(DockerConfig {
                image: "vllm/vllm-openai:latest".to_string(),
                args: vec!["--dtype".to_string(), "float16".to_string()],
                env,
                command: vec![],
                model_path: Some("/models/llama-7b".to_string()),
                tensor_parallel: Some(2),
            }),
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();
        let got = store.get("local-vllm").unwrap().unwrap();
        assert!(got.docker.is_some());
        let docker = got.docker.unwrap();
        assert_eq!(docker.image, "vllm/vllm-openai:latest");
        assert_eq!(docker.tensor_parallel, Some(2));
    }

    #[test]
    fn file_permissions() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "perm-test".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: None,
            api_key: Some("key".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();
        let metadata = fs::metadata(&store.config_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, FILE_PERMISSIONS);
    }

    #[test]
    fn add_model_to_backend() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "model-test".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: None,
            api_key: Some("key".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };
        store.add(backend).unwrap();

        let model = ModelConfig {
            id: "gpt-4o".to_string(),
            aliases: vec!["4o".to_string()],
            context_window: 128000,
            cost_per_1k_input: 0.005,
            cost_per_1k_output: 0.015,
            supports_vision: true,
            supports_functions: true,
            supports_thinking: false,
            supports_custom_temperature: None,
            max_output_tokens: None,
            tags: vec!["production".to_string()],
        };
        store.add_model("model-test", model).unwrap();

        let got = store.get("model-test").unwrap().unwrap();
        assert_eq!(got.models.len(), 1);
        assert_eq!(got.models[0].id, "gpt-4o");
        assert_eq!(got.models[0].context_window, 128000);
    }

    #[test]
    fn add_model_to_nonexistent_backend() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let model = ModelConfig {
            id: "gpt-4o".to_string(),
            aliases: vec![],
            context_window: 0,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            supports_vision: false,
            supports_functions: false,
            supports_thinking: false,
            supports_custom_temperature: None,
            max_output_tokens: None,
            tags: vec![],
        };
        let result = store.add_model("nonexistent", model);
        assert!(matches!(result, Err(BackendError::NotFound { .. })));
    }

    #[test]
    fn add_model_duplicate_fails() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "dup-model".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: None,
            api_key: Some("key".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };
        store.add(backend).unwrap();

        let model = ModelConfig {
            id: "gpt-4o".to_string(),
            aliases: vec![],
            context_window: 0,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            supports_vision: false,
            supports_functions: false,
            supports_thinking: false,
            supports_custom_temperature: None,
            max_output_tokens: None,
            tags: vec![],
        };
        store.add_model("dup-model", model.clone()).unwrap();
        let result = store.add_model("dup-model", model);
        assert!(matches!(
            result,
            Err(BackendError::ModelAlreadyExists { .. })
        ));
    }

    #[test]
    fn empty_backend_omits_inline_models_and_still_accepts_add_model() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "empty-model-list".to_string(),
            backend_type: BackendType::Local,
            protocol: ProviderProtocol::Vllm,
            endpoint: Some("http://localhost:8916".to_string()),
            api_key: None,
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };
        store.add(backend).unwrap();

        let serialized = fs::read_to_string(store.path()).unwrap();
        assert!(
            !serialized.contains("models = []"),
            "empty backends should omit inline models arrays:\n{serialized}"
        );

        let model = ModelConfig {
            id: "google/gemma-3-4b-it".to_string(),
            aliases: vec!["gemma".to_string()],
            context_window: 32768,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            supports_vision: false,
            supports_functions: false,
            supports_thinking: true,
            supports_custom_temperature: None,
            max_output_tokens: Some(4096),
            tags: vec!["vllm".to_string(), "gemma".to_string()],
        };
        store.add_model("empty-model-list", model).unwrap();

        let reopened = store.get("empty-model-list").unwrap().unwrap();
        assert_eq!(reopened.models.len(), 1);
        assert_eq!(reopened.models[0].id, "google/gemma-3-4b-it");
    }

    #[test]
    fn endpoint_normalized_with_v1_on_add() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "vllm-local".to_string(),
            backend_type: BackendType::Local,
            protocol: ProviderProtocol::Vllm,
            endpoint: Some("http://localhost:8000".to_string()),
            api_key: None,
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();
        let got = store.get("vllm-local").unwrap().unwrap();
        assert_eq!(got.endpoint.as_deref(), Some("http://localhost:8000/v1"));
    }

    #[test]
    fn endpoint_already_has_v1_unchanged() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "openai-prod".to_string(),
            backend_type: BackendType::Cloud,
            protocol: ProviderProtocol::OpenAI,
            endpoint: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-test".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();
        let got = store.get("openai-prod").unwrap().unwrap();
        assert_eq!(got.endpoint.as_deref(), Some("https://api.openai.com/v1"));
    }

    #[test]
    fn endpoint_trailing_slash_normalized() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "trailing-slash".to_string(),
            backend_type: BackendType::Local,
            protocol: ProviderProtocol::Vllm,
            endpoint: Some("http://localhost:8000/v1/".to_string()),
            api_key: None,
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();
        let got = store.get("trailing-slash").unwrap().unwrap();
        assert_eq!(got.endpoint.as_deref(), Some("http://localhost:8000/v1"));
    }

    #[test]
    fn ollama_endpoint_not_normalized() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "ollama-local".to_string(),
            backend_type: BackendType::Local,
            protocol: ProviderProtocol::Ollama,
            endpoint: Some("http://localhost:11434".to_string()),
            api_key: None,
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();
        let got = store.get("ollama-local").unwrap().unwrap();
        // Ollama does not use /v1 paths — endpoint should remain unchanged.
        assert_eq!(got.endpoint.as_deref(), Some("http://localhost:11434"));
    }

    #[test]
    fn anthropic_endpoint_normalized_with_v1() {
        let tmp = TempDir::new().unwrap();
        let store = test_store(tmp.path());

        let backend = BackendConfig {
            name: "anthropic-gw".to_string(),
            backend_type: BackendType::OnPrem,
            protocol: ProviderProtocol::Anthropic,
            endpoint: Some("https://gateway.internal.example.com".to_string()),
            api_key: Some("sk-ant-test".to_string()),
            headers: HashMap::new(),
            models: vec![],
            docker: None,
            auto_tool_choice: None,
        };

        store.add(backend).unwrap();
        let got = store.get("anthropic-gw").unwrap().unwrap();
        assert_eq!(
            got.endpoint.as_deref(),
            Some("https://gateway.internal.example.com/v1")
        );
    }
}
