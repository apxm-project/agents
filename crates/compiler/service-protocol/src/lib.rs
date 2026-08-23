//! Native Compilation Service protocol.
//!
//! Handshake, compile requests, diagnostics, artifact commit, cancellation,
//! and idempotency live here. The schema contains no runtime lifecycle type.

use apxm_source_port::{Frontend, PackageSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Only declared protocol version. Unknown versions fail closed.
pub const COMPILATION_PROTOCOL_VERSION: &str = "apxm.compilation.protocol/1";

/// Client-to-service handshake.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompilationHandshake {
    /// Exact protocol version the client speaks.
    pub protocol_version: String,
}

impl CompilationHandshake {
    /// Accept only the frozen version. No downgrade negotiation.
    pub fn admit(&self) -> Result<(), ProtocolError> {
        if self.protocol_version == COMPILATION_PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(ProtocolError::IncompatibleVersion)
        }
    }
}

/// Closed client request set. Runtime methods are not representable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompilationRequest {
    /// Compile an exact package snapshot.
    Compile {
        /// Caller correlation id.
        request_id: String,
        /// Idempotency key for this compile.
        idempotency_key: String,
        /// Snapshot admitted for compilation.
        snapshot: PackageSnapshot,
    },
    /// Cancel an in-flight compile.
    Cancel {
        /// Caller correlation id.
        request_id: String,
        /// Compile request being cancelled.
        target_request_id: String,
    },
}

/// Closed service result set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompilationResult {
    /// Digest-bound artifact committed to the content store.
    ArtifactCommitted {
        /// Matching request id.
        request_id: String,
        /// Artifact content digest.
        artifact_digest: String,
        /// Complete build key used for cache identity.
        build_key: String,
    },
    /// Structured diagnostics; no artifact and no runtime contact.
    Failed {
        /// Matching request id.
        request_id: String,
        /// Human-stable diagnostic code.
        code: String,
    },
    /// Compile cancelled before commit.
    Cancelled {
        /// Matching request id.
        request_id: String,
    },
}

/// Protocol admission errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    /// Client spoke an undeclared protocol version.
    IncompatibleVersion,
    /// Request asked the compiler to execute or accept runtime types.
    RuntimeMethod,
    /// Idempotency key reused with a conflicting snapshot.
    ConflictingIdempotency,
    /// Request correlation or idempotency fields were empty.
    InvalidRequest,
}

/// In-memory Compilation Service peer used by protocol vectors.
#[derive(Default)]
pub struct InMemoryCompilationPeer {
    idempotency: HashMap<String, String>,
}

impl InMemoryCompilationPeer {
    /// Handle one admitted request. Never contacts a runtime.
    pub fn handle(
        &mut self,
        handshake: &CompilationHandshake,
        request: CompilationRequest,
    ) -> Result<CompilationResult, ProtocolError> {
        handshake.admit()?;
        match request {
            CompilationRequest::Compile {
                request_id,
                idempotency_key,
                snapshot,
            } => {
                if request_id.trim().is_empty() || idempotency_key.trim().is_empty() {
                    return Err(ProtocolError::InvalidRequest);
                }
                snapshot
                    .validate()
                    .map_err(|_| ProtocolError::ConflictingIdempotency)?;
                let fingerprint = snapshot.snapshot_digest.clone();
                if let Some(prior) = self.idempotency.get(&idempotency_key)
                    && prior != &fingerprint
                {
                    return Err(ProtocolError::ConflictingIdempotency);
                }
                self.idempotency
                    .insert(idempotency_key, fingerprint.clone());
                Ok(CompilationResult::ArtifactCommitted {
                    request_id,
                    artifact_digest: format!("artifact:{fingerprint}"),
                    build_key: format!("{:?}:{fingerprint}", snapshot.frontend),
                })
            }
            CompilationRequest::Cancel {
                request_id,
                target_request_id,
            } => {
                if request_id.trim().is_empty() || target_request_id.trim().is_empty() {
                    return Err(ProtocolError::InvalidRequest);
                }
                Ok(CompilationResult::Cancelled { request_id })
            }
        }
    }
}

/// Reject a rust frontend selector at the protocol boundary.
#[must_use]
pub fn rust_frontend_selector_is_rejected() -> bool {
    serde_json::from_str::<Frontend>(r#""rust""#).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_source_port::SnapshotContent;

    fn handshake() -> CompilationHandshake {
        CompilationHandshake {
            protocol_version: COMPILATION_PROTOCOL_VERSION.to_owned(),
        }
    }

    fn snapshot() -> PackageSnapshot {
        PackageSnapshot::assemble(
            Frontend::Python,
            "src/agent.py",
            vec![SnapshotContent::from_bytes("src/agent.py", b"print('ok')")],
            None,
            "set",
        )
        .expect("protocol vector snapshot")
    }

    #[test]
    fn unknown_protocol_version_fails_closed() {
        let hs = CompilationHandshake {
            protocol_version: "apxm.compilation.protocol/0".to_owned(),
        };
        assert_eq!(hs.admit(), Err(ProtocolError::IncompatibleVersion));
    }

    #[test]
    fn compile_commits_an_artifact_without_runtime_types() {
        let mut peer = InMemoryCompilationPeer::default();
        let result = peer
            .handle(
                &handshake(),
                CompilationRequest::Compile {
                    request_id: "r1".to_owned(),
                    idempotency_key: "i1".to_owned(),
                    snapshot: snapshot(),
                },
            )
            .unwrap();
        match result {
            CompilationResult::ArtifactCommitted {
                artifact_digest, ..
            } => {
                assert!(artifact_digest.starts_with("artifact:"));
            }
            other => panic!("expected commit, got {other:?}"),
        }
    }

    #[test]
    fn conflicting_idempotency_fails_closed() {
        let mut peer = InMemoryCompilationPeer::default();
        let second = PackageSnapshot::assemble(
            Frontend::Python,
            "src/agent.py",
            vec![SnapshotContent::from_bytes(
                "src/agent.py",
                b"print('other')",
            )],
            None,
            "set",
        )
        .expect("conflicting snapshot");
        peer.handle(
            &handshake(),
            CompilationRequest::Compile {
                request_id: "r1".to_owned(),
                idempotency_key: "same".to_owned(),
                snapshot: snapshot(),
            },
        )
        .unwrap();
        let err = peer
            .handle(
                &handshake(),
                CompilationRequest::Compile {
                    request_id: "r2".to_owned(),
                    idempotency_key: "same".to_owned(),
                    snapshot: second,
                },
            )
            .unwrap_err();
        assert_eq!(err, ProtocolError::ConflictingIdempotency);
    }

    #[test]
    fn rust_frontend_is_not_a_protocol_selector() {
        assert!(rust_frontend_selector_is_rejected());
    }

    #[test]
    fn equivalent_python_and_typescript_snapshots_commit() {
        let mut peer = InMemoryCompilationPeer::default();
        for frontend in [Frontend::Python, Frontend::Typescript] {
            let entry = match frontend {
                Frontend::Python => "src/agent.py",
                Frontend::Typescript => "src/agent.ts",
            };
            let snap = PackageSnapshot::assemble(
                frontend,
                entry,
                vec![SnapshotContent::from_bytes(entry, b"export {}")],
                None,
                "set",
            )
            .expect("frontend snapshot");
            peer.handle(
                &handshake(),
                CompilationRequest::Compile {
                    request_id: format!("{frontend:?}"),
                    idempotency_key: format!("{frontend:?}"),
                    snapshot: snap,
                },
            )
            .unwrap();
        }
    }
}
