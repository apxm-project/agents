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
pub mod diagnostic;
pub mod frontend_graph;
pub mod grammar;
pub mod lower;
pub mod source_map;

pub use air::{AirModule, SemanticOpKind, StructuralKind, verify_air_json};
pub use artifact::{
    ExecutableArtifact, PortRequirement, PortSourceScope, validate_artifact_json,
};
pub use diagnostic::{Diagnostic, DiagnosticCode, Verdict};
pub use frontend_graph::{FrontendGraph, verify_frontend_graph_json};
pub use lower::frontend_graph_to_air;
pub use source_map::{SourceLanguage, SourceMap, verify_source_map_json};
