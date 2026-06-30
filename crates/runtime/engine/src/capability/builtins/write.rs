use super::{
    canonicalize_path_or_existing_ancestor, canonicalize_policy_path, normalize_path_lexically,
    require_string_arg,
};
use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::CapabilityMetadata,
};
use apxm_core::{constants::capabilities, error::RuntimeError, types::Value};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;

/// Atomic write with backup: writes to a temp file, then renames.
/// If the target file already exists, a backup is created and restored on failure.
async fn atomic_write_with_backup(path: &Path, content: &str) -> std::io::Result<()> {
    let temp_name = format!("{}.apxm_tmp_{}", path.display(), uuid::Uuid::now_v7());
    let temp_path = PathBuf::from(&temp_name);

    // Create backup if file exists
    let backup_path = if path.exists() {
        let backup = PathBuf::from(format!(
            "{}.apxm_bak_{}",
            path.display(),
            uuid::Uuid::now_v7()
        ));
        tokio::fs::copy(path, &backup).await?;
        Some(backup)
    } else {
        None
    };

    // Write content to temp file
    tokio::fs::write(&temp_path, content).await?;

    // Atomic rename
    match tokio::fs::rename(&temp_path, path).await {
        Ok(()) => {
            // Success - clean up backup
            if let Some(backup) = backup_path {
                let _ = tokio::fs::remove_file(&backup).await;
            }
            Ok(())
        }
        Err(e) => {
            // Failure - restore backup, clean temp
            if let Some(backup) = &backup_path {
                let _ = tokio::fs::rename(backup, path).await;
            }
            let _ = tokio::fs::remove_file(&temp_path).await;
            Err(e)
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub blocked_paths: Vec<PathBuf>,
    #[serde(default)]
    pub allowed_paths: Option<Vec<PathBuf>>,
    #[serde(default)]
    pub allowed_extensions: Option<Vec<String>>,
    #[serde(default)]
    pub blocked_extensions: Vec<String>,
    #[serde(default = "super::default_true")]
    pub create_directories: bool,
    #[serde(default = "super::default_true")]
    pub overwrite_existing: bool,
    #[serde(default)]
    pub max_file_size: Option<usize>,
    #[serde(default)]
    pub base_directory: Option<PathBuf>,
}

impl Default for WriteConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            blocked_paths: Vec::new(),
            allowed_paths: None,
            allowed_extensions: None,
            blocked_extensions: Vec::new(),
            create_directories: true,
            overwrite_existing: true,
            max_file_size: None,
            base_directory: None,
        }
    }
}

pub struct WriteCapability {
    metadata: CapabilityMetadata,
    config: WriteConfig,
}

impl WriteCapability {
    pub fn new() -> Self {
        Self::with_config(WriteConfig::default())
    }

    pub fn with_config(config: WriteConfig) -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                apxm_core::constants::capabilities::WRITE,
                "Write content to a file with policy enforcement",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "content": { "type": "string" },
                        "append": { "type": "boolean", "default": false }
                    },
                    "required": ["file_path", "content"]
                }),
            )
            .with_returns("string")
            .with_groups(vec![
                capabilities::groups::FILE.to_string(),
                capabilities::groups::FILE_WRITE.to_string(),
                capabilities::groups::WRITE.to_string(),
            ])
            .with_latency(30),
            config,
        }
    }

    pub fn safe() -> Self {
        Self::with_config(WriteConfig {
            blocked_extensions: vec![
                "exe", "com", "msi", "app", "dmg", "sh", "bat", "ps1", "cmd", "vbs", "dll", "so",
                "dylib", "bin",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            ..Default::default()
        })
    }

    fn resolve_path(&self, raw_path: &str) -> CapabilityResult<PathBuf> {
        let raw_path = Path::new(raw_path);
        let path = if raw_path.is_absolute() {
            raw_path.to_path_buf()
        } else if let Some(base_directory) = &self.config.base_directory {
            base_directory.join(raw_path)
        } else {
            raw_path.to_path_buf()
        };
        let path = normalize_path_lexically(&path);

        if let Some(base_directory) = &self.config.base_directory {
            let base_directory =
                canonicalize_path_or_existing_ancestor(base_directory).map_err(|error| {
                    RuntimeError::Capability {
                        capability: self.metadata.name.clone(),
                        message: format!(
                            "Failed to resolve base_directory '{}': {error}",
                            base_directory.display()
                        ),
                    }
                })?;
            let policy_path = canonicalize_path_or_existing_ancestor(&path).map_err(|error| {
                RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!("Failed to resolve path '{}': {error}", path.display()),
                }
            })?;
            if !policy_path.starts_with(&base_directory) {
                return Err(RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!(
                        "Path '{}' is outside base_directory '{}'",
                        policy_path.display(),
                        base_directory.display()
                    ),
                });
            }
        }

        Ok(path)
    }

    fn validate_path_and_content(
        &self,
        path: &Path,
        content_len: usize,
        append: bool,
    ) -> CapabilityResult<()> {
        let policy_path = canonicalize_policy_path(path, &self.metadata.name, "requested")?;
        if let Some(blocked_path) = self
            .config
            .blocked_paths
            .iter()
            .map(|blocked| {
                canonicalize_policy_path(blocked.as_path(), &self.metadata.name, "blocked")
                    .map(|canonical| (blocked.clone(), canonical))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .find(|(_, blocked)| policy_path.starts_with(blocked.as_path()))
        {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!("Path blocked by policy: {}", blocked_path.0.display()),
            });
        }

        if let Some(allowed_paths) = &self.config.allowed_paths
            && !allowed_paths
                .iter()
                .map(|allowed| {
                    canonicalize_policy_path(allowed.as_path(), &self.metadata.name, "allowed")
                })
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .any(|allowed| policy_path.starts_with(allowed.as_path()))
        {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!("Path not in allowed list: {}", policy_path.display()),
            });
        }

        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();

        if self
            .config
            .blocked_extensions
            .iter()
            .any(|blocked| blocked == extension)
        {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!("Extension '{}' is blocked", extension),
            });
        }

        if let Some(allowed_extensions) = &self.config.allowed_extensions
            && !allowed_extensions
                .iter()
                .any(|allowed| allowed == extension)
        {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!(
                    "Extension '{}' is not allowed. Allowed: {}",
                    extension,
                    allowed_extensions.join(", ")
                ),
            });
        }

        let effective_len = if append {
            std::fs::metadata(path)
                .map(|metadata| metadata.len() as usize)
                .unwrap_or(0)
                .saturating_add(content_len)
        } else {
            content_len
        };

        if let Some(max_file_size) = self.config.max_file_size
            && effective_len > max_file_size
        {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!(
                    "Content too large ({} bytes > {} bytes)",
                    effective_len, max_file_size
                ),
            });
        }

        Ok(())
    }
}

impl Default for WriteCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for WriteCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let file_path = require_string_arg(&args, "file_path", &self.metadata.name)?;
        let content = args
            .get("content")
            .and_then(|value| value.as_string())
            .ok_or_else(|| RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: "Missing required 'content' argument".to_string(),
            })?;
        let append = args
            .get("append")
            .and_then(|value| value.as_boolean())
            .unwrap_or(false);

        let path = self.resolve_path(file_path)?;
        self.validate_path_and_content(&path, content.len(), append)?;

        if !self.config.overwrite_existing && !append && path.exists() {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: "File already exists and overwrite_existing=false".to_string(),
            });
        }

        if self.config.create_directories
            && let Some(parent) = path.parent()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!(
                        "Failed to create parent directory '{}': {error}",
                        parent.display()
                    ),
                })?;
        }

        if append {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .map_err(|error| RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!("Failed to open file '{}': {error}", path.display()),
                })?;
            file.write_all(content.as_bytes())
                .await
                .map_err(|error| RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!("Failed to append file '{}': {error}", path.display()),
                })?;
        } else {
            atomic_write_with_backup(&path, content)
                .await
                .map_err(|error| RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!("Failed to write file '{}': {error}", path.display()),
                })?;
        }

        Ok(Value::String(format!(
            "Wrote {} bytes to {}",
            content.len(),
            path.display()
        )))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::{WriteCapability, WriteConfig};

    #[test]
    fn configured_base_directory_rejects_escape_paths() {
        let dir = tempfile::tempdir().unwrap();
        let cap = WriteCapability::with_config(WriteConfig {
            base_directory: Some(dir.path().to_path_buf()),
            ..Default::default()
        });

        let error = cap.resolve_path("../escape.txt").unwrap_err();
        assert!(
            error.to_string().contains("outside base_directory"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn configured_base_directory_allows_child_paths() {
        let dir = tempfile::tempdir().unwrap();
        let cap = WriteCapability::with_config(WriteConfig {
            base_directory: Some(dir.path().to_path_buf()),
            ..Default::default()
        });

        let path = cap.resolve_path("nested/output.txt").unwrap();
        assert!(path.starts_with(dir.path()));
    }
}
