//! AIS Operations - Single Source of Truth
//!
//! This module contains the complete specification for all 41 AIS operations
//! (38 public + 1 metadata + 2 internal). Both the compiler and runtime use
//! these definitions to ensure consistent semantics.
//!
//! The `tablegen` submodule generates MLIR TableGen files from these definitions,
//! enabling Rust to be the single source of truth for operation metadata.

mod category;
mod definitions;
pub mod mlir_keywords;
pub mod tablegen;

pub use category::OperationCategory;
pub use definitions::{
    AIS_OPERATIONS, AISOperationType, ContextStyle, MlirEmissionSpec, MlirResultType,
    OperationField, OperationLatency, OperationSpec, ReferenceType, get_all_operations,
    get_operation_spec,
};
