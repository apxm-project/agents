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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::{CapabilityRecord, GoalTree};
    use apxm_core::types::goal::{Goal, GoalId, GoalStatus};
    use apxm_core::types::values::Value;
    use chrono::Utc;
    use std::collections::HashMap;

    fn sample_checkpoint() -> AamCheckpoint {
        let mut beliefs = HashMap::new();
        beliefs.insert("user".to_string(), Value::String("Alice".to_string()));
        beliefs.insert(
            "count".to_string(),
            Value::Number(apxm_core::types::values::Number::Integer(42)),
        );

        let goal = Goal {
            id: GoalId::new(),
            description: "test-goal".to_string(),
            priority: 80,
            status: GoalStatus::Active,
            parent_id: None,
        };

        let mut capabilities = HashMap::new();
        capabilities.insert(
            "search".to_string(),
            CapabilityRecord {
                name: "search".to_string(),
                description: "search the web".to_string(),
                schema: serde_json::json!({"type": "object"}),
                cost_estimate: 0.5,
            },
        );

        AamCheckpoint {
            beliefs,
            goals: vec![goal],
            capabilities,
            goal_tree: GoalTree::new(),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn checkpoint_save_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_checkpoint.json");

        let original = sample_checkpoint();
        original.save_to_file(&path).unwrap();
        let loaded = AamCheckpoint::load_from_file(&path).unwrap();

        assert_eq!(original.beliefs, loaded.beliefs);
        assert_eq!(original.goals.len(), loaded.goals.len());
        assert_eq!(original.goals[0].id, loaded.goals[0].id);
        assert_eq!(original.goals[0].description, loaded.goals[0].description);
        assert_eq!(original.goals[0].priority, loaded.goals[0].priority);
        assert_eq!(original.goals[0].status, loaded.goals[0].status);
        assert_eq!(original.capabilities.len(), loaded.capabilities.len());
        assert!(loaded.capabilities.contains_key("search"));
    }

    #[test]
    fn checkpoint_load_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let result = AamCheckpoint::load_from_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn session_manager_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

        let checkpoint = sample_checkpoint();
        let saved_path = mgr
            .save_checkpoint("sess-001", None, &checkpoint)
            .unwrap();
        assert!(saved_path.exists());

        let loaded = mgr.load_checkpoint("sess-001", None).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.beliefs, checkpoint.beliefs);
        assert_eq!(loaded.goals.len(), checkpoint.goals.len());
    }

    #[test]
    fn session_manager_save_and_load_with_scope() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

        let checkpoint = sample_checkpoint();
        let saved_path = mgr
            .save_checkpoint("sess-001", Some("scope-a"), &checkpoint)
            .unwrap();
        assert!(saved_path.exists());

        // Loading with scope returns the checkpoint
        let loaded = mgr
            .load_checkpoint("sess-001", Some("scope-a"))
            .unwrap();
        assert!(loaded.is_some());

        // Loading without scope returns None (different key)
        let loaded_global = mgr.load_checkpoint("sess-001", None).unwrap();
        assert!(loaded_global.is_none());
    }

    #[test]
    fn session_manager_load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

        let result = mgr.load_checkpoint("no-such-session", None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn session_manager_list_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

        let checkpoint = sample_checkpoint();
        mgr.save_checkpoint("alpha", None, &checkpoint).unwrap();
        mgr.save_checkpoint("beta", None, &checkpoint).unwrap();
        mgr.save_checkpoint("gamma", None, &checkpoint).unwrap();

        let sessions = mgr.list_sessions().unwrap();
        assert_eq!(sessions, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn session_manager_delete_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

        let checkpoint = sample_checkpoint();
        mgr.save_checkpoint("to-delete", None, &checkpoint)
            .unwrap();
        assert!(mgr
            .load_checkpoint("to-delete", None)
            .unwrap()
            .is_some());

        mgr.delete_checkpoint("to-delete", None).unwrap();
        assert!(mgr
            .load_checkpoint("to-delete", None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn session_manager_delete_nonexistent_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();
        // Deleting a session that doesn't exist should not error.
        mgr.delete_checkpoint("never-existed", None).unwrap();
    }

    #[test]
    fn session_manager_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("checkpoints");
        assert!(!nested.exists());

        let mgr = SessionManager::new(nested.clone()).unwrap();
        assert!(nested.exists());

        // Verify we can actually use it
        let checkpoint = sample_checkpoint();
        mgr.save_checkpoint("sess", None, &checkpoint).unwrap();
        assert!(mgr.load_checkpoint("sess", None).unwrap().is_some());
    }

    #[test]
    fn aam_checkpoint_restore_round_trip_via_session() {
        use crate::aam::{Aam, TransitionLabel};

        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

        // Build an AAM with state
        let aam = Aam::new();
        aam.set_belief(
            "key".into(),
            Value::String("value".into()),
            TransitionLabel::custom("test"),
        );
        let goal = Goal {
            id: GoalId::new(),
            description: "survive".into(),
            priority: 99,
            status: GoalStatus::Active,
            parent_id: None,
        };
        aam.add_goal(goal.clone(), TransitionLabel::custom("test"));

        // Checkpoint, save to disk
        let cp = aam.checkpoint();
        mgr.save_checkpoint("full-round-trip", None, &cp).unwrap();

        // Load into a fresh AAM
        let loaded = mgr
            .load_checkpoint("full-round-trip", None)
            .unwrap()
            .unwrap();
        let aam2 = Aam::new();
        aam2.restore(&loaded);

        assert_eq!(aam2.get_belief("key"), Some(Value::String("value".into())));
        let goals = aam2.goals();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].id, goal.id);
        assert_eq!(goals[0].priority, 99);
    }
}
