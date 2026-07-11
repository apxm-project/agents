//! # Agent Instruction Set (AIS)
//!
//! This crate provides the **single source of truth** for all AIS operation definitions.
//! Both the compiler and runtime depend on this crate to ensure consistent operation
//! semantics across the entire system.
//!
//! ## Operations (39 total)
//!
//! The table below reflects each operation's actual `OperationCategory` (see
//! `operations::category`), not an informal grouping — it is kept honest by
//! `apxm ops list`, which renders operations grouped by this same field.
//!
//! | Category | Operations |
//! |----------|------------|
//! | Metadata | AGENT |
//! | Memory | QMEM, UMEM, UPDATE_GOAL |
//! | Reasoning | ASK, THINK, REASON, PLAN, REFLECT, VERIFY |
//! | Tools | INV_CAP, EXC, PRINT |
//! | Control Flow | JUMP, BRANCH_ON_VALUE, RETURN, SWITCH, FLOW_CALL, WORKFLOW_SPAWN, CALL_SKILL, RESUME |
//! | Synchronization | MERGE, FENCE, WAIT_ALL, CHECKPOINT |
//! | Error Handling | TRY_CATCH, ERR |
//! | Communication | COMMUNICATE, HANDOFF, PAUSE |
//! | Coordination | DELEGATE, SPAWN_AGENT, REGISTER_CAPABILITY, REGISTER_HOOK, AUTONOMOUS |
//! | Identity | NOP, IDENTITY |
//! | Internal | CONST_STR, YIELD |
//!
//! NEGOTIATE, SPAWN_TEAM, GUARD, and CLAIM were deleted: measured
//! zero emissions across the example/test/studio-lowering corpus. LOOP_START
//! and LOOP_END were deleted because they compiled and verified but never
//! re-executed at runtime — the executor is a DAG engine with no back-edge or
//! re-splice wired to either handler. The one real in-graph iteration
//! mechanism is graph splicing (`splice_dag`/`rearm_session_turn` in
//! `apxm-runtime`'s scheduler); AUTONOMOUS is a documented macro-op with its
//! own internal loop, not the general iteration mechanism. See
//! graph splicing is the executable iteration mechanism. These are `.apxmobj` wire-format breaks; see
//! [`operations::WIRE_INDEXED_OPERATIONS`] for the retired indices.

pub mod aam;
pub mod attrs;
pub mod capabilities;
pub mod chat;
pub mod defaults;
pub mod memory;
pub mod operations;
pub mod passes;
pub mod types;
pub mod validation;

pub use aam::{AAM, Beliefs, Capabilities, Goal, GoalId, GoalStatus, Goals};
pub use memory::MemoryTier;
pub use operations::tablegen::generate_tablegen;
pub use operations::{
    AIS_OPERATIONS, AISOperationType, ARTIFACT_OPERATION_KIND_CASES_FILE,
    ARTIFACT_OPERATION_KIND_ENTRIES_FILE, ContextStyle, MlirEmissionSpec, MlirResultType,
    OP_SPEC_CATALOG_FILE, OP_SPEC_SCHEMA_VERSION, OP_SPEC_VECTORS_FILE,
    OP_SPEC_VECTORS_SCHEMA_VERSION, OperationCategory, OperationField, OperationLatency,
    OperationSpec, WIRE_INDEXED_OPERATIONS, generate_artifact_operation_kind_cases,
    generate_artifact_operation_kind_entries, generate_op_spec_catalog, generate_op_spec_vectors,
    get_all_operations, get_operation_spec, render_op_spec_files,
};
pub use types::Value;
pub use validation::{
    ValidationError, has_required_fields, missing_required_fields, validate_operation,
    validate_operation_strict,
};

pub use passes::{generate_pass_descriptors, generate_pass_dispatch, generate_passes_tablegen};
