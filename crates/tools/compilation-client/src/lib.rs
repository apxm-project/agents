//! Compilation Client: snapshot, submit, render diagnostics, return artifact refs.

use apxm_compilation_protocol::{
    COMPILATION_PROTOCOL_VERSION, CompilationHandshake, CompilationRequest, CompilationResult,
};
use apxm_compilation_service::CompilationService;
use apxm_source_port::{Frontend, PACKAGE_SNAPSHOT_CONTRACT, PackageSnapshot, SnapshotContent};
use std::path::Path;

/// Headless build client. Contains no frontend or compiler implementation.
pub struct CompilationClient {
    service: CompilationService,
}

impl Default for CompilationClient {
    fn default() -> Self {
        Self {
            service: CompilationService::default(),
        }
    }
}

impl CompilationClient {
    /// Snapshot a local package directory and compile it. The CLI never
    /// constructs a `PackageSnapshot` itself.
    pub fn build_package(&mut self, package_root: &Path) -> Result<String, String> {
        self.build(PackageSnapshot {
            contract: PACKAGE_SNAPSHOT_CONTRACT.to_owned(),
            frontend: Frontend::Python,
            entrypoint: "agent".to_owned(),
            contents: vec![SnapshotContent {
                path: package_root.display().to_string(),
                digest: "local".to_owned(),
            }],
            dependency_lock_digest: None,
            compatibility_set: "apxm.compatibility-set/local".to_owned(),
            snapshot_digest: package_root.display().to_string(),
        })
    }

    /// Submit one exact snapshot. Failed or uncertain compiles return no digest.
    pub fn build(&mut self, snapshot: PackageSnapshot) -> Result<String, String> {
        let result = self
            .service
            .handle(
                &CompilationHandshake {
                    protocol_version: COMPILATION_PROTOCOL_VERSION.to_owned(),
                },
                CompilationRequest::Compile {
                    request_id: "build".to_owned(),
                    idempotency_key: snapshot.snapshot_digest.clone(),
                    snapshot,
                },
            )
            .map_err(|error| format!("{error:?}"))?;
        match result {
            CompilationResult::ArtifactCommitted {
                artifact_digest, ..
            } => Ok(artifact_digest),
            CompilationResult::Failed { code, .. } => Err(code),
            CompilationResult::Cancelled { .. } => Err("cancelled".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_source_port::{Frontend, PACKAGE_SNAPSHOT_CONTRACT, SnapshotContent};

    #[test]
    fn build_returns_a_committed_digest() {
        let mut client = CompilationClient::default();
        let digest = client
            .build(PackageSnapshot {
                contract: PACKAGE_SNAPSHOT_CONTRACT.to_owned(),
                frontend: Frontend::Python,
                entrypoint: "a.py".to_owned(),
                contents: vec![SnapshotContent {
                    path: "a.py".to_owned(),
                    digest: "d".to_owned(),
                }],
                dependency_lock_digest: None,
                compatibility_set: "set".to_owned(),
                snapshot_digest: "snap".to_owned(),
            })
            .unwrap();
        assert!(digest.starts_with("artifact:"));
    }
}
