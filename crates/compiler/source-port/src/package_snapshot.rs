//! Exact package snapshot identity for Compilation Service admission.
//!
//! A snapshot is the validated, digest-bound package the Compilation Service
//! compiles. It is not a live directory, not a FrontendGraph, and not an
//! executable artifact. The manifest selector is the only frontend choice;
//! there is no `rust` selector and no caller-controlled fallback.

use serde::{Deserialize, Serialize};

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

/// One recognized snapshot member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotContent {
    /// Package-relative POSIX path.
    pub path: String,
    /// Content digest of that path's bytes.
    pub digest: String,
}

impl PackageSnapshot {
    /// Reject unknown contracts, empty digests, and a rust frontend spelling.
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
        Ok(())
    }
}

/// Why a snapshot cannot be compiled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// Contract name is not the frozen snapshot identity.
    UnsupportedContract,
    /// Required identity fields are missing.
    Incomplete,
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedContract => {
                formatter.write_str("unsupported package snapshot contract")
            }
            Self::Incomplete => formatter.write_str("package snapshot is incomplete"),
        }
    }
}

impl std::error::Error for SnapshotError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(frontend: Frontend) -> PackageSnapshot {
        PackageSnapshot {
            contract: PACKAGE_SNAPSHOT_CONTRACT.to_owned(),
            frontend,
            entrypoint: "src/agent.py".to_owned(),
            contents: vec![SnapshotContent {
                path: "src/agent.py".to_owned(),
                digest: "abc".to_owned(),
            }],
            dependency_lock_digest: None,
            compatibility_set: "apxm.compatibility-set/test".to_owned(),
            snapshot_digest: "snap".to_owned(),
        }
    }

    #[test]
    fn python_and_typescript_snapshots_validate() {
        snapshot(Frontend::Python).validate().unwrap();
        snapshot(Frontend::Typescript).validate().unwrap();
    }

    #[test]
    fn unknown_snapshot_contract_fails_closed() {
        let mut value = snapshot(Frontend::Python);
        value.contract = "apxm.package-snapshot/legacy".to_owned();
        assert_eq!(value.validate(), Err(SnapshotError::UnsupportedContract));
    }
}
