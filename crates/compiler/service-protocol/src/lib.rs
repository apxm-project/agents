//! Native Compilation Service protocol.
//!
//! Handshake, compile requests, diagnostics, artifact commit, cancellation,
//! and idempotency live here. The schema contains no runtime lifecycle type.

use apxm_source_port::{Frontend, PackageSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

pub use apxm_program::source_map::Span;
pub use apxm_source_port::{
    COMPILE_DIAGNOSTICS_SCHEMA, CompileDiagnostic, DiagnosticReport, DiagnosticReportVersion,
    Location, MAX_FIELD_PATH, MAX_MESSAGE_BYTES, MAX_RELATED, MAX_REPORT_ITEMS, Phase, Related,
    Severity, TypeFact,
};

/// Only declared protocol version. Unknown versions fail closed.
pub const COMPILATION_PROTOCOL_VERSION: &str = "apxm.compilation.protocol/2";

/// The primary code a failed compile reports when its report names no error.
pub const COMPILE_FAILED_CODE: &str = "compile_failed";
pub use apxm_program::EXECUTION_LINEAGE_COMPILER_IDENTITY;

/// Derive the opaque lineage reference returned by Compilation Service.
#[must_use]
pub fn execution_lineage_ref(
    source_digest: &str,
    canonical_artifact_digest: &str,
    compiler_identity: &str,
) -> String {
    apxm_program::execution_lineage_ref(source_digest, canonical_artifact_digest, compiler_identity)
}

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
        /// Complete canonical executable-artifact envelope. Its encoded bytes
        /// are admissible by Runtime under `artifact_digest`.
        artifact: Box<apxm_program::ExecutableArtifact>,
        /// Opaque source/AIR/compiler lineage commitment.
        execution_lineage_ref: String,
        /// Complete build key used for cache identity.
        build_key: String,
        /// Warnings and notes raised while compiling. Never an error.
        diagnostics: DiagnosticReport,
    },
    /// Structured diagnostics; no artifact and no runtime contact.
    Failed {
        /// Matching request id.
        request_id: String,
        /// The primary closed slug: the code of the first error in
        /// `diagnostics`.
        code: String,
        /// Every diagnostic the compile emitted, bounded and ordered.
        diagnostics: DiagnosticReport,
    },
    /// Compile cancelled before commit.
    Cancelled {
        /// Matching request id.
        request_id: String,
    },
}

impl CompilationResult {
    /// A failed compile whose primary code is its report's first error.
    #[must_use]
    pub fn failed(request_id: impl Into<String>, diagnostics: DiagnosticReport) -> Self {
        let code = diagnostics
            .first_error_code()
            .unwrap_or(COMPILE_FAILED_CODE)
            .to_owned();
        Self::Failed {
            request_id: request_id.into(),
            code,
            diagnostics,
        }
    }

    /// A failed compile with one error in `phase`, after which nothing ran.
    #[must_use]
    pub fn failed_with(
        request_id: impl Into<String>,
        code: &str,
        phase: Phase,
        message: impl Into<String>,
    ) -> Self {
        Self::failed(
            request_id,
            DiagnosticReport::from_diagnostics(
                [CompileDiagnostic::new(
                    Severity::Error,
                    code,
                    phase,
                    message,
                )],
                Some(phase),
            ),
        )
    }

    /// Whether this result keeps the protocol's diagnostic invariants: a
    /// committed artifact carries no error and no stopping phase, and a failed
    /// compile's code is its report's first error.
    #[must_use]
    pub fn diagnostics_are_consistent(&self) -> bool {
        match self {
            Self::ArtifactCommitted { diagnostics, .. } => {
                !diagnostics.has_errors() && diagnostics.stopped_at.is_none()
            }
            Self::Failed {
                code, diagnostics, ..
            } => diagnostics.first_error_code() == Some(code.as_str()),
            Self::Cancelled { .. } => true,
        }
    }
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
    artifacts: HashMap<String, apxm_program::ExecutableArtifact>,
}

impl InMemoryCompilationPeer {
    /// Return the canonical envelope committed by this peer.
    #[must_use]
    pub fn artifact(&self, digest: &str) -> Option<&apxm_program::ExecutableArtifact> {
        self.artifacts.get(digest)
    }

    /// Return canonical bytes suitable for Runtime artifact admission.
    pub fn artifact_bytes(&self, digest: &str) -> Option<Vec<u8>> {
        self.artifact(digest)
            .and_then(|artifact| artifact.encode().ok())
    }

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
                let mut artifact = protocol_artifact(&fingerprint)?;
                let artifact_digest = artifact
                    .canonical_digest()
                    .map_err(|_| ProtocolError::InvalidRequest)?;
                artifact.artifact_digest.clone_from(&artifact_digest);
                if !artifact.validate().is_accepted() {
                    return Err(ProtocolError::InvalidRequest);
                }
                let execution_lineage_ref = artifact
                    .execution_lineage_ref
                    .clone()
                    .ok_or(ProtocolError::InvalidRequest)?;
                self.artifacts
                    .insert(artifact_digest.clone(), artifact.clone());
                Ok(CompilationResult::ArtifactCommitted {
                    request_id,
                    artifact_digest,
                    artifact: Box::new(artifact),
                    execution_lineage_ref,
                    build_key: format!("{:?}:{fingerprint}", snapshot.frontend),
                    diagnostics: DiagnosticReport::empty(),
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

/// Build a deterministic valid envelope for protocol-only fixtures. The
/// snapshot digest is carried as the source-bundle commitment; the peer does
/// not pretend to implement a frontend compiler.
fn protocol_artifact(
    source_digest: &str,
) -> Result<apxm_program::ExecutableArtifact, ProtocolError> {
    let air: apxm_program::air::AirModule = serde_json::from_value(json!({
        "schema_version": "apxm.air",
        "semantic_operations": [{
            "node_id": "node.await",
            "op": "await.event",
            "parent_region_id": "region.body",
            "execution_order": 0,
            "operands": [{"slot": "event_ref", "value_id": "session-input", "type_ref": "EventRef"}]
        }],
        "structural_ir": [
            {"region_id": "region.body", "kind": "region", "execution_order": 0},
            {"region_id": "region.return", "kind": "return", "parent_region_id": "region.body", "execution_order": 1}
        ],
        "context_flow": [],
        "source_map": {
            "schema_version": "apxm.source-map",
            "source_language": "python",
            "node_spans": [{
                "node_id": "node.await",
                "source_file": "submitted_source.py",
                "span": {"start_line": 3, "start_column": 4, "end_line": 3, "end_column": 38},
                "semantic_annotation": "await_event"
            }],
            "region_annotations": []
        }
    }))
    .map_err(|_| ProtocolError::InvalidRequest)?;
    let mut artifact = apxm_program::ExecutableArtifact::from_air(&air)
        .map_err(|_| ProtocolError::InvalidRequest)?;
    let source_digest = format!("sha256:{source_digest}");
    artifact.source_bundle_digest.clone_from(&source_digest);
    artifact.execution_lineage_ref = Some(apxm_program::execution_lineage_ref(
        &source_digest,
        &artifact.air_digest,
        EXECUTION_LINEAGE_COMPILER_IDENTITY,
    ));
    Ok(artifact)
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
        for version in ["apxm.compilation.protocol/0", "apxm.compilation.protocol/1"] {
            let hs = CompilationHandshake {
                protocol_version: version.to_owned(),
            };
            assert_eq!(hs.admit(), Err(ProtocolError::IncompatibleVersion));
        }
        assert_eq!(handshake().admit(), Ok(()));
    }

    fn located_error() -> CompileDiagnostic {
        CompileDiagnostic {
            location: Some(Location {
                source_file: "submitted_source.ts".to_owned(),
                span: Span {
                    start_line: 4,
                    start_column: 44,
                    end_line: 4,
                    end_column: 61,
                },
            }),
            ..CompileDiagnostic::new(
                Severity::Error,
                "graph_rejected",
                Phase::TypeCheck,
                "the Capability reference 'host:notes.append' is not declared",
            )
        }
    }

    /// The exact wire shape of a failed compile: optional fields are absent
    /// rather than null, and the location reuses the source map's span.
    #[test]
    fn failed_result_wire_shape_is_exact() {
        let result = CompilationResult::failed(
            "r1",
            DiagnosticReport::from_diagnostics([located_error()], Some(Phase::TypeCheck)),
        );
        assert!(result.diagnostics_are_consistent());
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(
            value,
            json!({
                "kind": "failed",
                "request_id": "r1",
                "code": "graph_rejected",
                "diagnostics": {
                    "schema_version": "apxm.compile-diagnostics.v1",
                    "items": [{
                        "severity": "error",
                        "code": "graph_rejected",
                        "phase": "type_check",
                        "message": "the Capability reference 'host:notes.append' is not declared",
                        "location": {
                            "source_file": "submitted_source.ts",
                            "span": {"start_line": 4, "start_column": 44, "end_line": 4, "end_column": 61}
                        }
                    }],
                    "truncated": false,
                    "total_count": 1,
                    "stopped_at": "type_check"
                }
            })
        );
        let decoded: CompilationResult = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, result);
    }

    /// Every optional field round-trips, and nothing outside the closed shape
    /// decodes.
    #[test]
    fn diagnostic_report_is_closed_and_round_trips() {
        let full = CompileDiagnostic {
            node_id: Some("Reviewer.capability_invocation.1".to_owned()),
            field_path: Some(vec![
                "declarations".to_owned(),
                "decl.capability.Notes".to_owned(),
            ]),
            related: Some(vec![Related {
                message: "declared here".to_owned(),
                location: None,
                node_id: Some("Reviewer.capability_invocation.2".to_owned()),
            }]),
            expected: Some(TypeFact {
                type_ref: Some("Review".to_owned()),
                schema_digest: None,
            }),
            actual: Some(TypeFact {
                type_ref: None,
                schema_digest: Some(format!("sha256:{}", "a".repeat(64))),
            }),
            ..located_error()
        };
        let report = DiagnosticReport::from_diagnostics([full], Some(Phase::Lowering));
        let text = serde_json::to_string(&report).unwrap();
        assert_eq!(
            serde_json::from_str::<DiagnosticReport>(&text).unwrap(),
            report
        );

        let mut value = serde_json::to_value(&report).unwrap();
        value["items"][0]["extra"] = json!(true);
        assert!(serde_json::from_value::<DiagnosticReport>(value).is_err());
        let mut value = serde_json::to_value(&report).unwrap();
        value["schema_version"] = json!("apxm.compile-diagnostics");
        assert!(serde_json::from_value::<DiagnosticReport>(value).is_err());
        let mut value = serde_json::to_value(&report).unwrap();
        value["items"][0]["phase"] = json!("execution");
        assert!(serde_json::from_value::<DiagnosticReport>(value).is_err());
        let mut value = serde_json::to_value(&report).unwrap();
        value["items"][0]["location"]["span"]["line"] = json!(1);
        assert!(serde_json::from_value::<DiagnosticReport>(value).is_err());
    }

    /// A report keeps the first 64 items in emission order, counts the rest,
    /// and bounds every message, field path and related list.
    #[test]
    fn diagnostic_report_is_bounded() {
        let items = (0..70).map(|index| CompileDiagnostic {
            field_path: Some((0..20).map(|segment| segment.to_string()).collect()),
            related: Some(
                (0..10)
                    .map(|note| Related {
                        message: note.to_string(),
                        location: None,
                        node_id: None,
                    })
                    .collect(),
            ),
            ..CompileDiagnostic::new(
                Severity::Error,
                format!("code_{index}"),
                Phase::Capture,
                "\u{e9}".repeat(MAX_MESSAGE_BYTES),
            )
        });
        let report = DiagnosticReport::from_diagnostics(items, Some(Phase::Capture));
        assert_eq!(report.items.len(), MAX_REPORT_ITEMS);
        assert!(report.truncated);
        assert_eq!(report.total_count, 70);
        assert_eq!(report.first_error_code(), Some("code_0"));
        assert_eq!(report.items[63].code, "code_63");
        for item in &report.items {
            assert!(item.message.len() <= MAX_MESSAGE_BYTES);
            assert_eq!(item.field_path.as_ref().unwrap().len(), MAX_FIELD_PATH);
            assert_eq!(item.related.as_ref().unwrap().len(), MAX_RELATED);
        }
    }

    /// A committed artifact never carries an error or a stopping phase; a
    /// failed compile's code is its first error.
    #[test]
    fn diagnostic_invariants_are_checked() {
        let committed_with = |diagnostics| CompilationResult::ArtifactCommitted {
            request_id: "r".to_owned(),
            artifact_digest: "sha256:x".to_owned(),
            artifact: Box::new(protocol_artifact("x").unwrap()),
            execution_lineage_ref: "sha256:y".to_owned(),
            build_key: "k".to_owned(),
            diagnostics,
        };
        let warning = CompileDiagnostic::new(
            Severity::Warning,
            "source_warning",
            Phase::TypeCheck,
            "SyntaxWarning",
        );
        assert!(
            committed_with(DiagnosticReport::from_diagnostics([warning], None))
                .diagnostics_are_consistent()
        );
        assert!(
            !committed_with(DiagnosticReport::from_diagnostics([located_error()], None))
                .diagnostics_are_consistent()
        );
        let failed = CompilationResult::failed_with(
            "r",
            "missing_frontend",
            Phase::Package,
            "the package manifest declares no frontend",
        );
        assert!(failed.diagnostics_are_consistent());
        let CompilationResult::Failed {
            code, diagnostics, ..
        } = failed
        else {
            panic!("failed_with builds a failure");
        };
        assert_eq!(code, "missing_frontend");
        assert_eq!(diagnostics.stopped_at, Some(Phase::Package));
        assert_eq!(
            CompilationResult::failed("r", DiagnosticReport::empty()),
            CompilationResult::Failed {
                request_id: "r".to_owned(),
                code: COMPILE_FAILED_CODE.to_owned(),
                diagnostics: DiagnosticReport::empty(),
            }
        );
    }

    /// Every valid published diagnostic vector decodes into the Rust carrier
    /// and re-encodes to the same JSON, so the schema and the type agree.
    #[test]
    fn published_valid_vectors_decode_into_the_carrier() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../contracts/vectors/apxm.compile-diagnostics.v1.json");
        let vectors: Vec<serde_json::Value> =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut valid = 0;
        for vector in vectors {
            if vector["expected_valid"] != json!(true) {
                continue;
            }
            valid += 1;
            let report: DiagnosticReport = serde_json::from_value(vector["input"].clone())
                .unwrap_or_else(|error| panic!("{}: {error}", vector["name"]));
            assert_eq!(serde_json::to_value(&report).unwrap(), vector["input"]);
        }
        assert!(valid >= 2);
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
                artifact_digest,
                artifact,
                execution_lineage_ref,
                ..
            } => {
                assert!(artifact_digest.starts_with("sha256:"));
                assert!(execution_lineage_ref.starts_with("sha256:"));
                let bytes = artifact.encode().expect("canonical artifact bytes");
                assert_eq!(artifact.artifact_digest, artifact_digest);
                // Every semantic operation of the committed artifact is bound
                // to a source span, so a consumer can locate each node id.
                for operation in &artifact.air.semantic_operations {
                    assert!(
                        artifact
                            .source_map
                            .node_spans
                            .iter()
                            .any(|span| span.node_id == operation.node_id),
                        "{} has a node span",
                        operation.node_id
                    );
                }
                assert!(
                    apxm_program::ExecutableArtifact::decode_for_execution(
                        &bytes,
                        &artifact_digest
                    )
                    .is_ok()
                );
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
