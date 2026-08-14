//! Canonical graph operation contract materialized into `apxm-core`.
//!
//! `apxm-ais` remains the authoring/codegen source of truth. `apxm-core`
//! exposes the generated downstream contract surface.

mod generated {
    pub mod category {
        include!(concat!(env!("OUT_DIR"), "/apxm_operation_category.rs"));
    }

    pub mod mlir_keywords {
        include!(concat!(env!("OUT_DIR"), "/apxm_operation_mlir_keywords.rs"));
    }

    pub mod definitions {
        include!(concat!(env!("OUT_DIR"), "/apxm_operation_definitions.rs"));
    }

    pub mod validation {
        include!(concat!(env!("OUT_DIR"), "/apxm_operation_validation.rs"));
    }
}

pub use generated::category::OperationCategory;
pub use generated::definitions::{
    AIS_OPERATIONS, AISOperationType, ContextStyle, MlirEmissionSpec, MlirResultType,
    OperationField, OperationLatency, OperationSpec, ReferenceType, SLOT_CARRIED, SLOT_INITIAL,
    SLOT_OUTPUT, STRUCTURAL_OPERAND_SLOTS, SemanticOpKind, StructuralOpKind,
    WIRE_INDEXED_OPERATIONS, get_all_operations, get_operation_spec, operand_slots,
};
pub use generated::validation::{
    ValidationError, has_required_fields, missing_required_fields, validate_operation,
    validate_operation_strict,
};
