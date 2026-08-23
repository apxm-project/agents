use super::fs_boundary::secure_write_under_root;
use super::{
    canonicalize_path_or_existing_ancestor, canonicalize_policy_path, normalize_path_lexically,
    require_string_arg,
};
use crate::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::RuntimeCapability,
};
use apxm_core::{constants::capabilities, error::RuntimeError, types::Value};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

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
            // An unconstrained write capability is ambient host mutation.
            // Hosts must explicitly provide a root (or base_directory).
            allowed_paths: Some(Vec::new()),
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
    metadata: RuntimeCapability,
    config: WriteConfig,
}

impl WriteCapability {
    pub fn new() -> Self {
        Self::with_config(WriteConfig::default())
    }

    pub fn with_config(config: WriteConfig) -> Self {
        Self {
            metadata: RuntimeCapability::new(
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

    pub fn new_with_base_directory(base_directory: PathBuf) -> Self {
        Self::with_config(WriteConfig {
            allowed_paths: Some(vec![base_directory.clone()]),
            base_directory: Some(base_directory),
            ..Default::default()
        })
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
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|error| RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!("Failed to resolve current directory: {error}"),
                })?
                .join(path)
        };

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

    fn secure_root_for(&self, path: &Path) -> CapabilityResult<PathBuf> {
        let mut roots = Vec::new();
        if let Some(base) = &self.config.base_directory {
            roots.push(base.clone());
        }
        if let Some(allowed) = &self.config.allowed_paths {
            roots.extend(allowed.iter().cloned());
        }
        if roots.is_empty() {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: "No configured filesystem root; refusing ambient host mutation"
                    .to_string(),
            });
        }
        let requested = canonicalize_path_or_existing_ancestor(path).map_err(|error| {
            RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!(
                    "Failed to resolve requested path '{}': {error}",
                    path.display()
                ),
            }
        })?;
        roots
            .into_iter()
            .find_map(|root| {
                let canonical = std::fs::canonicalize(&root).ok()?;
                if !requested.starts_with(&canonical) {
                    return None;
                }
                let absolute = if root.is_absolute() {
                    root
                } else {
                    std::env::current_dir().ok()?.join(root)
                };
                Some(absolute)
            })
            .ok_or_else(|| RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!(
                    "Path '{}' is not inside a configured filesystem root",
                    path.display()
                ),
            })
    }

    fn validate_path_and_content(
        &self,
        path: &Path,
        content_len: usize,
        _append: bool,
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

        if let Some(max_file_size) = self.config.max_file_size
            && content_len > max_file_size
        {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!(
                    "Content too large ({} bytes > {} bytes)",
                    content_len, max_file_size
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

        let root = self.secure_root_for(&path)?;
        secure_write_under_root(
            &root,
            &path,
            content.as_bytes(),
            append,
            self.config.create_directories,
            self.config.overwrite_existing,
            self.config.max_file_size,
        )
        .map_err(|error| RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message: format!(
                "Failed to securely write file '{}': {error}",
                path.display()
            ),
        })?;

        Ok(Value::String(format!(
            "Wrote {} bytes to {}",
            content.len(),
            path.display()
        )))
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::{WriteCapability, WriteConfig};
    use crate::executor::CapabilityExecutor;
    use apxm_core::types::Value;
    use std::collections::HashMap;

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

    #[tokio::test]
    async fn default_writer_refuses_ambient_host_mutation() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let path = temporary.path().join("ambient.txt");
        let error = WriteCapability::new()
            .execute(HashMap::from([
                (
                    "file_path".to_owned(),
                    Value::String(path.display().to_string()),
                ),
                ("content".to_owned(), Value::String("secret".to_owned())),
            ]))
            .await
            .expect_err("an unconfigured writer must not mutate host paths");
        assert!(format!("{error}").contains("Path not in allowed list"));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn configured_writer_supports_nested_atomic_write_and_append() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let capability = WriteCapability::new_with_base_directory(temporary.path().to_path_buf());
        let args = |append: bool, content: &str| {
            HashMap::from([
                (
                    "file_path".to_owned(),
                    Value::String("nested/output.txt".to_owned()),
                ),
                ("content".to_owned(), Value::String(content.to_owned())),
                ("append".to_owned(), Value::Bool(append)),
            ])
        };
        capability
            .execute(args(false, "one"))
            .await
            .expect("initial write");
        capability
            .execute(args(true, "-two"))
            .await
            .expect("append write");
        assert_eq!(
            std::fs::read_to_string(temporary.path().join("nested/output.txt")).unwrap(),
            "one-two"
        );
    }

    #[tokio::test]
    async fn no_overwrite_is_published_atomically() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let capability = WriteCapability::with_config(WriteConfig {
            allowed_paths: Some(vec![temporary.path().to_path_buf()]),
            base_directory: Some(temporary.path().to_path_buf()),
            overwrite_existing: false,
            ..Default::default()
        });
        let args = |content: &str| {
            HashMap::from([
                (
                    "file_path".to_owned(),
                    Value::String("output.txt".to_owned()),
                ),
                ("content".to_owned(), Value::String(content.to_owned())),
            ])
        };
        capability
            .execute(args("first"))
            .await
            .expect("initial write");
        let error = capability
            .execute(args("second"))
            .await
            .expect_err("overwrite_existing=false must reject an existing target");
        assert!(format!("{error}").contains("securely write"));
        assert_eq!(
            std::fs::read_to_string(temporary.path().join("output.txt")).unwrap(),
            "first"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn configured_writer_rejects_symlink_target_without_touching_outside() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("temporary directory");
        let outside = tempfile::tempdir().expect("outside directory");
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, "original").expect("outside file");
        let link = temporary.path().join("link.txt");
        symlink(&outside_file, &link).expect("target symlink");
        let capability = WriteCapability::new_with_base_directory(temporary.path().to_path_buf());
        let error = capability
            .execute(HashMap::from([
                ("file_path".to_owned(), Value::String("link.txt".to_owned())),
                ("content".to_owned(), Value::String("overwrite".to_owned())),
            ]))
            .await
            .expect_err("a symlink target must fail closed");
        assert!(
            format!("{error}").contains("outside base_directory")
                || format!("{error}").contains("securely write")
        );
        assert_eq!(std::fs::read_to_string(outside_file).unwrap(), "original");
    }
}
