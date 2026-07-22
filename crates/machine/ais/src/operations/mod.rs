//! AIS Operations - Single Source of Truth
//!
//! The canonical semantic and structural operation families live in [`definitions`].

mod artifact_wire;
mod category;
mod definitions;
pub mod mlir_keywords;
mod op_spec;
pub mod tablegen;

pub use artifact_wire::{
    ARTIFACT_OPERATION_KIND_CASES_FILE, ARTIFACT_OPERATION_KIND_ENTRIES_FILE,
    generate_artifact_operation_kind_cases, generate_artifact_operation_kind_entries,
};
pub use category::OperationCategory;
pub use definitions::{
    AIS_OPERATIONS, AISOperationType, ContextStyle, MlirEmissionSpec, MlirResultType,
    OperationField, OperationLatency, OperationSpec, ReferenceType, SemanticOpKind,
    StructuralOpKind, WIRE_INDEXED_OPERATIONS, get_all_operations, get_operation_spec,
};
pub use op_spec::{
    OP_SPEC_CATALOG_FILE, OP_SPEC_SCHEMA_VERSION, OP_SPEC_VECTORS_FILE,
    OP_SPEC_VECTORS_SCHEMA_VERSION, generate_op_spec_catalog, generate_op_spec_vectors,
    render_op_spec_files,
};
pub use tablegen::{
    SEMANTIC_TABLEGEN_DECLARATIONS_FILE, STRUCTURAL_TABLEGEN_DECLARATIONS_FILE,
    generate_semantic_tablegen_declarations, generate_structural_tablegen_declarations,
};
