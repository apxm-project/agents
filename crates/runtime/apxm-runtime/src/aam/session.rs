//! Session-level checkpoint persistence.
//!
//! Provides [`SessionManager`] which saves and loads [`AamCheckpoint`]s to disk,
//! keyed by session ID. This enables process-restart continuity: a session can be
//! checkpointed before shutdown and restored on the next startup.

use std::path::PathBuf;

use apxm_core::error::RuntimeError;

use super::AamCheckpoint;

/// Manages checkpoint persistence on disk, one checkpoint file per session.
///
/// Directory layout:
/// ```text
/// <checkpoint_dir>/
///   <session_id>.json
///   <session_id>.json
///   ...
/// ```
pub struct SessionManager {
    checkpoint_dir: PathBuf,
}

impl SessionManager {
    /// Create a new `SessionManager` rooted at `checkpoint_dir`.
    ///
    /// The directory is created (including parents) if it does not exist.
    pub fn new(checkpoint_dir: PathBuf) -> Result<Self, RuntimeError> {
        std::fs::create_dir_all(&checkpoint_dir).map_err(|e| {
            RuntimeError::State(format!(
                "Failed to create checkpoint directory {}: {}",
                checkpoint_dir.display(),
                e,
            ))
        })?;
        Ok(Self { checkpoint_dir })
    }

    /// Save a checkpoint for the given `(session_id, scope_id)` pair.
    ///
    /// `scope_id` of `None` represents the global (root) scope.
    /// Returns the path the checkpoint was written to.
    pub fn save_checkpoint(
        &self,
        session_id: &str,
        scope_id: Option<&str>,
        checkpoint: &AamCheckpoint,
    ) -> Result<PathBuf, RuntimeError> {
        let path = self.checkpoint_path(session_id, scope_id);
        checkpoint.save_to_file(&path)?;
        Ok(path)
    }

    /// Load the most recent checkpoint for `(session_id, scope_id)`, if one exists.
    ///
    /// `scope_id` of `None` represents the global (root) scope.
    pub fn load_checkpoint(
        &self,
        session_id: &str,
        scope_id: Option<&str>,
    ) -> Result<Option<AamCheckpoint>, RuntimeError> {
        let path = self.checkpoint_path(session_id, scope_id);
        if !path.exists() {
            return Ok(None);
        }
        AamCheckpoint::load_from_file(&path).map(Some)
    }

    /// List all session IDs that have saved checkpoints.
    pub fn list_sessions(&self) -> Result<Vec<String>, RuntimeError> {
        let entries = std::fs::read_dir(&self.checkpoint_dir).map_err(|e| {
            RuntimeError::State(format!(
                "Failed to read checkpoint directory {}: {}",
                self.checkpoint_dir.display(),
                e,
            ))
        })?;

        let mut sessions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| {
                RuntimeError::State(format!("Failed to read directory entry: {}", e))
            })?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    sessions.push(stem.to_string());
                }
            }
        }
        sessions.sort();
        sessions.dedup();
        Ok(sessions)
    }

    /// Delete the checkpoint for the given `(session_id, scope_id)` pair.
    ///
    /// `scope_id` of `None` represents the global (root) scope.
    pub fn delete_checkpoint(
        &self,
        session_id: &str,
        scope_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let path = self.checkpoint_path(session_id, scope_id);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                RuntimeError::State(format!(
                    "Failed to delete checkpoint {}: {}",
                    path.display(),
                    e,
                ))
            })?;
        }
        Ok(())
    }

    /// Return the on-disk path for a session's checkpoint file.
    ///
    /// When `scope_id` is `Some`, the filename encodes both session and scope
    /// as `<session_id>__<scope_id>.json`. When `None`, it uses `<session_id>.json`
    /// (the global scope).
    pub fn checkpoint_path(&self, session_id: &str, scope_id: Option<&str>) -> PathBuf {
        match scope_id {
            Some(scope) => self
                .checkpoint_dir
                .join(format!("{}__{}.json", session_id, scope)),
            None => self.checkpoint_dir.join(format!("{}.json", session_id)),
        }
    }
}

