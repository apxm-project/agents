//! Exact package snapshot identity for Compilation Service admission.
//!
//! A snapshot is the validated, digest-bound package the Compilation Service
//! compiles. It is not a live directory, not a FrontendGraph, and not an
//! executable artifact. The manifest selector is the only frontend choice;
//! there is no `rust` selector and no caller-controlled fallback.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::frontend::Frontend;

/// Wire identity of one complete package snapshot.
pub const PACKAGE_SNAPSHOT_CONTRACT: &str = "apxm.package-snapshot";

/// Validated package contents the Compilation Service may compile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSnapshot {
    /// Closed snapshot contract name. Unknown names fail closed.
    pub contract: String,
    /// Manifest-declared frontend. Compilation must not infer from extensions.
    pub frontend: Frontend,
    /// Package-relative entrypoint path.
    pub entrypoint: String,
    /// Normalized relative paths included in the snapshot.
    pub contents: Vec<SnapshotContent>,
    /// Exact dependency lock bytes digest, when the package declares one.
    pub dependency_lock_digest: Option<String>,
    /// Compatibility Set identity the package claims.
    pub compatibility_set: String,
    /// Digest over the complete snapshot, not over a single file.
    pub snapshot_digest: String,
}

/// One recognized snapshot member, including the exact file bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotContent {
    /// Package-relative POSIX path.
    pub path: String,
    /// Content digest of that path's bytes.
    pub digest: String,
    /// Exact file bytes. Compilation never reads the live workspace.
    pub bytes: Vec<u8>,
}

impl SnapshotContent {
    /// Bind one path to its bytes and the matching content digest.
    #[must_use]
    pub fn from_bytes(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        Self {
            path: path.into(),
            digest: content_digest(&bytes),
            bytes,
        }
    }
}

impl PackageSnapshot {
    /// Assemble a snapshot whose identity digest matches its contents.
    pub fn assemble(
        frontend: Frontend,
        entrypoint: impl Into<String>,
        mut contents: Vec<SnapshotContent>,
        dependency_lock_digest: Option<String>,
        compatibility_set: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        contents.sort_by(|left, right| left.path.cmp(&right.path));
        let mut snapshot = Self {
            contract: PACKAGE_SNAPSHOT_CONTRACT.to_owned(),
            frontend,
            entrypoint: entrypoint.into(),
            contents,
            dependency_lock_digest,
            compatibility_set: compatibility_set.into(),
            snapshot_digest: String::new(),
        };
        snapshot.snapshot_digest = snapshot_identity_digest(&snapshot);
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Reject unknown contracts, empty identity, unsafe paths, digest drift,
    /// and lock drift.
    pub fn validate(&self) -> Result<(), SnapshotError> {
        if self.contract != PACKAGE_SNAPSHOT_CONTRACT {
            return Err(SnapshotError::UnsupportedContract);
        }
        if self.entrypoint.trim().is_empty() || self.snapshot_digest.trim().is_empty() {
            return Err(SnapshotError::Incomplete);
        }
        if self.contents.is_empty() {
            return Err(SnapshotError::Incomplete);
        }
        if self.compatibility_set.trim().is_empty() {
            return Err(SnapshotError::Incomplete);
        }
        let mut saw_entrypoint = false;
        for content in &self.contents {
            if !is_safe_relative_path(&content.path) {
                return Err(SnapshotError::UnsafePath);
            }
            if content.digest != content_digest(&content.bytes) {
                return Err(SnapshotError::DigestMismatch);
            }
            if content.path == self.entrypoint {
                saw_entrypoint = true;
            }
        }
        if !saw_entrypoint {
            return Err(SnapshotError::MissingEntrypoint);
        }
        if self.snapshot_digest != snapshot_identity_digest(self) {
            return Err(SnapshotError::DigestMismatch);
        }
        validate_lock_digest(self)?;
        Ok(())
    }

    /// Look up one snapshot member by package-relative path.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&SnapshotContent> {
        self.contents.iter().find(|content| content.path == path)
    }
}

/// SHA-256 hex digest of exact file bytes.
#[must_use]
pub fn content_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Digest over the complete snapshot identity, excluding the digest field itself.
#[must_use]
pub fn snapshot_identity_digest(snapshot: &PackageSnapshot) -> String {
    let mut hasher = Sha256::new();
    hasher.update(snapshot.contract.as_bytes());
    hasher.update([0]);
    hasher.update(snapshot.frontend.wire().as_bytes());
    hasher.update([0]);
    hasher.update(snapshot.entrypoint.as_bytes());
    hasher.update([0]);
    hasher.update(snapshot.compatibility_set.as_bytes());
    hasher.update([0]);
    if let Some(lock) = &snapshot.dependency_lock_digest {
        hasher.update(lock.as_bytes());
    }
    hasher.update([0]);
    let mut contents = snapshot.contents.clone();
    contents.sort_by(|left, right| left.path.cmp(&right.path));
    for content in contents {
        hasher.update(content.path.as_bytes());
        hasher.update([0]);
        hasher.update(content.digest.as_bytes());
        hasher.update([0]);
        hasher.update(&content.bytes);
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return false;
    }
    !path
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
}

fn is_lock_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "uv.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "Cargo.lock"
            | "poetry.lock"
    ) || name.ends_with(".lock")
}

fn validate_lock_digest(snapshot: &PackageSnapshot) -> Result<(), SnapshotError> {
    let locks: Vec<&SnapshotContent> = snapshot
        .contents
        .iter()
        .filter(|content| is_lock_path(&content.path))
        .collect();
    match (locks.as_slice(), snapshot.dependency_lock_digest.as_deref()) {
        ([], None) => Ok(()),
        ([], Some(_)) | ([_, ..], None) => Err(SnapshotError::LockDrift),
        (locks, Some(declared)) => {
            if locks.iter().any(|content| content.digest == declared) {
                Ok(())
            } else {
                Err(SnapshotError::LockDrift)
            }
        }
    }
}

/// Why a snapshot cannot be compiled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// Contract name is not the frozen snapshot identity.
    UnsupportedContract,
    /// Required identity fields are missing.
    Incomplete,
    /// A path is absolute, parent-escaping, or otherwise unsafe.
    UnsafePath,
    /// A content or snapshot digest does not match the bound bytes.
    DigestMismatch,
    /// Declared lock digest and lock file bytes disagree, or one side is missing.
    LockDrift,
    /// The declared entrypoint is not a snapshot member.
    MissingEntrypoint,
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedContract => {
                formatter.write_str("unsupported package snapshot contract")
            }
            Self::Incomplete => formatter.write_str("package snapshot is incomplete"),
            Self::UnsafePath => formatter.write_str("package snapshot path is unsafe"),
            Self::DigestMismatch => formatter.write_str("package snapshot digest does not match"),
            Self::LockDrift => formatter.write_str("package snapshot lock digest drifted"),
            Self::MissingEntrypoint => {
                formatter.write_str("package snapshot is missing its entrypoint")
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(frontend: Frontend) -> PackageSnapshot {
        PackageSnapshot::assemble(
            frontend,
            "src/agent.py",
            vec![SnapshotContent::from_bytes(
                "src/agent.py",
                b"print('ok')".to_vec(),
            )],
            None,
            "apxm.compatibility-set/test",
        )
        .unwrap()
    }

    #[test]
    fn python_and_typescript_snapshots_validate() {
        snapshot(Frontend::Python).validate().unwrap();
        PackageSnapshot::assemble(
            Frontend::Typescript,
            "src/agent.ts",
            vec![SnapshotContent::from_bytes("src/agent.ts", b"export {}")],
            None,
            "apxm.compatibility-set/test",
        )
        .unwrap()
        .validate()
        .unwrap();
    }

    #[test]
    fn unknown_snapshot_contract_fails_closed() {
        let mut value = snapshot(Frontend::Python);
        value.contract = "apxm.package-snapshot/legacy".to_owned();
        assert_eq!(value.validate(), Err(SnapshotError::UnsupportedContract));
    }

    #[test]
    fn content_digest_mismatch_fails_closed() {
        let mut value = snapshot(Frontend::Python);
        value.contents[0].digest = "deadbeef".to_owned();
        value.snapshot_digest = snapshot_identity_digest(&value);
        assert_eq!(value.validate(), Err(SnapshotError::DigestMismatch));
    }

    #[test]
    fn lock_file_without_declared_digest_is_drift() {
        let err = PackageSnapshot::assemble(
            Frontend::Python,
            "src/agent.py",
            vec![
                SnapshotContent::from_bytes("src/agent.py", b"print('ok')"),
                SnapshotContent::from_bytes("uv.lock", b"lock"),
            ],
            None,
            "apxm.compatibility-set/test",
        )
        .unwrap_err();
        assert_eq!(err, SnapshotError::LockDrift);
    }

    #[test]
    fn parent_path_is_rejected() {
        let err = PackageSnapshot::assemble(
            Frontend::Python,
            "../agent.py",
            vec![SnapshotContent::from_bytes("../agent.py", b"x")],
            None,
            "apxm.compatibility-set/test",
        )
        .unwrap_err();
        assert_eq!(err, SnapshotError::UnsafePath);
    }
}
