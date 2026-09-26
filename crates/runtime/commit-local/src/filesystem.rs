//! Filesystem owner-local ExecutionCommitPort.
//!
//! One directory holds one atomic JSON store. Writes use temp-file + rename
//! so a crash cannot leave a partial authoritative record. There is no network
//! listener and no remote replication — restart durability is directory-local.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use apxm_kernel::{
    CommittedContinuation, ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    PreparedSessionOutputRef, ProgramInstanceRef, canonical_json_bytes,
};
use apxm_runtime_protocol::{
    ContentReadResult, ContentRef, ExecutionObservation, ExecutionReadRequest, ExecutionReadResult,
    ReadContext,
};
use async_trait::async_trait;
use fs2::FileExt;
use serde::de::{self, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, value::RawValue};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::store::ReadAccessHook;
use crate::store::{
    CommitLocalError, CommitLocalStore, MAX_STORE_BYTES, PreparedOutputRef,
    SessionOutputPreparation,
};

const STORE_ENVELOPE_FORMAT: &str = "apxm.execution-commit-local.envelope.v1";
const STORE_ENVELOPE_PREFIX: &[u8] =
    b"{\"format\":\"apxm.execution-commit-local.envelope.v1\",\"body\":";
const STORE_ENVELOPE_TAG_PREFIX: &[u8] = b",\"integrity_tag\":\"";
const STORE_ENVELOPE_END: &[u8] = b"\"}";
const STORE_ENVELOPE_AUTH_DOMAIN: &[u8] = b"apxm.execution-commit-local.envelope.v1\0";
const STORE_ENVELOPE_TAG_LEN: usize = "hmac-sha256:".len() + 64;

struct BoundedStoreBody {
    bytes: Vec<u8>,
    max_body_len: usize,
    envelope_overhead: usize,
    first_rejected_total: Option<usize>,
}

impl Write for BoundedStoreBody {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let attempted = self.bytes.len().checked_add(bytes.len());
        if attempted.is_none_or(|length| length > self.max_body_len) {
            self.first_rejected_total = Some(
                attempted
                    .and_then(|length| length.checked_add(self.envelope_overhead))
                    .unwrap_or(usize::MAX),
            );
            return Err(io::Error::other("owner-local store body exceeds its bound"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct StoreEnvelopeProbe<'a> {
    format: Option<&'a str>,
    body: Option<&'a RawValue>,
    integrity_tag: Option<&'a str>,
    saw_format: bool,
    saw_body: bool,
    saw_integrity_tag: bool,
    saw_unknown_field: bool,
}

impl<'de> Deserialize<'de> for StoreEnvelopeProbe<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ProbeVisitor;

        impl<'de> Visitor<'de> for ProbeVisitor {
            type Value = StoreEnvelopeProbe<'de>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an owner-local JSON store object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut probe = StoreEnvelopeProbe::default();
                while let Some(key) = map.next_key::<&str>()? {
                    match key {
                        "format" => {
                            if probe.saw_format {
                                return Err(de::Error::duplicate_field("format"));
                            }
                            probe.saw_format = true;
                            probe.format = Some(map.next_value()?);
                        }
                        "body" => {
                            if probe.saw_body {
                                return Err(de::Error::duplicate_field("body"));
                            }
                            probe.saw_body = true;
                            probe.body = Some(map.next_value()?);
                        }
                        "integrity_tag" => {
                            if probe.saw_integrity_tag {
                                return Err(de::Error::duplicate_field("integrity_tag"));
                            }
                            probe.saw_integrity_tag = true;
                            probe.integrity_tag = Some(map.next_value()?);
                        }
                        _ => {
                            probe.saw_unknown_field = true;
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(probe)
            }
        }

        deserializer.deserialize_map(ProbeVisitor)
    }
}

/// Single-writer filesystem commit adapter for owner-local conformance.
pub struct FilesystemExecutionCommit {
    root: PathBuf,
    _lock_file: File,
    auth_key: [u8; 32],
    store: Mutex<CommitLocalStore>,
    read_hook: Arc<dyn ReadAccessHook>,
}

impl FilesystemExecutionCommit {
    /// Open or create an owner-local store under `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, CommitLocalError> {
        Self::open_with_read_access_hook(root, Arc::new(crate::store::DenyReadAccess))
    }

    /// Open a store with the Composition Root's reauthorization/audit hook.
    pub fn open_with_read_access_hook(
        root: impl Into<PathBuf>,
        read_hook: Arc<dyn ReadAccessHook>,
    ) -> Result<Self, CommitLocalError> {
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
            read_hook,
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

    /// Bind the composition-owned scope to outputs staged by this adapter.
    pub fn set_default_access_scope_ref(&self, reference: String) {
        self.store
            .lock()
            .expect("commit-local filesystem lock")
            .set_default_access_scope_ref(reference);
    }

    /// Read opaque composition metadata from the authenticated store.
    #[must_use]
    pub fn runtime_metadata(&self) -> Option<Value> {
        self.store
            .lock()
            .expect("commit-local filesystem lock")
            .runtime_metadata()
    }

    #[must_use]
    pub fn invocation_status(
        &self,
        invocation: &str,
    ) -> Option<apxm_runtime_protocol::ProgramInvocationStatus> {
        self.store
            .lock()
            .expect("commit-local filesystem lock")
            .invocation_status(invocation)
    }

    #[must_use]
    pub fn terminal_failure_code(&self, invocation: &str) -> Option<String> {
        self.store
            .lock()
            .expect("commit-local filesystem lock")
            .terminal_failure_code(invocation)
    }

    /// Atomically replace opaque composition metadata in the authenticated
    /// store. The adapter does not inspect or interpret the JSON value.
    pub fn set_runtime_metadata(&self, metadata: Option<Value>) -> Result<(), CommitLocalError> {
        let mut guard = self.store.lock().expect("commit-local filesystem lock");
        let previous = guard.replace_runtime_metadata(metadata);
        if let Err(error) = persist(&self.root, &guard, &self.auth_key) {
            guard.replace_runtime_metadata(previous);
            return Err(error);
        }
        Ok(())
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

    /// Read observations, inspection, committed content, or evidence through
    /// the typed scope-bound contract.
    pub fn read_execution(
        &self,
        request: ExecutionReadRequest,
    ) -> Result<ExecutionReadResult, CommitLocalError> {
        self.read_execution_with_live(request, &[])
    }

    /// Read with a bounded process-local live observation overlay.
    pub fn read_execution_with_live(
        &self,
        request: ExecutionReadRequest,
        live_observations: &[ExecutionObservation],
    ) -> Result<ExecutionReadResult, CommitLocalError> {
        self.store
            .lock()
            .expect("commit-local filesystem lock")
            .read_execution_with_live(
                request,
                &self.auth_key,
                self.read_hook.as_ref(),
                live_observations,
            )
    }

    /// Read one committed output with a scope-bound typed context.
    pub fn read_committed_output(
        &self,
        context: ReadContext,
        content_ref: ContentRef,
    ) -> Result<ContentReadResult, CommitLocalError> {
        match self.read_execution(ExecutionReadRequest::ContentRead {
            context,
            content_ref,
        })? {
            ExecutionReadResult::Content { content } => Ok(content),
            _ => Err(CommitLocalError::InvalidRead(
                "content read returned the wrong result kind".into(),
            )),
        }
    }

    /// Read one committed output through the distinct output.read operation.
    pub fn read_output(
        &self,
        context: ReadContext,
        output_ref: apxm_runtime_protocol::OutputRef,
    ) -> Result<ContentReadResult, CommitLocalError> {
        match self.read_execution(ExecutionReadRequest::OutputRead {
            context,
            output_ref,
        })? {
            ExecutionReadResult::Output { output } => Ok(output),
            _ => Err(CommitLocalError::InvalidRead(
                "output read returned the wrong result kind".into(),
            )),
        }
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
    async fn prepare_output(
        &self,
        preparation: apxm_kernel::SessionOutputPreparation,
    ) -> Result<PreparedSessionOutputRef, String> {
        let preparation = SessionOutputPreparation::from_kernel(preparation)
            .map_err(|error| error.to_string())?;
        let prepared = self
            .prepare_output(preparation)
            .map_err(|error| error.to_string())?
            .to_kernel();
        prepared.validate().map_err(str::to_owned)?;
        Ok(prepared)
    }

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
    restrict_permissions(&file)?;
    file.write_all(&key)
        .and_then(|()| file.sync_all())
        .map_err(|e| CommitLocalError::Io(e.to_string()))?;
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
    let probe: StoreEnvelopeProbe<'_> =
        serde_json::from_slice(&bytes).map_err(|e| CommitLocalError::Codec(e.to_string()))?;
    if probe.saw_format || probe.saw_body {
        if probe.saw_unknown_field
            || !probe.saw_format
            || !probe.saw_body
            || !probe.saw_integrity_tag
        {
            return Err(CommitLocalError::AuthenticationFailed(
                "local store envelope has missing or unknown fields".into(),
            ));
        }
        let format = probe.format.expect("validated envelope format presence");
        if format != STORE_ENVELOPE_FORMAT {
            return Err(CommitLocalError::SchemaMismatch {
                found: format.to_owned(),
            });
        }
        let body = probe.body.expect("validated envelope body presence").get();
        let actual = probe
            .integrity_tag
            .expect("validated envelope tag presence");
        let expected = keyed_digest_parts(auth_key, &[STORE_ENVELOPE_AUTH_DOMAIN, body.as_bytes()]);
        if actual != expected {
            return Err(CommitLocalError::AuthenticationFailed(
                "local store envelope integrity tag mismatch".into(),
            ));
        }
        let store: CommitLocalStore =
            serde_json::from_str(body).map_err(|e| CommitLocalError::Codec(e.to_string()))?;
        if !store.integrity_tag.is_empty() {
            return Err(CommitLocalError::AuthenticationFailed(
                "local store envelope body has a legacy integrity tag".into(),
            ));
        }
        store.validate_schema()?;
        return Ok(store);
    }
    let mut store: CommitLocalStore =
        serde_json::from_slice(&bytes).map_err(|e| CommitLocalError::Codec(e.to_string()))?;
    store.validate_schema()?;
    verify_store_auth(&store, auth_key)?;
    store.integrity_tag.clear();
    Ok(store)
}

fn persist(
    root: &Path,
    store: &CommitLocalStore,
    auth_key: &[u8; 32],
) -> Result<(), CommitLocalError> {
    store.validate_schema()?;
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
    if !store.integrity_tag.is_empty() {
        return Err(CommitLocalError::Codec(
            "local store envelope body requires an empty legacy integrity tag".into(),
        ));
    }
    let envelope_overhead = bounded_envelope_size(0, STORE_ENVELOPE_TAG_LEN)?;
    let mut bounded_body = BoundedStoreBody {
        bytes: Vec::new(),
        max_body_len: MAX_STORE_BYTES - envelope_overhead,
        envelope_overhead,
        first_rejected_total: None,
    };
    if let Err(error) = serde_json::to_writer(&mut bounded_body, store) {
        if let Some(bytes) = bounded_body.first_rejected_total {
            return Err(CommitLocalError::StoreTooLarge {
                bytes: u64::try_from(bytes).unwrap_or(u64::MAX),
            });
        }
        return Err(CommitLocalError::Codec(error.to_string()));
    }
    let body = bounded_body.bytes;
    let integrity_tag = keyed_digest_parts(auth_key, &[STORE_ENVELOPE_AUTH_DOMAIN, &body]);
    debug_assert_eq!(integrity_tag.len(), STORE_ENVELOPE_TAG_LEN);
    bounded_envelope_size(body.len(), integrity_tag.len())?;
    {
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&tmp)
            .map_err(|e| CommitLocalError::Io(e.to_string()))?;
        restrict_permissions(&file)?;
        file.write_all(STORE_ENVELOPE_PREFIX)
            .and_then(|()| file.write_all(&body))
            .and_then(|()| file.write_all(STORE_ENVELOPE_TAG_PREFIX))
            .and_then(|()| file.write_all(integrity_tag.as_bytes()))
            .and_then(|()| file.write_all(STORE_ENVELOPE_END))
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

fn bounded_envelope_size(body_len: usize, tag_len: usize) -> Result<usize, CommitLocalError> {
    let total_bytes = STORE_ENVELOPE_PREFIX
        .len()
        .checked_add(body_len)
        .and_then(|size| size.checked_add(STORE_ENVELOPE_TAG_PREFIX.len()))
        .and_then(|size| size.checked_add(tag_len))
        .and_then(|size| size.checked_add(STORE_ENVELOPE_END.len()))
        .ok_or(CommitLocalError::StoreTooLarge { bytes: u64::MAX })?;
    if total_bytes > MAX_STORE_BYTES {
        return Err(CommitLocalError::StoreTooLarge {
            bytes: total_bytes as u64,
        });
    }
    Ok(total_bytes)
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
    let actual = &store.integrity_tag;
    let mut body =
        serde_json::to_value(store).map_err(|e| CommitLocalError::Codec(e.to_string()))?;
    body.as_object_mut()
        .ok_or_else(|| CommitLocalError::Codec("local store is not an object".to_owned()))?
        .insert("integrity_tag".to_owned(), Value::String(String::new()));
    let expected = keyed_digest(auth_key, &canonical_json_bytes(&body));
    if actual != &expected {
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
    keyed_digest_parts(key, &[bytes])
}

fn keyed_digest_parts(key: &[u8; 32], parts: &[&[u8]]) -> String {
    const BLOCK: usize = 64;
    let mut inner = [0x36_u8; BLOCK];
    let mut outer = [0x5c_u8; BLOCK];
    for (index, value) in key.iter().enumerate() {
        inner[index] ^= value;
        outer[index] ^= value;
    }
    let mut inner_hash = Sha256::new();
    inner_hash.update(inner);
    for part in parts {
        inner_hash.update(part);
    }
    let inner_digest = inner_hash.finalize();
    let mut outer_hash = Sha256::new();
    outer_hash.update(outer);
    outer_hash.update(inner_digest);
    format!("hmac-sha256:{:x}", outer_hash.finalize())
}

#[cfg(test)]
mod compact_store_tests {
    use super::*;
    use crate::store::{CommitReplayIdentity, CommitRequestIdentity};
    use apxm_kernel::{
        AtomicWriteSet, ExecutionCommitTuple, ProgramInvocationRef, continuation_digest,
        runtime_evidence_and_observation_digest, session_output_refs_digest,
    };
    use serde_json::json;

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn request() -> ExecutionCommitRequest {
        let continuation = Some(json!({"pc": 1}));
        ExecutionCommitRequest {
            commit_id: "commit.legacy-identity".to_owned(),
            program_instance_ref: ProgramInstanceRef::new("instance.legacy-identity"),
            program_invocation_ref: ProgramInvocationRef::new("invoke.legacy-identity"),
            idempotency_key: "idem.legacy-identity".to_owned(),
            expected_program_state_version: 0,
            write_set: AtomicWriteSet {
                next_program_state_digest: digest('1'),
                continuation_digest: continuation_digest(continuation.as_ref()),
                checkpoint_effect_outcomes_digest: digest('3'),
                runtime_evidence_batch_digest: runtime_evidence_and_observation_digest(&[], &[]),
                usage_facts_digest: digest('5'),
                session_output_refs_digest: session_output_refs_digest(&[]),
            },
            tuple: ExecutionCommitTuple {
                context: json!({"scope": "legacy"}),
                continuation,
                event_wait: None,
                effect_outcomes: vec![],
                evidence: vec![],
                usage: Value::Null,
                output_refs: vec![],
                observations: vec![],
            },
            evidence_batch: vec![],
        }
    }

    #[tokio::test]
    async fn authenticated_legacy_full_identity_reopens_and_replays() {
        let dir = tempfile::tempdir().expect("tempdir");
        let port = FilesystemExecutionCommit::open(dir.path()).expect("open");
        let request = request();
        let first = port.commit(request.clone()).await;
        assert!(matches!(first, ExecutionCommitResult::Committed { .. }));
        let mut old_pretty_bytes = {
            let mut store = port.store.lock().expect("commit-local store lock");
            let retained = store
                .by_commit_scope
                .values_mut()
                .next()
                .expect("commit row");
            retained.request_identity =
                CommitReplayIdentity::LegacyFull(Box::new(CommitRequestIdentity::from(&request)));
            serde_json::to_value(&*store).expect("legacy full store body")
        };
        let legacy_key = port.auth_key;
        drop(port);
        let path = store_path(dir.path());
        assert_eq!(old_pretty_bytes["integrity_tag"], "");
        let old_oracle = serde_json::to_vec(&apxm_kernel::canonical_json_value(&old_pretty_bytes))
            .expect("legacy canonical body");
        assert_eq!(canonical_json_bytes(&old_pretty_bytes), old_oracle);
        let old_tag = keyed_digest(&legacy_key, &old_oracle);
        old_pretty_bytes["integrity_tag"] = Value::String(old_tag);
        fs::write(
            &path,
            serde_json::to_vec_pretty(&old_pretty_bytes).expect("old pretty representation"),
        )
        .expect("write old pretty representation");
        let reopened = FilesystemExecutionCommit::open(dir.path()).expect("authenticate legacy");
        assert_eq!(reopened.commit(request.clone()).await, first);
        let mut changed = request.clone();
        changed.tuple.context = json!({"scope": "foreign"});
        assert!(matches!(
            reopened.commit(changed).await,
            ExecutionCommitResult::OutcomeUnknown { .. }
        ));
        reopened
            .set_runtime_metadata(Some(json!({"migrated": true})))
            .expect("rewrite legacy store as authenticated envelope");
        drop(reopened);
        let rewritten: Value =
            serde_json::from_slice(&fs::read(&path).expect("read rewrite")).expect("envelope JSON");
        assert_eq!(rewritten["format"], STORE_ENVELOPE_FORMAT);
        assert_eq!(rewritten["body"]["integrity_tag"], "");
        let final_reopen = FilesystemExecutionCommit::open(dir.path()).expect("reopen migrated");
        assert_eq!(
            final_reopen.runtime_metadata(),
            Some(json!({"migrated": true}))
        );
        assert_eq!(final_reopen.commit(request).await, first);
    }

    #[test]
    fn envelope_refuses_tamper_unknown_fields_and_ambiguous_formats() {
        let dir = tempfile::tempdir().expect("tempdir");
        let port = FilesystemExecutionCommit::open(dir.path()).expect("open");
        port.set_runtime_metadata(Some(json!({"marker": "original"})))
            .expect("persist metadata");
        let key = port.auth_key;
        drop(port);
        let path = store_path(dir.path());
        let original = fs::read(&path).expect("read envelope");
        let value: Value = serde_json::from_slice(&original).expect("envelope JSON");
        assert_eq!(value["format"], STORE_ENVELOPE_FORMAT);
        assert_eq!(value["body"]["integrity_tag"], "");
        assert!(serde_json::from_slice::<CommitLocalStore>(&original).is_err());

        let mut changed = value.clone();
        changed["body"]["runtime_metadata"]["marker"] = json!("tampered");
        let mut wrong_tag = value.clone();
        wrong_tag["integrity_tag"] = json!("hmac-sha256:00");
        let mut unknown = value.clone();
        unknown["unexpected"] = json!(true);
        let mut mixed = value.clone();
        mixed["schema_version"] = json!("apxm.execution-commit-local.v2");
        let mut missing_format = value.clone();
        missing_format.as_object_mut().unwrap().remove("format");
        let mut null_format = value.clone();
        null_format["format"] = Value::Null;
        let mut wrong_format = value.clone();
        wrong_format["format"] = json!("apxm.execution-commit-local.envelope.v2");
        let duplicate_format = format!(
            "{{\"format\":\"{}\",{}",
            STORE_ENVELOPE_FORMAT,
            std::str::from_utf8(&original)
                .unwrap()
                .strip_prefix('{')
                .unwrap()
        );
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(matches!(
            FilesystemExecutionCommit::open(dir.path()),
            Err(CommitLocalError::AuthenticationFailed(message))
                if message.contains("envelope integrity tag mismatch")
        ));
        for (label, bytes) in [
            ("tag", serde_json::to_vec(&wrong_tag).unwrap()),
            ("unknown", serde_json::to_vec(&unknown).unwrap()),
            ("mixed", serde_json::to_vec(&mixed).unwrap()),
            ("missing", serde_json::to_vec(&missing_format).unwrap()),
            ("null", serde_json::to_vec(&null_format).unwrap()),
            ("version", serde_json::to_vec(&wrong_format).unwrap()),
            ("duplicate", duplicate_format.into_bytes()),
        ] {
            fs::write(&path, bytes).expect("write malformed envelope");
            assert!(
                FilesystemExecutionCommit::open(dir.path()).is_err(),
                "{label} was accepted"
            );
        }

        let mut tagged_body = value.clone();
        tagged_body["body"]["integrity_tag"] = json!("legacy-tag");
        let body = serde_json::to_vec(&tagged_body["body"]).unwrap();
        tagged_body["integrity_tag"] = json!(keyed_digest_parts(
            &key,
            &[STORE_ENVELOPE_AUTH_DOMAIN, &body]
        ));
        fs::write(&path, serde_json::to_vec(&tagged_body).unwrap()).unwrap();
        assert!(matches!(
            FilesystemExecutionCommit::open(dir.path()),
            Err(CommitLocalError::AuthenticationFailed(message))
                if message.contains("legacy integrity tag")
        ));

        fs::write(&path, original).expect("restore exact envelope");
        let reopened = FilesystemExecutionCommit::open(dir.path()).expect("reopen exact envelope");
        assert_eq!(
            reopened.runtime_metadata(),
            Some(json!({"marker": "original"}))
        );
    }

    #[test]
    fn envelope_total_size_includes_header_tag_and_closing_bytes() {
        let tag_len = keyed_digest_parts(&[7_u8; 32], &[STORE_ENVELOPE_AUTH_DOMAIN, b"{}"]).len();
        let overhead = STORE_ENVELOPE_PREFIX.len()
            + STORE_ENVELOPE_TAG_PREFIX.len()
            + tag_len
            + STORE_ENVELOPE_END.len();
        assert_eq!(
            bounded_envelope_size(MAX_STORE_BYTES - overhead, tag_len).unwrap(),
            MAX_STORE_BYTES
        );
        assert!(matches!(
            bounded_envelope_size(MAX_STORE_BYTES - overhead + 1, tag_len),
            Err(CommitLocalError::StoreTooLarge { bytes }) if bytes == (MAX_STORE_BYTES + 1) as u64
        ));
    }

    #[cfg(unix)]
    #[test]
    fn new_auth_key_and_envelope_are_owner_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let port = FilesystemExecutionCommit::open(dir.path()).expect("open");
        for path in [auth_key_path(dir.path()), store_path(dir.path())] {
            assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
        }
        drop(port);
    }

    #[tokio::test]
    async fn oversized_metadata_persist_preserves_prior_authority() {
        let dir = tempfile::tempdir().expect("tempdir");
        let port = FilesystemExecutionCommit::open(dir.path()).expect("open");
        let request = request();
        let committed = port.commit(request.clone()).await;
        assert!(matches!(committed, ExecutionCommitResult::Committed { .. }));
        port.set_runtime_metadata(Some(json!({"state": "before"})))
            .expect("persist initial metadata");
        assert!(matches!(
            port.set_runtime_metadata(Some(json!({"padding": "x".repeat(MAX_STORE_BYTES)}))),
            Err(CommitLocalError::StoreTooLarge { .. })
        ));
        assert_eq!(port.runtime_metadata(), Some(json!({"state": "before"})));
        assert_eq!(port.commit(request.clone()).await, committed);
        drop(port);
        let reopened = FilesystemExecutionCommit::open(dir.path()).expect("reopen prior authority");
        assert_eq!(
            reopened.runtime_metadata(),
            Some(json!({"state": "before"}))
        );
        assert_eq!(reopened.commit(request).await, committed);
        reopened
            .set_runtime_metadata(Some(json!({"state": "after"})))
            .expect("continue after refused metadata persistence");
        drop(reopened);
        let final_reopen =
            FilesystemExecutionCommit::open(dir.path()).expect("reopen later authority");
        assert_eq!(
            final_reopen.runtime_metadata(),
            Some(json!({"state": "after"}))
        );
    }
}
