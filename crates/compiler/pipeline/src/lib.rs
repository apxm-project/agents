//! APXM compiler crate for parsing, transforming, and lowering APXM AIR.
//!
//! This crate compiles AI operations into executable artifacts. It provides APIs for
//! parsing, transforming, and generating binary artifacts from AI operation definitions.
//! The MLIR C++ FFI bridge is the crate's unsafe boundary, so unsafe code is
//! enabled explicitly here instead of being hidden behind narrower attributes.
//!
//! # Overview
//!
//! The compiler processes AI operations through several stages:
//! - Parses AI operation definitions (AIS DSL)
//! - Builds intermediate representations (AIS MLIR dialect)
//! - Applies optimization passes
//! - Generates binary artifacts for runtime execution
//!
//! # Components
//!
//! - [`api`]: Interfaces for compiler interaction
//! - [`passes`]: Optimization and transformation passes
//! - [`codegen`]: Artifact generation

#![allow(unsafe_code)]

pub mod air_builder;
mod analysis;
pub mod api;
mod artifact_validation;
pub mod codegen;
mod ffi;
pub mod optimization;
pub mod passes;
pub mod template;
pub mod token_estimate;

pub use air_builder::{
    AirEdge, AirError, AirModule, AirModuleBuilder, AirNode, AirParam, AirProgram, FrontendEdge,
    FrontendGraph, FrontendGraphError, FrontendNode, FrontendParameter,
};
pub use analysis::{
    ActiveGrantEvidence, AnalysisNodeKey, ApprovalEvidence, BackendCapabilityEvidence,
    CompilerAnalysisInputs, ConfiguredBackendEvidence, ExecutionReadinessEvidence,
    ProfileCostEvidence, TokenizerEvidence,
};
pub use api::{Context, Module, Pipeline};
pub use passes::{
    ExecutionProfile, NodeProfile, PassManager, PassMetrics, PipelineDiagnostics, ProfileError,
    SelectBackendError, find_pass, get_pass_count, get_pass_info, list_passes,
    parse_backend_catalog_toml, select_backend, select_backend_from_toml,
};

pub use apxm_core::error::compiler::{CompilerError, Result};
pub use apxm_core::types::compiler::{PassCategory, PassInfo};
pub use apxm_core::types::{CodegenOptions, OptimizationLevel, PipelineConfig};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
