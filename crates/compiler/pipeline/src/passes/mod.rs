//! APXM Compiler Pass System
//!
//! This module contains the pass system that transforms and optimizes
//! intermediate representations of AI operations.
//!
//! # Components
//!
//! - [`PassManager`]: Runs optimization passes
//! - [`build_pipeline`]: Creates compilation pipelines
//! - [`PassMetrics`] / [`PipelineDiagnostics`]: Per-pass timing and change tracking
//! - Registry functions: Pass management

pub mod bind_capability_handlers;
mod manager;
pub mod metrics;
mod pipeline;
pub mod profile;
mod registry;
pub mod capability_binding;
pub mod validate_model_allowlist;

pub use bind_capability_handlers::{
    BIND_CAPABILITY_HANDLERS_PASS_NAME, bind_python_handlers_to_dag, bind_capability_handlers,
};
pub use manager::PassManager;
pub use metrics::{PassMetrics, PipelineDiagnostics};
pub use pipeline::{
    build_pass_list, build_pass_list_with_warn, build_pipeline, is_mlir_pass, resolve_pass_list,
};
pub use profile::{ExecutionProfile, NodeProfile, ProfileError};
pub use registry::{find_pass, get_pass_count, get_pass_info, list_passes};
pub use capability_binding::{
    PythonCapabilityManifestEntry, CAPABILITY_BINDING_PASS_NAME, capability_binding_check, capability_binding_check_dag,
};
pub use validate_model_allowlist::validate_model_allowlist;
