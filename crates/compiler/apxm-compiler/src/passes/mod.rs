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

mod manager;
pub mod metrics;
mod pipeline;
pub mod profile;
mod registry;
pub mod validate_model_allowlist;
pub mod vllm_hints;

pub use manager::PassManager;
pub use metrics::{PassMetrics, PipelineDiagnostics};
pub use pipeline::{build_pass_list, build_pipeline};
pub use profile::{ExecutionProfile, NodeProfile, ProfileError};
pub use registry::{find_pass, get_pass_count, get_pass_info, list_passes};
pub use validate_model_allowlist::validate_model_allowlist;
pub use vllm_hints::{VLLM_HINTS_PASS_NAME, vllm_hints};
