//! APXM Compiler API
//!
//! This module provides the public API for the APXM compiler.
//! It contains types for managing compilation state and verified modules.
//!
//! # Components
//!
//! - [`Context`]: Compiler state and configuration
//! - [`Module`]: Compilation units

pub mod context;
pub mod module;

pub use context::Context;
pub use module::Module;
