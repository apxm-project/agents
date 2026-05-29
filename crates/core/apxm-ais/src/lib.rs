//! # Agent Instruction Set (AIS)
//!
//! This crate provides the **single source of truth** for all AIS operation definitions.
//! Both the compiler and runtime depend on this crate to ensure consistent operation
//! semantics across the entire system.
//!
//! ## Operations (44 total)
//!
//! | Category | Operations |
//! |----------|------------|
//! | Metadata | AGENT |
//! | Memory | QMEM, UMEM |
//! | LLM/Reasoning | ASK, THINK, REASON, PLAN, REFLECT, VERIFY |
//! | Tools | INV_TOOL, EXC, PRINT |
//! | Control Flow | JUMP, BRANCH_ON_VALUE, LOOP_START, LOOP_END, RETURN, SWITCH, FLOW_CALL, WORKFLOW_SPAWN, CALL_SKILL |
//! | Synchronization | MERGE, FENCE, WAIT_ALL |
//! | Error Handling | TRY_CATCH, ERR |
//! | Communication | COMMUNICATE, HANDOFF |
//! | Goal/State | UPDATE_GOAL, GUARD, CLAIM, PAUSE, RESUME |
//! | Coordination | DELEGATE, NEGOTIATE, SPAWN_AGENT, SPAWN_TEAM, REGISTER_CAPABILITY, AUTONOMOUS, CHECKPOINT |
//! | Identity | NOP, IDENTITY |
//! | Internal | CONST_STR, YIELD |

pub mod aam;
pub mod attrs;
pub mod capabilities;
pub mod defaults;
pub mod memory;
pub mod operations;
pub mod passes;
pub mod plan;
pub mod types;
pub mod validation;

// Re-export commonly used types
pub use aam::{AAM, Beliefs, Capabilities, Goal, GoalId, GoalStatus, Goals};
pub use memory::MemoryTier;
pub use operations::tablegen::generate_tablegen;
pub use operations::{
    AIS_OPERATIONS, AISOperationType, ARTIFACT_OPERATION_KIND_CASES_FILE,
    ARTIFACT_OPERATION_KIND_ENTRIES_FILE, ContextStyle, MlirEmissionSpec, MlirResultType,
    OperationCategory, OperationField, OperationLatency, OperationSpec, WIRE_INDEXED_OPERATIONS,
    generate_artifact_operation_kind_cases, generate_artifact_operation_kind_entries,
    get_all_operations, get_operation_spec,
};
pub use types::Value;
pub use validation::{
    ValidationError, has_required_fields, missing_required_fields, validate_operation,
    validate_operation_strict,
};

// Re-export pass generation functions (used by build.rs)
pub use passes::{generate_pass_descriptors, generate_pass_dispatch, generate_passes_tablegen};
