use crate::require_string_arg;
use apxm_core::{error::RuntimeError, types::Value};
use apxm_runtime::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::CapabilityMetadata,
};
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

/// Multi-file transactional write. All writes succeed or all are rolled back.
pub struct FileTransaction {
    pending: Vec<PendingWrite>,
    backups: Vec<(PathBuf, PathBuf)>,
}

struct PendingWrite {
    temp_path: PathBuf,
    final_path: PathBuf,
}

impl Default for FileTransaction {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTransaction {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            backups: Vec::new(),
        }
    }

    /// Stage a write. Creates the temp file immediately but doesn't rename.
    pub async fn add_write(&mut self, path: PathBuf, content: &str) -> std::io::Result<()> {
        // Create parent dirs if needed
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let temp_path = PathBuf::from(format!(
            "{}.apxm_tmp_{}",
            path.display(),
            uuid::Uuid::now_v7()
        ));

        // Write content to temp
        tokio::fs::write(&temp_path, content).await?;

        // Backup existing file
        if path.exists() {
            let backup = PathBuf::from(format!(
                "{}.apxm_bak_{}",
                path.display(),
                uuid::Uuid::now_v7()
            ));
            if let Err(e) = tokio::fs::copy(&path, &backup).await {
                // Clean up the temp file we already wrote
                let _ = tokio::fs::remove_file(&temp_path).await;
                return Err(e);
            }
            self.backups.push((path.clone(), backup));
        }

        self.pending.push(PendingWrite {
            temp_path,
            final_path: path,
        });
        Ok(())
    }

    /// Commit all staged writes atomically.
    pub async fn commit(self) -> std::io::Result<()> {
        // Rename all temps to finals
        for (i, write) in self.pending.iter().enumerate() {
            if let Err(e) = tokio::fs::rename(&write.temp_path, &write.final_path).await {
                // Rollback already-committed writes
                for committed in &self.pending[..i] {
                    if let Some((_, backup)) = self
                        .backups
                        .iter()
                        .find(|(orig, _)| orig == &committed.final_path)
                    {
                        let _ = tokio::fs::rename(backup, &committed.final_path).await;
                    } else {
                        let _ = tokio::fs::remove_file(&committed.final_path).await;
                    }
                }
                // Clean remaining temps
                for remaining in &self.pending[i..] {
                    let _ = tokio::fs::remove_file(&remaining.temp_path).await;
                }
                // Clean remaining backups
                for (_, backup) in &self.backups {
                    let _ = tokio::fs::remove_file(backup).await;
                }
                return Err(e);
            }
        }

        // Success - clean up all backups
        for (_, backup) in &self.backups {
            let _ = tokio::fs::remove_file(backup).await;
        }
        Ok(())
    }

    /// Explicitly rollback all staged writes.
    pub async fn rollback(self) {
        // Remove all temp files
        for write in &self.pending {
            let _ = tokio::fs::remove_file(&write.temp_path).await;
        }
        // Remove all backups
        for (_, backup) in &self.backups {
            let _ = tokio::fs::remove_file(backup).await;
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteConfig {
    #[serde(default = "crate::default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub blocked_paths: Vec<PathBuf>,
    #[serde(default)]
    pub allowed_paths: Option<Vec<PathBuf>>,
    #[serde(default)]
    pub allowed_extensions: Option<Vec<String>>,
    #[serde(default)]
    pub blocked_extensions: Vec<String>,
    #[serde(default = "crate::default_true")]
    pub create_directories: bool,
    #[serde(default = "crate::default_true")]
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
                "write",
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

    fn resolve_path(&self, raw_path: &str) -> PathBuf {
        if Path::new(raw_path).is_absolute() {
            return PathBuf::from(raw_path);
        }
        if let Some(base_directory) = &self.config.base_directory {
            return base_directory.join(raw_path);
        }
        PathBuf::from(raw_path)
    }

    fn validate_path_and_content(&self, path: &Path, content_len: usize) -> CapabilityResult<()> {
        if let Some(blocked_path) = self
            .config
            .blocked_paths
            .iter()
            .find(|blocked| path.starts_with(blocked.as_path()))
        {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!("Path blocked by policy: {}", blocked_path.display()),
            });
        }

        if let Some(allowed_paths) = &self.config.allowed_paths
            && !allowed_paths
                .iter()
                .any(|allowed| path.starts_with(allowed.as_path()))
        {
            return Err(RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: format!("Path not in allowed list: {}", path.display()),
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
        let file_path = require_string_arg(&args, "file_path", "path", &self.metadata.name)?;
        let content = args
            .get("content")
            .or_else(|| args.get("arg_content"))
            .or_else(|| args.get("arg1"))
            .and_then(|value| value.as_string())
            .ok_or_else(|| RuntimeError::Capability {
                capability: self.metadata.name.clone(),
                message: "Missing required 'content' argument".to_string(),
            })?;
        let append = args
            .get("append")
            .and_then(|value| value.as_boolean())
            .unwrap_or(false);

        let path = self.resolve_path(file_path);
        self.validate_path_and_content(&path, content.len())?;

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
    use super::*;

    #[tokio::test]
    async fn test_atomic_write_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        atomic_write_with_backup(&path, "hello world")
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "hello world"
        );
        // No temp or backup files remain
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1); // only the target file
    }

    #[tokio::test]
    async fn test_atomic_write_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "original").await.unwrap();
        atomic_write_with_backup(&path, "updated").await.unwrap();
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "updated");
        // No temp or backup files remain
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn test_file_transaction_commit() {
        let dir = tempfile::tempdir().unwrap();
        let mut txn = FileTransaction::new();
        txn.add_write(dir.path().join("a.txt"), "content A")
            .await
            .unwrap();
        txn.add_write(dir.path().join("b.txt"), "content B")
            .await
            .unwrap();
        txn.commit().await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(dir.path().join("a.txt"))
                .await
                .unwrap(),
            "content A"
        );
        assert_eq!(
            tokio::fs::read_to_string(dir.path().join("b.txt"))
                .await
                .unwrap(),
            "content B"
        );
    }

    #[tokio::test]
    async fn test_file_transaction_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let mut txn = FileTransaction::new();
        txn.add_write(dir.path().join("a.txt"), "content")
            .await
            .unwrap();
        txn.rollback().await;
        assert!(!dir.path().join("a.txt").exists());
    }
}
