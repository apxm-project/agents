//! Filesystem owner-local ExecutionCommitPort.
//!
//! One directory holds one atomic JSON store. Writes use temp-file + rename
//! so a crash cannot leave a partial authoritative record. There is no network
//! listener and no remote replication — restart durability is directory-local.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use apxm_kernel::{
    ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult, ProgramInstanceRef,
};
use async_trait::async_trait;
use serde_json::Value;

use crate::store::{COMMIT_LOCAL_SCHEMA, CommitLocalError, CommitLocalStore};

/// Single-writer filesystem commit adapter for owner-local conformance.
pub struct FilesystemExecutionCommit {
    root: PathBuf,
    store: Mutex<CommitLocalStore>,
}

impl FilesystemExecutionCommit {
    /// Open or create an owner-local store under `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, CommitLocalError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| CommitLocalError::Io(e.to_string()))?;
        let store = load_or_init(&root)?;
        Ok(Self {
            root,
            store: Mutex::new(store),
        })
    }

    /// Failure-injection hook used by owner-local conformance suites.
    pub fn inject_outcome_unknown(
        &self,
        commit_id: impl Into<String>,
    ) -> Result<(), CommitLocalError> {
        let mut guard = self.store.lock().expect("commit-local filesystem lock");
        guard.inject_outcome_unknown(commit_id);
        persist(&self.root, &guard)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[async_trait]
impl ExecutionCommitPort for FilesystemExecutionCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        let mut guard = self.store.lock().expect("commit-local filesystem lock");
        // Mutate a staged copy so a failed persist cannot leave a partial
        // authoritative in-memory write set.
        let mut staged = guard.clone();
        let result = match staged.commit(&request) {
            Ok(result) => result,
            Err(err) => {
                return ExecutionCommitResult::OutcomeUnknown {
                    reconciliation_ref: format!("reconcile:error:{err}"),
                };
            }
        };
        if let Err(err) = persist(&self.root, &staged) {
            return ExecutionCommitResult::OutcomeUnknown {
                reconciliation_ref: format!("reconcile:persist:{err}"),
            };
        }
        *guard = staged;
        result
    }

    async fn current_version(&self, program_instance_ref: &ProgramInstanceRef) -> u64 {
        self.store
            .lock()
            .expect("commit-local filesystem lock")
            .current_version(program_instance_ref)
    }

    async fn load_continuation(&self, program_instance_ref: &ProgramInstanceRef) -> Option<Value> {
        self.store
            .lock()
            .expect("commit-local filesystem lock")
            .load_continuation(program_instance_ref)
    }
}

fn store_path(root: &Path) -> PathBuf {
    root.join("execution-commit-local.v1.json")
}

fn load_or_init(root: &Path) -> Result<CommitLocalStore, CommitLocalError> {
    let path = store_path(root);
    if !path.exists() {
        let store = CommitLocalStore::new();
        persist(root, &store)?;
        return Ok(store);
    }
    let mut file = File::open(&path).map_err(|e| CommitLocalError::Io(e.to_string()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| CommitLocalError::Io(e.to_string()))?;
    let store: CommitLocalStore =
        serde_json::from_slice(&bytes).map_err(|e| CommitLocalError::Codec(e.to_string()))?;
    store.validate_schema()?;
    Ok(store)
}

fn persist(root: &Path, store: &CommitLocalStore) -> Result<(), CommitLocalError> {
    store.validate_schema()?;
    if store.schema_version != COMMIT_LOCAL_SCHEMA {
        return Err(CommitLocalError::SchemaMismatch {
            found: store.schema_version.clone(),
        });
    }
    let path = store_path(root);
    let tmp = root.join(format!(
        "execution-commit-local.v1.{}.tmp",
        std::process::id()
    ));
    let bytes =
        serde_json::to_vec_pretty(store).map_err(|e| CommitLocalError::Codec(e.to_string()))?;
    {
        let mut file = File::create(&tmp).map_err(|e| CommitLocalError::Io(e.to_string()))?;
        file.write_all(&bytes)
            .map_err(|e| CommitLocalError::Io(e.to_string()))?;
        file.sync_all()
            .map_err(|e| CommitLocalError::Io(e.to_string()))?;
    }
    fs::rename(&tmp, &path).map_err(|e| CommitLocalError::Io(e.to_string()))?;
    // Best-effort directory sync for crash durability on POSIX.
    if let Ok(dir) = File::open(root) {
        let _ = dir.sync_all();
    }
    Ok(())
}
