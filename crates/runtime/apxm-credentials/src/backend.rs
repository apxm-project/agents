//! Backend configuration management.
//!
//! This module provides the [`BackendStore`] for managing the APXM backend
//! registry. Backends include deployment type, protocol, model metadata, and
//! Docker configuration for local deployments.

use apxm_backends::llm::{BackendConfig, ModelConfig, normalize_endpoint_for_protocol};
use apxm_core::env::apxm_home;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

const FILE_PERMISSIONS: u32 = 0o600;
const DIR_PERMISSIONS: u32 = 0o700;
const CONFIG_FILENAME: &str = "config.toml";
const CONFIG_KEY_BACKENDS: &str = "backends";
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

#[derive(Debug, Clone, Default)]
struct BackendConfigFile {
    document: toml::value::Table,
    backends: Vec<BackendConfig>,
}

/// Backend configuration store.
///
/// Manages the unified backend configuration at `$APXM_HOME/config.toml`
/// (default `~/.apxm/config.toml`). The home directory is resolved through
/// [`apxm_core::env::apxm_home`], so setting `APXM_HOME` redirects the
/// entire backend roster — this is the multi-instance contract that lets
/// two `apxm-server` processes own distinct backend configurations.
///
/// Stores backend registrations as a hierarchical Backend → Model → Endpoint
/// structure.
pub struct BackendStore {
    config_path: PathBuf,
    dir: PathBuf,
}

impl BackendStore {
    /// Open the backend store at `$APXM_HOME/config.toml`
    /// (default `~/.apxm/config.toml`).
    ///
    /// Routes through [`apxm_core::env::apxm_home`], the workspace's
    /// single source of truth for the global home directory. Project-local
    /// `.apxm/` directories are intentionally ignored: backend credentials
    /// are a per-instance concern, not a per-checkout concern, and the
    /// multi-instance contract is "one `APXM_HOME` per `apxm-server`".
    pub fn open() -> Result<Self, BackendError> {
        let dir = apxm_home();
        let config_path = dir.join(CONFIG_FILENAME);
        Ok(Self { config_path, dir })
    }

    /// Ensure the directory exists with correct permissions.
    fn ensure_dir(&self) -> Result<(), BackendError> {
        if !self.dir.exists() {
            fs::create_dir_all(&self.dir)?;
            fs::set_permissions(&self.dir, fs::Permissions::from_mode(DIR_PERMISSIONS))?;
        }
        // Create .gitignore as a safety net for local machine configuration.
        let gitignore = self.dir.join(".gitignore");
        if !gitignore.exists() {
            fs::write(&gitignore, "config.toml\n")?;
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

    /// Read the config file while preserving sections owned by other APXM layers.
    fn read_file(&self) -> Result<BackendConfigFile, BackendError> {
        self.check_permissions()?;
        if !self.config_path.exists() {
            return Ok(BackendConfigFile::default());
        }
        let contents = fs::read_to_string(&self.config_path)?;
        let document: toml::value::Table = toml::from_str(&contents)?;
        let backends = document
            .get(CONFIG_KEY_BACKENDS)
            .cloned()
            .map(toml::Value::try_into)
            .transpose()?
            .unwrap_or_default();
        Ok(BackendConfigFile { document, backends })
    }

    /// Write the config file atomically without dropping non-backend sections.
    fn write_file(&self, file: &BackendConfigFile) -> Result<(), BackendError> {
        self.ensure_dir()?;
        let mut document = file.document.clone();
        document.insert(
            CONFIG_KEY_BACKENDS.to_string(),
            toml::Value::try_from(&file.backends).map_err(BackendError::Serialize)?,
        );
        let serialized = toml::to_string_pretty(&document)?;
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
        let mut backend = backend;
        normalize_endpoint(&mut backend);
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

    /// Get the path to the config file.
    pub fn path(&self) -> &Path {
        &self.config_path
    }
}

/// Ensure the stored endpoint includes the `/v1` version prefix for protocols
/// that need it (OpenAI, Anthropic, vLLM). This makes the endpoint consistent
/// with the runtime, which builds paths like `${endpoint}/chat/completions`.
fn normalize_endpoint(backend: &mut BackendConfig) {
    if let Some(ref mut url) = backend.endpoint {
        *url = normalize_endpoint_for_protocol(backend.protocol, url);
    }
}

