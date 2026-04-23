//! Compilation pipeline types module.
//!
//! Contains types for compilation stages, optimization levels, and code generation.

mod codegen;
pub mod metadata;
mod optimization;
mod passes;
mod stages;

pub use codegen::CodegenOptions;
pub use metadata::{PassMetadata, find_pass_metadata, list_pass_metadata};
pub use optimization::{OptimizationLevel, OptimizationTarget, PipelineConfig};
pub use passes::{PassCategory, PassInfo};
pub use stages::{CompilationStage, EmitFormat, stage_rank};
