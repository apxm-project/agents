//! Canonical Agent Program semantic surface.
//!
//! This crate is the first-party consumer of the closed Agent Program semantic
//! schemas — `apxm.frontend-graph.v1`, `apxm.air.v1`, `apxm.source-map.v1`,
//! `apxm.executable-artifact.v1`, and `apxm.port-requirement.v1`. It provides
//! closed Rust types that mirror those schemas exactly, a canonical artifact
//! codec, and deterministic verifiers that fail closed on any value outside the
//! declared closures. It has no provider, runtime, store, or discovery
//! dependency.

pub mod air;
pub mod artifact;
pub mod common;
pub mod diagnostic;
pub mod execution_commit;
pub mod frontend_graph;
pub mod grammar;
pub mod lower;
pub mod runtime_evidence;
pub mod source_map;

pub use air::{AirModule, SemanticOpKind, StructuralKind, verify_air_json};
pub use artifact::{
    ExecutableArtifact, PortRequirement, PortSourceScope, validate_artifact_json,
};
pub use common::{IdempotencyKey, TypedErrorEnvelope, TypedRef};
pub use diagnostic::{Diagnostic, DiagnosticCode, Verdict};
pub use execution_commit::{
    AtomicWriteSetMember, CommitResult, ExecutionCommit, CANONICAL_ATOMIC_WRITE_SET,
    verify_execution_commit_json,
};
pub use frontend_graph::{FrontendGraph, verify_frontend_graph_json};
pub use lower::{frontend_graph_to_air, lower_frontend_graph_json};
pub use runtime_evidence::{
    Fact, FactKind, InstanceState, InvocationState, ModelOutcome, ProgramIdentity, RuntimeEvidence,
    verify_runtime_evidence_json,
};
pub use source_map::{SourceLanguage, SourceMap, verify_source_map_json};
