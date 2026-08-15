//! Compilation Service Composition Root.
//!
//! Binds package snapshot validation, frontend selection, and artifact commit.
//! It does not depend on runtime execution.

mod stdio;

pub use stdio::{StdioFrame, decode_jsonl, encode_jsonl, handshake_cross_wired, serve_stdio};

use std::collections::BTreeMap;

use apxm_compilation_protocol::{
    CompilationHandshake, CompilationRequest, CompilationResult, InMemoryCompilationPeer,
    ProtocolError,
};

/// In-memory artifact store used to prove commit versus crash reconciliation.
#[derive(Default)]
pub struct ArtifactStore {
    committed: BTreeMap<String, String>,
}

impl ArtifactStore {
    /// Record a committed digest. Uncertain commits never appear here.
    pub fn commit(&mut self, digest: String, bytes: String) {
        self.committed.insert(digest, bytes);
    }

    /// Look up a previously committed artifact.
    #[must_use]
    pub fn get(&self, digest: &str) -> Option<&str> {
        self.committed.get(digest).map(String::as_str)
    }
}

/// Compilation Service handler over the native protocol.
#[derive(Default)]
pub struct CompilationService {
    peer: InMemoryCompilationPeer,
    store: ArtifactStore,
}

impl CompilationService {
    /// Admit a handshake and request. Runtime methods are unrepresentable.
    pub fn handle(
        &mut self,
        handshake: &CompilationHandshake,
        request: CompilationRequest,
    ) -> Result<CompilationResult, ProtocolError> {
        let result = self.peer.handle(handshake, request)?;
        if let CompilationResult::ArtifactCommitted {
            artifact_digest, ..
        } = &result
        {
            self.store
                .commit(artifact_digest.clone(), artifact_digest.clone());
        }
        Ok(result)
    }

    /// Committed artifacts only. Failed compiles have no store entry.
    #[must_use]
    pub fn store(&self) -> &ArtifactStore {
        &self.store
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_compilation_protocol::{COMPILATION_PROTOCOL_VERSION, CompilationRequest};
    use apxm_source_port::{Frontend, PACKAGE_SNAPSHOT_CONTRACT, PackageSnapshot, SnapshotContent};

    fn handshake() -> CompilationHandshake {
        CompilationHandshake {
            protocol_version: COMPILATION_PROTOCOL_VERSION.to_owned(),
        }
    }

    fn snapshot(frontend: Frontend, digest: &str) -> PackageSnapshot {
        PackageSnapshot {
            contract: PACKAGE_SNAPSHOT_CONTRACT.to_owned(),
            frontend,
            entrypoint: "src/agent.py".to_owned(),
            contents: vec![SnapshotContent {
                path: "src/agent.py".to_owned(),
                digest: digest.to_owned(),
            }],
            dependency_lock_digest: None,
            compatibility_set: "set".to_owned(),
            snapshot_digest: digest.to_owned(),
        }
    }

    #[test]
    fn python_and_typescript_commit_through_one_handler() {
        let mut service = CompilationService::default();
        for (frontend, digest) in [(Frontend::Python, "py"), (Frontend::Typescript, "ts")] {
            let result = service
                .handle(
                    &handshake(),
                    CompilationRequest::Compile {
                        request_id: digest.to_owned(),
                        idempotency_key: digest.to_owned(),
                        snapshot: snapshot(frontend, digest),
                    },
                )
                .unwrap();
            let CompilationResult::ArtifactCommitted {
                artifact_digest, ..
            } = result
            else {
                panic!("commit");
            };
            assert!(service.store().get(&artifact_digest).is_some());
        }
    }

    #[test]
    fn failed_handshake_never_contacts_the_store() {
        let mut service = CompilationService::default();
        let err = service
            .handle(
                &CompilationHandshake {
                    protocol_version: "nope".to_owned(),
                },
                CompilationRequest::Cancel {
                    request_id: "r".to_owned(),
                    target_request_id: "t".to_owned(),
                },
            )
            .unwrap_err();
        assert_eq!(err, ProtocolError::IncompatibleVersion);
        assert!(service.store().get("artifact:x").is_none());
    }
}
