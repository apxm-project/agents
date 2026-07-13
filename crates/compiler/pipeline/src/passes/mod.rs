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
pub mod capability_binding;
mod manager;
pub mod metrics;
mod pipeline;
pub mod plan;
pub mod profile;
mod registry;
pub mod select_backend;
pub mod validate_model_allowlist;

pub use bind_capability_handlers::{
    BIND_CAPABILITY_HANDLERS_PASS_NAME, bind_capability_handlers, bind_python_handlers_to_dag,
};
pub use capability_binding::{
    CAPABILITY_BINDING_PASS_NAME, capability_binding_check, capability_binding_check_dag,
};
pub use manager::PassManager;
pub use metrics::{
    ConvergenceDiagnostics, ConvergenceStatus, PassMetrics, PipelineDiagnostics,
    PipelineStageStatus,
};
pub(crate) use pipeline::mandatory_artifact_stages;
pub use pipeline::{
    build_pass_list, build_pass_list_with_warn, build_pipeline, build_pipeline_plan,
    build_pipeline_plan_with_warn, is_mlir_pass, resolve_pass_list, resolve_pipeline_plan,
    stage_kind_for_name,
};
pub use plan::{
    O3_MAX_CLEANUP_ITERATIONS, PipelineConvergenceGroup, PipelinePlan, PipelinePlanStep,
    PipelineStage, PipelineStageKind,
};
pub use profile::{ExecutionProfile, NodeProfile, ProfileError};
pub use registry::{find_pass, get_pass_count, get_pass_info, list_passes};
pub use select_backend::{
    SelectBackendError, parse_backend_catalog_toml, select_backend, select_backend_from_toml,
};
pub use validate_model_allowlist::validate_model_allowlist;
