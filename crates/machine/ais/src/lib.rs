//! # Agent Instruction Set (AIS)
//!
//! This crate provides the **single source of truth** for the five canonical
//! public semantic AIS operations and the closed compiler-emitted structural
//! AIS operation family.

pub mod attrs;
pub mod capabilities;
pub mod chat;
pub mod defaults;
pub mod operations;
pub mod passes;
pub mod types;
pub mod validation;

pub use operations::tablegen::generate_tablegen;
pub use operations::{
    AIS_OPERATIONS, AISOperationType, ARTIFACT_OPERATION_KIND_CASES_FILE,
    ARTIFACT_OPERATION_KIND_ENTRIES_FILE, ContextStyle, MlirEmissionSpec, MlirResultType,
    OP_SPEC_CATALOG_FILE, OP_SPEC_SCHEMA_VERSION, OP_SPEC_VECTORS_FILE,
    OP_SPEC_VECTORS_SCHEMA_VERSION, OperationCategory, OperationField, OperationLatency,
    OperationSpec, SEMANTIC_TABLEGEN_DECLARATIONS_FILE, SLOT_CARRIED, SLOT_INITIAL, SLOT_OUTPUT,
    STRUCTURAL_OPERAND_SLOTS, STRUCTURAL_TABLEGEN_DECLARATIONS_FILE, SemanticOpKind,
    StructuralOpKind, WIRE_INDEXED_OPERATIONS, generate_artifact_operation_kind_cases,
    generate_artifact_operation_kind_entries, generate_op_spec_catalog, generate_op_spec_vectors,
    generate_semantic_tablegen_declarations, generate_structural_tablegen_declarations,
    get_all_operations, get_operation_spec, operand_slots, render_op_spec_files,
};
pub use types::Value;
pub use validation::{
    ValidationError, has_required_fields, missing_required_fields, validate_operation,
    validate_operation_strict,
};

pub use passes::{generate_pass_descriptors, generate_pass_dispatch, generate_passes_tablegen};
