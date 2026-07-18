//! Durable session-bound AAM checkpoints.
//!
//! A [`SessionManager`] owns exactly one checkpoint location: the trusted
//! session directory supplied by the runtime host. The stored envelope binds
//! the snapshot to both the canonical session and execution identities, so a
//! checkpoint cannot be restored into a different execution after restart.

use std::path::{Path, PathBuf};

use apxm_core::error::RuntimeError;
use serde::{Deserialize, Serialize};

use super::{AamCheckpoint, atomic_replace_checkpoint};

/// Version of the durable session-checkpoint envelope.
pub const SESSION_CHECKPOINT_VERSION: u16 = 1;

/// Canonical filename for the one AAM checkpoint stored in a session directory.
pub const SESSION_CHECKPOINT_FILE_NAME: &str = "aam-checkpoint.v1.json";

/// A durable AAM snapshot bound to one host-authoritative execution identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    /// Envelope version, incremented only for an intentional incompatible format change.
    pub version: u16,
    /// Canonical session identity supplied by the host.
    pub session_id: String,
    /// Canonical execution identity supplied by the host.
    pub execution_id: String,
    /// Complete AAM state captured at the durable boundary.
    pub aam: AamCheckpoint,
}

impl SessionCheckpoint {
    /// Construct a checkpoint only for complete host identities.
    pub fn new(
        session_id: impl Into<String>,
        execution_id: impl Into<String>,
        aam: AamCheckpoint,
    ) -> Result<Self, RuntimeError> {
        let checkpoint = Self {
            version: SESSION_CHECKPOINT_VERSION,
            session_id: session_id.into(),
            execution_id: execution_id.into(),
            aam,
        };
        checkpoint.validate_shape()?;
        Ok(checkpoint)
    }

    /// Validate this checkpoint before restoring its state.
    pub fn validate_for(&self, session_id: &str, execution_id: &str) -> Result<(), RuntimeError> {
        self.validate_shape()?;
        if self.session_id != session_id {
            return Err(RuntimeError::State(
                "session checkpoint identity does not match the requested session".to_string(),
            ));
        }
        if self.execution_id != execution_id {
            return Err(RuntimeError::State(
                "session checkpoint identity does not match the requested execution".to_string(),
            ));
        }
        Ok(())
    }

    /// Reject malformed or unsupported checkpoint envelopes before use.
    fn validate_shape(&self) -> Result<(), RuntimeError> {
        if self.version != SESSION_CHECKPOINT_VERSION {
            return Err(RuntimeError::State(format!(
                "unsupported session checkpoint version {}",
                self.version
            )));
        }
        if self.session_id.trim().is_empty() {
            return Err(RuntimeError::State(
                "session checkpoint is missing a session identity".to_string(),
            ));
        }
        if self.execution_id.trim().is_empty() {
            return Err(RuntimeError::State(
                "session checkpoint is missing an execution identity".to_string(),
            ));
        }
        Ok(())
    }
}

/// Persists the one AAM checkpoint associated with a trusted session directory.
pub struct SessionManager {
    session_dir: PathBuf,
}

impl SessionManager {
    /// Create a checkpoint manager rooted at one host-provided session directory.
    pub fn new(session_dir: impl Into<PathBuf>) -> Result<Self, RuntimeError> {
        let session_dir = session_dir.into();
        std::fs::create_dir_all(&session_dir).map_err(|error| {
            RuntimeError::State(format!(
                "failed to create session checkpoint directory {}: {error}",
                session_dir.display(),
            ))
        })?;
        Ok(Self { session_dir })
    }

    /// Atomically persist a complete identity-bound checkpoint.
    pub fn save_checkpoint(&self, checkpoint: &SessionCheckpoint) -> Result<(), RuntimeError> {
        checkpoint.validate_shape()?;
        let payload = serde_json::to_vec_pretty(checkpoint)
            .map_err(|error| RuntimeError::Serialization(error.to_string()))?;
        atomic_replace_checkpoint(&self.checkpoint_path(), &payload)
    }

    /// Load the checkpoint if the trusted session directory contains one.
    pub fn load_checkpoint(&self) -> Result<Option<SessionCheckpoint>, RuntimeError> {
        let path = self.checkpoint_path();
        if !path.exists() {
            return Ok(None);
        }
        let payload = std::fs::read(&path).map_err(|error| {
            RuntimeError::State(format!(
                "failed to read session checkpoint {}: {error}",
                path.display()
            ))
        })?;
        let checkpoint = serde_json::from_slice::<SessionCheckpoint>(&payload)
            .map_err(|error| RuntimeError::Serialization(error.to_string()))?;
        checkpoint.validate_shape()?;
        Ok(Some(checkpoint))
    }

    /// Delete the current session's checkpoint and durably record its removal.
    pub fn delete_checkpoint(&self) -> Result<(), RuntimeError> {
        let path = self.checkpoint_path();
        if path.exists() {
            std::fs::remove_file(&path).map_err(|error| {
                RuntimeError::State(format!(
                    "failed to delete session checkpoint {}: {error}",
                    path.display(),
                ))
            })?;
            sync_directory(&self.session_dir)?;
        }
        Ok(())
    }

    /// Return the canonical checkpoint location without exposing identity in a path.
    pub fn checkpoint_path(&self) -> PathBuf {
        self.session_dir.join(SESSION_CHECKPOINT_FILE_NAME)
    }
}

/// Durably flush a directory update such as checkpoint activation or deletion.
fn sync_directory(path: &Path) -> Result<(), RuntimeError> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            RuntimeError::State(format!(
                "failed to sync session checkpoint directory {}: {error}",
                path.display(),
            ))
        })
}

/// Restart-continuity coverage for durable AAM session checkpoints.
#[cfg(test)]
mod tests {
    use super::{SessionCheckpoint, SessionManager};
    use crate::{Aam, TransitionLabel};
    use apxm_core::types::values::Value;

    /// Produce an isolated filesystem location for one checkpoint test.
    fn temporary_directory(test_name: &str) -> std::path::PathBuf {
        let unique = format!(
            "{test_name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos(),
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn saved_checkpoint_restores_aam_state_after_a_fresh_process_boundary() {
        let directory = temporary_directory("apxm-aam-checkpoint-restart");
        let manager = SessionManager::new(directory.clone()).expect("checkpoint manager");
        let source = Aam::new();
        source.set_belief(
            "durable-belief".to_string(),
            Value::String("preserved".to_string()),
            TransitionLabel::custom("restart-regression"),
        );

        manager
            .save_checkpoint(
                &SessionCheckpoint::new("session-1", "execution-1", source.checkpoint())
                    .expect("complete checkpoint"),
            )
            .expect("persist checkpoint");

        let restored = Aam::new();
        let checkpoint = manager
            .load_checkpoint()
            .expect("load checkpoint")
            .expect("checkpoint exists");
        checkpoint
            .validate_for("session-1", "execution-1")
            .expect("matching identity");
        restored.restore(&checkpoint.aam);

        assert_eq!(
            restored.get_belief("durable-belief"),
            Some(Value::String("preserved".to_string())),
        );

        std::fs::remove_dir_all(directory).expect("remove temporary checkpoint directory");
    }

    #[test]
    fn checkpoint_rejects_mismatched_execution_identity() {
        let checkpoint =
            SessionCheckpoint::new("session-1", "execution-1", Aam::new().checkpoint())
                .expect("complete checkpoint");

        let error = checkpoint
            .validate_for("session-1", "execution-2")
            .expect_err("wrong execution must not restore");

        assert!(error.to_string().contains("requested execution"));
    }
}
