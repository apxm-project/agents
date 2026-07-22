//! AIS operation types module.
//!
//! Exposes the generated shared operation contract plus runtime-specific definitions.

mod definition;
pub mod metadata;

pub use metadata::{
    AISOperationType, ContextStyle, MlirEmissionSpec, MlirResultType, OperationCategory,
    OperationField, OperationLatency, OperationSpec, ReferenceType, SemanticOpKind,
    StructuralOpKind, ValidationError, WIRE_INDEXED_OPERATIONS, get_all_operations,
    get_operation_spec, validate_operation,
};

// Runtime-specific types
pub use definition::AISOperation;
