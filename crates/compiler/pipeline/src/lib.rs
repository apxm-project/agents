//! APXM compiler crate for parsing, transforming, and lowering APXM AIR.
//!
//! This crate compiles AI operations into executable artifacts. It provides APIs for
//! parsing, transforming, and generating binary artifacts from AI operation definitions.
//! The MLIR C++ FFI bridge is the crate's unsafe boundary, so unsafe code is
//! enabled explicitly here instead of being hidden behind narrower attributes.
//!
//! # Overview
//!
//! The compiler lowers canonical `apxm.air` modules to the AIS MLIR
//! dialect text through the C++ FFI bridge, then verifies the result.
//!
//! # Components
//!
//! - [`api`]: Interfaces for compiler interaction
//! - [`canonical`]: AIR → AIS MLIR lowering and verification

#![allow(unsafe_code)]

pub mod api;
pub mod canonical;
mod ffi;
pub use api::{Context, Module};
pub use canonical::{LoweringError, lower_air_to_mlir_text, lower_and_verify, verify_mlir_text};

pub use apxm_core::error::compiler::{CompilerError, Result};
pub use apxm_core::types::compiler::{PassCategory, PassInfo};
pub use apxm_core::types::{CodegenOptions, OptimizationLevel, PipelineConfig};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
