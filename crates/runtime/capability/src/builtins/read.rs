use super::fs_boundary::secure_read_under_root;
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
pub struct ReadConfig {
    #[serde(default = "super::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub blocked_paths: Vec<PathBuf>,
    #[serde(default)]
    pub allowed_paths: Option<Vec<PathBuf>>,
    #[serde(default)]
    pub allowed_extensions: Option<Vec<String>>,
    #[serde(default = "default_max_file_size")]
    pub max_file_size: usize,
    #[serde(default)]
    pub base_directory: Option<PathBuf>,
    #[serde(default = "default_max_lines")]
    pub max_default_lines: usize,
}

fn default_max_file_size() -> usize {
    1024 * 1024
}

fn default_max_lines() -> usize {
    2000
}

impl Default for ReadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            blocked_paths: Vec::new(),
            // A read capability without an explicit root is not safe to
            // register: its caller could otherwise name any readable file on
            // the host. Hosts opt in by supplying `allowed_paths` and/or a
            // `base_directory`.
            allowed_paths: Some(Vec::new()),
            allowed_extensions: None,
            max_file_size: default_max_file_size(),
            base_directory: None,
            max_default_lines: default_max_lines(),
        }
    }
}

pub struct ReadCapability {
    metadata: RuntimeCapability,
    config: ReadConfig,
}

impl ReadCapability {
    pub fn new() -> Self {
        Self::with_config(ReadConfig::default())
    }

    /// Same policy as [`new`] but confined to `base_directory`: relative paths
    /// resolve under it and any resolved path that escapes it is rejected.
    pub fn new_with_base_directory(base_directory: PathBuf) -> Self {
        Self::with_config(ReadConfig {
            allowed_paths: Some(vec![base_directory.clone()]),
            base_directory: Some(base_directory),
            ..Default::default()
        })
    }

    pub fn with_config(config: ReadConfig) -> Self {
        Self {
            metadata: RuntimeCapability::new(
                apxm_core::constants::capabilities::READ,
                "Read file contents with path and extension restrictions",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string" },
                        "offset": { "type": "integer", "minimum": 0 },
                        "limit": { "type": "integer", "minimum": 1 }
                    },
                    "required": ["file_path"]
                }),
            )
            .with_returns("string")
            .with_groups(vec![
                capabilities::groups::FILE.to_string(),
                capabilities::groups::FILE_READ.to_string(),
                capabilities::groups::READ.to_string(),
            ])
            // Reading a file is side-effect-free: mark read-only so it is not
            // gated as a write (path/extension policy still applies).
            .with_read_only()
            .with_latency(35),
            config,
        }
    }

    pub fn source() -> Self {
        let source_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::with_config(ReadConfig {
            allowed_paths: Some(vec![source_root.clone()]),
            base_directory: Some(source_root),
            allowed_extensions: Some(
                vec![
                    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp",
                    "toml", "yaml", "yml", "json", "md", "txt", "sh", "css", "scss", "html", "sql",
                    "graphql", "proto", "xml", "lock", "mod", "sum",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            ),
            blocked_paths: vec![
                PathBuf::from(".env"),
                PathBuf::from(".secret"),
                PathBuf::from("credentials"),
                PathBuf::from(".git/config"),
            ],
            ..Default::default()
        })
    }

    fn resolve_path(&self, raw_path: &str) -> CapabilityResult<PathBuf> {
        let raw_path = raw_path.trim();
        let raw_path = if raw_path.starts_with("Users/")
            || raw_path.starts_with("home/")
            || raw_path.starts_with("var/")
            || raw_path.starts_with("tmp/")
            || raw_path.starts_with("etc/")
            || raw_path.starts_with("opt/")
        {
            PathBuf::from(format!("/{raw_path}"))
        } else {
            PathBuf::from(raw_path)
        };
        let path = if raw_path.is_absolute() {
            raw_path
        } else if let Some(base_directory) = &self.config.base_directory {
            base_directory.join(raw_path)
        } else {
            raw_path
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

    /// Select a configured root using the policy view of the path. The
    /// descriptor boundary re-checks this relationship while opening every
    /// component, so this is routing/policy selection rather than the final
    /// security decision.
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
                message: "No configured filesystem root; refusing ambient host access".to_string(),
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

    fn validate_path(&self, path: &Path) -> CapabilityResult<()> {
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

        if let Some(allowed_extensions) = &self.config.allowed_extensions {
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default();
            if !allowed_extensions
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
        }

        Ok(())
    }
}

impl Default for ReadCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for ReadCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let raw_path = require_string_arg(&args, "file_path", &self.metadata.name)?;

        let path = self.resolve_path(raw_path)?;
        self.validate_path(&path)?;
        let root = self.secure_root_for(&path)?;
        let bytes =
            secure_read_under_root(&root, &path, self.config.max_file_size).map_err(|error| {
                RuntimeError::Capability {
                    capability: self.metadata.name.clone(),
                    message: format!("Unable to securely read file '{}': {error}", path.display()),
                }
            })?;
        let content = String::from_utf8(bytes).map_err(|error| RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message: format!(
                "Unable to read file '{}': invalid UTF-8: {error}",
                path.display()
            ),
        })?;

        let lines = content.lines().collect::<Vec<_>>();
        let offset = args
            .get("offset")
            .and_then(|value| value.as_u64())
            .map_or(0, |value| usize::try_from(value).unwrap_or(usize::MAX));
        let limit = args
            .get("limit")
            .and_then(|value| value.as_u64())
            .map_or(self.config.max_default_lines, |value| {
                usize::try_from(value).unwrap_or(usize::MAX)
            });

        if offset >= lines.len() {
            return Ok(Value::String(String::new()));
        }

        let end = offset.saturating_add(limit).min(lines.len());
        let numbered = lines[offset..end]
            .iter()
            .enumerate()
            .map(|(line_offset, line)| format!("{:>6}\t{}", offset + line_offset + 1, line))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(Value::String(numbered))
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn default_reader_refuses_ambient_host_paths() {
        let file = tempfile::NamedTempFile::new().expect("temporary file");
        std::fs::write(file.path(), "secret").expect("write temporary file");
        let error = ReadCapability::new()
            .execute(HashMap::from([(
                "file_path".to_owned(),
                Value::String(file.path().display().to_string()),
            )]))
            .await
            .expect_err("an unconfigured reader must not read arbitrary host files");
        assert!(format!("{error}").contains("Path not in allowed list"));
    }

    #[tokio::test]
    async fn configured_reader_keeps_legitimate_project_access() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let file = directory.path().join("notes.txt");
        std::fs::write(&file, "hello").expect("write project file");
        let capability = ReadCapability::new_with_base_directory(directory.path().to_path_buf());
        let value = capability
            .execute(HashMap::from([(
                "file_path".to_owned(),
                Value::String("notes.txt".to_owned()),
            )]))
            .await
            .expect("configured project read succeeds");
        assert!(value.as_string().is_some_and(|text| text.contains("hello")));
    }

    #[tokio::test]
    async fn descriptor_read_enforces_bound_after_open() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let file = directory.path().join("large.txt");
        std::fs::write(&file, vec![b'x'; 32]).expect("write file");
        let capability = ReadCapability::with_config(ReadConfig {
            allowed_paths: Some(vec![directory.path().to_path_buf()]),
            base_directory: Some(directory.path().to_path_buf()),
            max_file_size: 8,
            ..Default::default()
        });
        let error = capability
            .execute(HashMap::from([(
                "file_path".to_owned(),
                Value::String("large.txt".to_owned()),
            )]))
            .await
            .expect_err("a file over the descriptor read bound must fail");
        let error = format!("{error}");
        assert!(
            error.contains("securely read") || error.contains("outside base_directory"),
            "unexpected refusal: {error}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn descriptor_read_rejects_symlink_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let outside = tempfile::tempdir().expect("outside directory");
        std::fs::write(outside.path().join("secret.txt"), "secret").expect("outside file");
        symlink(
            outside.path().join("secret.txt"),
            directory.path().join("link.txt"),
        )
        .expect("symlink");
        let capability = ReadCapability::new_with_base_directory(directory.path().to_path_buf());
        let error = capability
            .execute(HashMap::from([(
                "file_path".to_owned(),
                Value::String("link.txt".to_owned()),
            )]))
            .await
            .expect_err("a symlink target must not be read");
        let error = format!("{error}");
        assert!(
            error.contains("securely read") || error.contains("outside base_directory"),
            "unexpected symlink refusal: {error}"
        );
    }
}
