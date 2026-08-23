//! Filesystem owner-local ExecutionCommitPort.
//!
//! One directory holds one atomic JSON store. Writes use temp-file + rename
//! so a crash cannot leave a partial authoritative record. There is no network
//! listener and no remote replication — restart durability is directory-local.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use apxm_kernel::{
    CommittedContinuation, ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    ProgramInstanceRef, canonical_json_bytes,
};
use async_trait::async_trait;
use fs2::FileExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::store::{
    COMMIT_LOCAL_SCHEMA, CommitLocalError, CommitLocalStore, MAX_STORE_BYTES, PreparedOutputRef,
    SessionOutputPreparation,
};

/// Single-writer filesystem commit adapter for owner-local conformance.
pub struct FilesystemExecutionCommit {
    root: PathBuf,
    _lock_file: File,
    auth_key: [u8; 32],
    store: Mutex<CommitLocalStore>,
}

impl FilesystemExecutionCommit {
    /// Open or create an owner-local store under `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, CommitLocalError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| CommitLocalError::Io(e.to_string()))?;
        verify_store_root(&root)?;
        let lock_file = acquire_lock(&root)?;
        if !auth_key_path(&root).exists() && legacy_store_path(&root).exists() {
            return Err(CommitLocalError::SchemaMismatch {
                found: "apxm.execution-commit-local.v1".to_string(),
            });
        }
        let auth_key = load_or_create_auth_key(&root)?;
        let store = load_or_init(&root, &auth_key)?;
        Ok(Self {
            root,
            _lock_file: lock_file,
            auth_key,
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
        persist(&self.root, &guard, &self.auth_key)
    }

    pub fn prepare_output(
        &self,
        preparation: SessionOutputPreparation,
    ) -> Result<PreparedOutputRef, CommitLocalError> {
        let mut guard = self.store.lock().expect("commit-local filesystem lock");
        let mut staged = guard.clone();
        let prepared = staged.prepare_output(preparation)?;
        persist(&self.root, &staged, &self.auth_key)?;
        *guard = staged;
        Ok(prepared)
    }

    pub fn read_output(&self, output_ref: &str) -> Option<Vec<u8>> {
        self.store
            .lock()
            .expect("commit-local filesystem lock")
            .read_output(output_ref)
    }

    pub fn reclaim_prepared_output(&self, output_ref: &str) -> Result<bool, CommitLocalError> {
        let mut guard = self.store.lock().expect("commit-local filesystem lock");
        let mut staged = guard.clone();
        let reclaimed = staged.reclaim_prepared_output(output_ref)?;
        if reclaimed {
            persist(&self.root, &staged, &self.auth_key)?;
            *guard = staged;
        }
        Ok(reclaimed)
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
        if let Err(err) = persist(&self.root, &staged, &self.auth_key) {
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

    async fn load_continuation_with_integrity(
        &self,
        program_instance_ref: &ProgramInstanceRef,
    ) -> Option<CommittedContinuation> {
        self.store
            .lock()
            .expect("commit-local filesystem lock")
            .load_continuation_with_integrity(program_instance_ref)
    }
}

fn store_path(root: &Path) -> PathBuf {
    root.join("execution-commit-local.v2.json")
}

fn legacy_store_path(root: &Path) -> PathBuf {
    root.join("execution-commit-local.v1.json")
}

fn lock_path(root: &Path) -> PathBuf {
    root.join("execution-commit-local.v2.lock")
}

fn auth_key_path(root: &Path) -> PathBuf {
    root.join("execution-commit-local.v2.key")
}

fn acquire_lock(root: &Path) -> Result<File, CommitLocalError> {
    reject_symlink(&lock_path(root), "lock file")?;
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path(root))
        .map_err(|e| CommitLocalError::Io(e.to_string()))?;
    lock_file.try_lock_exclusive().map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            CommitLocalError::OwnershipContended
        } else {
            CommitLocalError::Io(error.to_string())
        }
    })?;
    Ok(lock_file)
}

fn load_or_create_auth_key(root: &Path) -> Result<[u8; 32], CommitLocalError> {
    let path = auth_key_path(root);
    reject_symlink(&path, "auth key")?;
    if path.exists() {
        let bytes = fs::read(&path).map_err(|e| CommitLocalError::Io(e.to_string()))?;
        let key = <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
            CommitLocalError::AuthenticationFailed("local auth key has the wrong length".into())
        })?;
        return Ok(key);
    }
    if store_path(root).exists() || legacy_store_path(root).exists() {
        if let Ok(metadata) = fs::metadata(store_path(root)) {
            let max_store_bytes_u64 = u64::try_from(MAX_STORE_BYTES).unwrap_or(u64::MAX);
            if metadata.len() > max_store_bytes_u64 {
                return Err(CommitLocalError::StoreTooLarge {
                    bytes: metadata.len(),
                });
            }
        }
        return Err(CommitLocalError::AuthenticationFailed(
            "local auth key is missing".into(),
        ));
    }
    let mut key = [0_u8; 32];
    key[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| CommitLocalError::Io(e.to_string()))?;
    file.write_all(&key)
        .and_then(|()| file.sync_all())
        .map_err(|e| CommitLocalError::Io(e.to_string()))?;
    restrict_permissions(&file)?;
    Ok(key)
}

fn restrict_permissions(file: &File) -> Result<(), CommitLocalError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| CommitLocalError::Io(e.to_string()))?;
    }
    Ok(())
}

fn load_or_init(root: &Path, auth_key: &[u8; 32]) -> Result<CommitLocalStore, CommitLocalError> {
    let path = store_path(root);
    reject_symlink(&path, "store")?;
    reject_symlink(&legacy_store_path(root), "legacy store")?;
    if !path.exists() {
        if legacy_store_path(root).exists() {
            return Err(CommitLocalError::SchemaMismatch {
                found: "apxm.execution-commit-local.v1".to_string(),
            });
        }
        let store = CommitLocalStore::new();
        persist(root, &store, auth_key)?;
        return Ok(store);
    }
    let file = File::open(&path).map_err(|e| CommitLocalError::Io(e.to_string()))?;
    let file_size = file
        .metadata()
        .map_err(|e| CommitLocalError::Io(e.to_string()))?
        .len();
    let max_store_bytes_u64 = u64::try_from(MAX_STORE_BYTES).unwrap_or(u64::MAX);
    if file_size > max_store_bytes_u64 {
        return Err(CommitLocalError::StoreTooLarge { bytes: file_size });
    }
    let mut bytes = Vec::new();
    file.take(max_store_bytes_u64.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| CommitLocalError::Io(e.to_string()))?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err(CommitLocalError::StoreTooLarge {
            bytes: bytes.len() as u64,
        });
    }
    let store: CommitLocalStore =
        serde_json::from_slice(&bytes).map_err(|e| CommitLocalError::Codec(e.to_string()))?;
    store.validate_schema()?;
    verify_store_auth(&store, auth_key)?;
    Ok(store)
}

fn persist(
    root: &Path,
    store: &CommitLocalStore,
    auth_key: &[u8; 32],
) -> Result<(), CommitLocalError> {
    store.validate_schema()?;
    if store.schema_version != COMMIT_LOCAL_SCHEMA {
        return Err(CommitLocalError::SchemaMismatch {
            found: store.schema_version.clone(),
        });
    }
    let path = store_path(root);
    // The temporary path is deliberately unpredictable and created with
    // `create_new`. A predictable `File::create` would follow an attacker-
    // planted symlink before the final atomic rename, turning a local store
    // write into an arbitrary-file overwrite.
    let tmp = root.join(format!(
        "execution-commit-local.v2.{}.{}.tmp",
        std::process::id(),
        Uuid::new_v4()
    ));
    let mut authenticated = store.clone();
    authenticated.integrity_tag.clear();
    let body =
        serde_json::to_value(&authenticated).map_err(|e| CommitLocalError::Codec(e.to_string()))?;
    authenticated.integrity_tag = keyed_digest(auth_key, &canonical_json_bytes(&body));
    let bytes = serde_json::to_vec_pretty(&authenticated)
        .map_err(|e| CommitLocalError::Codec(e.to_string()))?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err(CommitLocalError::StoreTooLarge {
            bytes: bytes.len() as u64,
        });
    }
    {
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&tmp)
            .map_err(|e| CommitLocalError::Io(e.to_string()))?;
        restrict_permissions(&file)?;
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

fn reject_symlink(path: &Path, label: &str) -> Result<(), CommitLocalError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                Err(CommitLocalError::AuthenticationFailed(format!(
                    "local {label} path '{}' must not be a symlink",
                    path.display()
                )))
            } else {
                Ok(())
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CommitLocalError::Io(error.to_string())),
    }
}

fn verify_store_root(root: &Path) -> Result<(), CommitLocalError> {
    let metadata =
        fs::symlink_metadata(root).map_err(|error| CommitLocalError::Io(error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CommitLocalError::AuthenticationFailed(format!(
            "local store root '{}' must be a real directory",
            root.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o022 != 0 {
            return Err(CommitLocalError::AuthenticationFailed(format!(
                "local store root '{}' is group/world writable",
                root.display()
            )));
        }
    }
    Ok(())
}

fn verify_store_auth(
    store: &CommitLocalStore,
    auth_key: &[u8; 32],
) -> Result<(), CommitLocalError> {
    if store.integrity_tag.is_empty() {
        return Err(CommitLocalError::AuthenticationFailed(
            "local store has no integrity tag".into(),
        ));
    }
    let mut unauthenticated = store.clone();
    let actual = std::mem::take(&mut unauthenticated.integrity_tag);
    let body = serde_json::to_value(&unauthenticated)
        .map_err(|e| CommitLocalError::Codec(e.to_string()))?;
    let expected = keyed_digest(auth_key, &canonical_json_bytes(&body));
    if actual != expected {
        return Err(CommitLocalError::AuthenticationFailed(
            "local store integrity tag mismatch".into(),
        ));
    }
    Ok(())
}

/// HMAC-SHA-256 without adding another crypto dependency to the owner-local
/// adapter. The key is generated once outside the JSON record and kept at
/// owner-local permissions, so edits to the record alone fail verification.
fn keyed_digest(key: &[u8; 32], bytes: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut inner = [0x36_u8; BLOCK];
    let mut outer = [0x5c_u8; BLOCK];
    for (index, value) in key.iter().enumerate() {
        inner[index] ^= value;
        outer[index] ^= value;
    }
    let mut inner_hash = Sha256::new();
    inner_hash.update(inner);
    inner_hash.update(bytes);
    let inner_digest = inner_hash.finalize();
    let mut outer_hash = Sha256::new();
    outer_hash.update(outer);
    outer_hash.update(inner_digest);
    format!("hmac-sha256:{:x}", outer_hash.finalize())
}
