//! AIS Operation Definitions - Single Source of Truth
//!
//! This module contains the complete specification for all 44 AIS operations
//! (41 public + 1 metadata + 2 internal). Both the compiler and runtime use
//! these definitions to ensure consistent semantics.

use super::category::OperationCategory;
use crate::attrs;
use serde::{Deserialize, Serialize};
use std::fmt;

// ============================================================================
// Operation Type Enum
// ============================================================================

/// Represents all AIS operation types.
///
/// This enum is the canonical list of operations (44 total):
/// - 1 metadata operation (AgentOp)
/// - 41 public operations
/// - 2 internal operations (ConstStr, Yield)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AISOperationType {
    // Metadata Operation (1) - matches tablegen ais.agent
    /// Agent metadata declaration (memory, beliefs, goals, capabilities).
    Agent,

    // Memory Operations (2)
    /// Query memory (read from memory system).
    #[serde(rename = "QMEM")]
    QMem,
    /// Update memory (write to memory system).
    #[serde(rename = "UMEM")]
    UMem,

    // LLM Operations (3) - Compiler markers for critical path analysis
    /// Simple Q&A (no extended thinking) - LOW latency marker.
    Ask,
    /// Extended thinking with budget - HIGH latency marker.
    Think,
    /// Structured reasoning with beliefs/goals - MEDIUM latency marker.
    Reason,

    // Planning & Analysis Operations (3)
    /// Planning operation (generate a plan using LLM).
    Plan,
    /// Reflection operation (analyze execution trace).
    Reflect,
    /// Verification operation (fact-check against evidence).
    Verify,

    // Tool Operations (3)
    /// Invoke a tool/capability (NOT for agents — use SPAWN_AGENT).
    InvTool,
    /// Execute code in sandbox.
    Exc,
    /// Print output to stdout.
    Print,

    // Control Flow Operations (7)
    /// Unconditional jump to label.
    Jump,
    /// Branch based on value comparison.
    BranchOnValue,
    /// Loop start marker.
    LoopStart,
    /// Loop end marker.
    LoopEnd,
    /// Return from subgraph with result.
    Return,
    /// Multi-way branch based on string value (switch/case).
    Switch,
    /// Call a flow on another agent.
    FlowCall,
    /// Spawn an external AIR file, artifact, or workflow as a child execution.
    WorkflowSpawn,
    /// Call another skill by manifest identity (`<skill_id>[@<version>]`).
    /// The runtime resolves the id through the live `SkillLibrary` and
    /// records the resolved `(skill_id, version, artifact_hash)` triple in
    /// the parent's provenance.
    CallSkill,

    // Synchronization Operations (3)
    /// Merge multiple tokens into one.
    Merge,
    /// Memory fence (synchronization barrier).
    Fence,
    /// Wait for all input tokens to be ready.
    WaitAll,

    // Error Handling Operations (2)
    /// Try-catch exception handling.
    TryCatch,
    /// Error handler invocation.
    Err,

    // Communication Operations (2)
    /// Communication between agents.
    Communicate,
    /// Hand off execution from one agent to another with optional state transfer.
    Handoff,

    // Coordination Operations
    /// Update agent goals at runtime (set/remove/clear).
    UpdateGoal,
    /// Enforce preconditions before execution continues.
    Guard,
    /// Atomically claim a task from a shared work queue.
    Claim,
    /// Suspend execution pending human-in-the-loop review.
    Pause,
    /// Resume a suspended execution from a PAUSE checkpoint.
    Resume,

    // Multi-agent coordination operations
    /// Delegate a task to a sub-agent.
    Delegate,
    /// Multi-agent negotiation protocol.
    Negotiate,

    // Identity Operations
    /// No-op passthrough (no AAM transition).
    Nop,
    /// Identity passthrough (AAM identity transition recorded).
    Identity,

    // Self-Organization Operations
    /// Spawn a new agent instance at runtime.
    SpawnAgent,
    /// Spawn all members of a team (expands to N SPAWN_AGENT operations).
    SpawnTeam,
    /// Register a new capability in the runtime registry.
    RegisterCapability,

    // Autonomous Execution
    /// Run a goal-directed autonomous loop.
    Autonomous,

    // Durable Execution
    /// Create a durable execution checkpoint (snapshot AAM + tokens to STM).
    Checkpoint,

    // Internal Operations (not part of public AIS)
    /// String constant (compiler internal).
    ConstStr,
    /// Yield value from switch case region (compiler internal).
    Yield,
}

/// Canonical AIS artifact wire operation table.
///
/// This table is the single source of truth for operation-kind indexes in the
/// artifact format. Index 30 is intentionally reserved.
pub const WIRE_INDEXED_OPERATIONS: &[(u32, AISOperationType)] = &[
    (0, AISOperationType::InvTool),
    (1, AISOperationType::Ask),
    (2, AISOperationType::QMem),
    (3, AISOperationType::UMem),
    (4, AISOperationType::Plan),
    (5, AISOperationType::WaitAll),
    (6, AISOperationType::Merge),
    (7, AISOperationType::Fence),
    (8, AISOperationType::Exc),
    (9, AISOperationType::Communicate),
    (10, AISOperationType::Reflect),
    (11, AISOperationType::Verify),
    (12, AISOperationType::Err),
    (13, AISOperationType::Return),
    (14, AISOperationType::Jump),
    (15, AISOperationType::BranchOnValue),
    (16, AISOperationType::LoopStart),
    (17, AISOperationType::LoopEnd),
    (18, AISOperationType::TryCatch),
    (19, AISOperationType::ConstStr),
    (20, AISOperationType::Switch),
    (21, AISOperationType::FlowCall),
    (22, AISOperationType::Print),
    (23, AISOperationType::Think),
    (24, AISOperationType::Reason),
    (25, AISOperationType::UpdateGoal),
    (26, AISOperationType::Guard),
    (27, AISOperationType::Claim),
    (28, AISOperationType::Pause),
    (29, AISOperationType::Resume),
    (31, AISOperationType::Delegate),
    (32, AISOperationType::Negotiate),
    (33, AISOperationType::Nop),
    (34, AISOperationType::Identity),
    (35, AISOperationType::SpawnAgent),
    (36, AISOperationType::RegisterCapability),
    (37, AISOperationType::Autonomous),
    (38, AISOperationType::Checkpoint),
    (39, AISOperationType::SpawnTeam),
    (40, AISOperationType::Handoff),
    (41, AISOperationType::WorkflowSpawn),
    (42, AISOperationType::CallSkill),
];

impl fmt::Display for AISOperationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Metadata
            AISOperationType::Agent => write!(f, "AGENT"),
            // Memory
            AISOperationType::QMem => write!(f, "QMEM"),
            AISOperationType::UMem => write!(f, "UMEM"),
            // LLM Operations (markers for runtime config lookup)
            AISOperationType::Ask => write!(f, "ASK"),
            AISOperationType::Think => write!(f, "THINK"),
            AISOperationType::Reason => write!(f, "REASON"),
            // Planning & Analysis
            AISOperationType::Plan => write!(f, "PLAN"),
            AISOperationType::Reflect => write!(f, "REFLECT"),
            AISOperationType::Verify => write!(f, "VERIFY"),
            // Tools
            AISOperationType::InvTool => write!(f, "INV_TOOL"),
            AISOperationType::Exc => write!(f, "EXC"),
            AISOperationType::Print => write!(f, "PRINT"),
            // Control Flow
            AISOperationType::Jump => write!(f, "JUMP"),
            AISOperationType::BranchOnValue => write!(f, "BRANCH_ON_VALUE"),
            AISOperationType::LoopStart => write!(f, "LOOP_START"),
            AISOperationType::LoopEnd => write!(f, "LOOP_END"),
            AISOperationType::Return => write!(f, "RETURN"),
            AISOperationType::Switch => write!(f, "SWITCH"),
            AISOperationType::FlowCall => write!(f, "FLOW_CALL"),
            AISOperationType::WorkflowSpawn => write!(f, "WORKFLOW_SPAWN"),
            AISOperationType::CallSkill => write!(f, "CALL_SKILL"),
            // Synchronization
            AISOperationType::Merge => write!(f, "MERGE"),
            AISOperationType::Fence => write!(f, "FENCE"),
            AISOperationType::WaitAll => write!(f, "WAIT_ALL"),
            // Error Handling
            AISOperationType::TryCatch => write!(f, "TRY_CATCH"),
            AISOperationType::Err => write!(f, "ERR"),
            // Communication
            AISOperationType::Communicate => write!(f, "COMMUNICATE"),
            AISOperationType::Handoff => write!(f, "HANDOFF"),
            // Coordination
            AISOperationType::UpdateGoal => write!(f, "UPDATE_GOAL"),
            AISOperationType::Guard => write!(f, "GUARD"),
            AISOperationType::Claim => write!(f, "CLAIM"),
            AISOperationType::Pause => write!(f, "PAUSE"),
            AISOperationType::Resume => write!(f, "RESUME"),
            // Multi-agent coordination
            AISOperationType::Delegate => write!(f, "DELEGATE"),
            AISOperationType::Negotiate => write!(f, "NEGOTIATE"),
            // Identity
            AISOperationType::Nop => write!(f, "NOP"),
            AISOperationType::Identity => write!(f, "IDENTITY"),
            // Self-Organization
            AISOperationType::SpawnAgent => write!(f, "SPAWN_AGENT"),
            AISOperationType::SpawnTeam => write!(f, "SPAWN_TEAM"),
            AISOperationType::RegisterCapability => write!(f, "REGISTER_CAPABILITY"),
            // Autonomous
            AISOperationType::Autonomous => write!(f, "AUTONOMOUS"),
            // Durable Execution
            AISOperationType::Checkpoint => write!(f, "CHECKPOINT"),
            // Internal
            AISOperationType::ConstStr => write!(f, "CONST_STR"),
            AISOperationType::Yield => write!(f, "YIELD"),
        }
    }
}

impl std::str::FromStr for AISOperationType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, String> {
        let normalized = value.trim().to_ascii_lowercase().replace('-', "_");

        match normalized.as_str() {
            "agent" => Ok(AISOperationType::Agent),
            "qmem" => Ok(AISOperationType::QMem),
            "umem" => Ok(AISOperationType::UMem),
            "ask" => Ok(AISOperationType::Ask),
            "think" => Ok(AISOperationType::Think),
            "reason" => Ok(AISOperationType::Reason),
            "plan" => Ok(AISOperationType::Plan),
            "reflect" => Ok(AISOperationType::Reflect),
            "verify" => Ok(AISOperationType::Verify),
            "inv_tool" => Ok(AISOperationType::InvTool),
            "exc" => Ok(AISOperationType::Exc),
            "print" => Ok(AISOperationType::Print),
            "jump" => Ok(AISOperationType::Jump),
            "branch_on_value" => Ok(AISOperationType::BranchOnValue),
            "loop_start" => Ok(AISOperationType::LoopStart),
            "loop_end" => Ok(AISOperationType::LoopEnd),
            "return" => Ok(AISOperationType::Return),
            "switch" => Ok(AISOperationType::Switch),
            "flow_call" => Ok(AISOperationType::FlowCall),
            "workflow_spawn" => Ok(AISOperationType::WorkflowSpawn),
            "call_skill" => Ok(AISOperationType::CallSkill),
            "merge" => Ok(AISOperationType::Merge),
            "fence" => Ok(AISOperationType::Fence),
            "wait_all" => Ok(AISOperationType::WaitAll),
            "try_catch" => Ok(AISOperationType::TryCatch),
            "err" => Ok(AISOperationType::Err),
            "communicate" => Ok(AISOperationType::Communicate),
            "handoff" => Ok(AISOperationType::Handoff),
            "update_goal" => Ok(AISOperationType::UpdateGoal),
            "guard" => Ok(AISOperationType::Guard),
            "claim" => Ok(AISOperationType::Claim),
            "pause" => Ok(AISOperationType::Pause),
            "resume" => Ok(AISOperationType::Resume),
            "delegate" => Ok(AISOperationType::Delegate),
            "negotiate" => Ok(AISOperationType::Negotiate),
            "nop" => Ok(AISOperationType::Nop),
            "identity" => Ok(AISOperationType::Identity),
            "spawn_agent" => Ok(AISOperationType::SpawnAgent),
            "spawn_team" => Ok(AISOperationType::SpawnTeam),
            "register_capability" => Ok(AISOperationType::RegisterCapability),
            "autonomous" => Ok(AISOperationType::Autonomous),
            "checkpoint" => Ok(AISOperationType::Checkpoint),
            "const_str" => Ok(AISOperationType::ConstStr),
            "yield" => Ok(AISOperationType::Yield),
            _ => Err(format!("Unknown AIS operation type: '{value}'")),
        }
    }
}

impl AISOperationType {
    /// Returns the MLIR mnemonic for this operation (e.g., "rsn", "qmem").
    pub fn mlir_mnemonic(&self) -> &'static str {
        match self {
            AISOperationType::Agent => "agent",
            AISOperationType::QMem => "qmem",
            AISOperationType::UMem => "umem",
            AISOperationType::Ask => "ask",
            AISOperationType::Think => "think",
            AISOperationType::Reason => "reason",
            AISOperationType::Plan => "plan",
            AISOperationType::Reflect => "reflect",
            AISOperationType::Verify => "verify",
            AISOperationType::InvTool => "inv_tool",
            AISOperationType::Exc => "exc",
            AISOperationType::Print => "print",
            AISOperationType::Jump => "jump",
            AISOperationType::BranchOnValue => "branch_on_value",
            AISOperationType::LoopStart => "loop_start",
            AISOperationType::LoopEnd => "loop_end",
            AISOperationType::Return => "return",
            AISOperationType::Switch => "switch",
            AISOperationType::FlowCall => "flow_call",
            AISOperationType::WorkflowSpawn => "workflow_spawn",
            AISOperationType::CallSkill => "call_skill",
            AISOperationType::Merge => "merge",
            AISOperationType::Fence => "fence",
            AISOperationType::WaitAll => "wait_all",
            AISOperationType::TryCatch => "try_catch",
            AISOperationType::Err => "err",
            AISOperationType::Communicate => "communicate",
            AISOperationType::Handoff => "handoff",
            AISOperationType::UpdateGoal => "update_goal",
            AISOperationType::Guard => "guard",
            AISOperationType::Claim => "claim",
            AISOperationType::Pause => "pause",
            AISOperationType::Resume => "resume",
            AISOperationType::Delegate => "delegate",
            AISOperationType::Negotiate => "negotiate",
            AISOperationType::Nop => "nop",
            AISOperationType::Identity => "identity",
            AISOperationType::SpawnAgent => "spawn_agent",
            AISOperationType::SpawnTeam => "spawn_team",
            AISOperationType::RegisterCapability => "register_capability",
            AISOperationType::Autonomous => "autonomous",
            AISOperationType::Checkpoint => "checkpoint",
            AISOperationType::ConstStr => "const_str",
            AISOperationType::Yield => "yield",
        }
    }

    /// Maps a wire-format operation kind index (u32) to an `AISOperationType`.
    ///
    /// This is the **single source of truth** for the u32→AISOperationType mapping
    /// used by both the compiler artifact parser and the runtime sub-DAG parser.
    pub fn from_wire_index(index: u32) -> Option<AISOperationType> {
        WIRE_INDEXED_OPERATIONS
            .iter()
            .find_map(|(wire_index, op)| (*wire_index == index).then_some(*op))
    }

    /// Convert an `AISOperationType` to its wire-format index.
    ///
    /// Returns `None` for operation types that do not have a wire-format index
    /// (e.g., `Agent`, `Yield`).
    ///
    /// This is the inverse of [`from_wire_index`].
    pub fn to_wire_index(self) -> Option<u32> {
        WIRE_INDEXED_OPERATIONS
            .iter()
            .find_map(|(wire_index, op)| (*op == self).then_some(*wire_index))
    }

    /// Return all artifact wire-indexed operations in stable wire order.
    pub fn wire_indexed_operations() -> &'static [(u32, AISOperationType)] {
        WIRE_INDEXED_OPERATIONS
    }

    /// Get all operation types (44 total).
    pub fn all_operations() -> &'static [AISOperationType] {
        &[
            AISOperationType::Agent,
            AISOperationType::QMem,
            AISOperationType::UMem,
            AISOperationType::Ask,
            AISOperationType::Think,
            AISOperationType::Reason,
            AISOperationType::Plan,
            AISOperationType::Reflect,
            AISOperationType::Verify,
            AISOperationType::InvTool,
            AISOperationType::Exc,
            AISOperationType::Print,
            AISOperationType::Jump,
            AISOperationType::BranchOnValue,
            AISOperationType::LoopStart,
            AISOperationType::LoopEnd,
            AISOperationType::Return,
            AISOperationType::Switch,
            AISOperationType::FlowCall,
            AISOperationType::WorkflowSpawn,
            AISOperationType::CallSkill,
            AISOperationType::Merge,
            AISOperationType::Fence,
            AISOperationType::WaitAll,
            AISOperationType::TryCatch,
            AISOperationType::Err,
            AISOperationType::Communicate,
            AISOperationType::Handoff,
            AISOperationType::UpdateGoal,
            AISOperationType::Guard,
            AISOperationType::Claim,
            AISOperationType::Pause,
            AISOperationType::Resume,
            AISOperationType::Delegate,
            AISOperationType::Negotiate,
            AISOperationType::Nop,
            AISOperationType::Identity,
            AISOperationType::SpawnAgent,
            AISOperationType::SpawnTeam,
            AISOperationType::RegisterCapability,
            AISOperationType::Autonomous,
            AISOperationType::Checkpoint,
            AISOperationType::ConstStr,
            AISOperationType::Yield,
        ]
    }
}

// ============================================================================
// Field Specifications
// ============================================================================

/// Indicates that an operation field references an external resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceType {
    /// APXM ACP agent profile.
    Profile,
    /// Registered LLM backend (e.g., "openai", "corp-gateway").
    Backend,
    /// LLM model identifier accepted by the selected backend.
    Model,
    /// Registered tool/capability (e.g., "bash", "web_search").
    Capability,
}

impl ReferenceType {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Profile => "agent profile",
            Self::Backend => "backend",
            Self::Model => "model",
            Self::Capability => "capability",
        }
    }

    pub const fn list_command(self) -> &'static str {
        match self {
            Self::Profile => "apxm agent list",
            Self::Backend => "apxm backend list",
            Self::Model => "apxm backend list",
            Self::Capability => "apxm tool list",
        }
    }

    pub const fn add_command(self) -> &'static str {
        match self {
            Self::Profile => "apxm agent add",
            Self::Backend => "apxm backend add",
            Self::Model => "apxm backend add-model",
            Self::Capability => "apxm tool add",
        }
    }
}

/// MLIR emission configuration for code generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MlirEmissionSpec {
    /// Primary attribute name (e.g., "template_str" for ASK).
    pub primary_attr: Option<&'static str>,
    /// Context style for operand lists.
    pub context_style: ContextStyle,
    /// MLIR result type.
    pub result_type: MlirResultType,
    /// Positional attributes (e.g., ["recipient"] for COMMUNICATE).
    pub positional_attrs: &'static [&'static str],
    /// Keyword arguments emitted as the trailing attr-dict
    /// (e.g., ["profile", "mode"] → `{profile = "...", mode = "..."}`).
    pub keywords: &'static [&'static str],
    /// Syntactic-keyword attributes that emit as `<keyword> "<value>"` between
    /// the primary attribute and the operand list (e.g., COMMUNICATE's
    /// `to $recipient`, DELEGATE's `to $target_agent`). Pairs are
    /// `(literal_keyword, attr_name)`.
    pub syntactic_keywords: &'static [(&'static str, &'static str)],
}

/// Context style for MLIR emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextStyle {
    /// Bracketed context: [...].
    Bracketed,
    /// Parenthesized context: (...).
    Parenthesized,
    /// Direct variadic operands (no delimiters): %a, %b : type, type -> result_type.
    Direct,
    /// No context.
    None,
}

/// MLIR result type for operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MlirResultType {
    /// Returns a token (!ais.token).
    Token,
    /// Returns a handle (!ais.handle).
    Handle,
    /// No return value (void).
    Void,
}

/// A field in an operation specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OperationField {
    /// Field name.
    pub name: &'static str,
    /// Whether the field is required.
    pub required: bool,
    /// Description of the field.
    pub description: &'static str,
    /// If set, the field value references an external resource of this type.
    pub ref_type: Option<ReferenceType>,
}

impl OperationField {
    /// Create a required field.
    pub const fn required(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            required: true,
            description,
            ref_type: None,
        }
    }

    /// Create an optional field.
    pub const fn optional(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            required: false,
            description,
            ref_type: None,
        }
    }

    /// Create a required field that references an external resource.
    pub const fn required_ref(
        name: &'static str,
        description: &'static str,
        ref_type: ReferenceType,
    ) -> Self {
        Self {
            name,
            required: true,
            description,
            ref_type: Some(ref_type),
        }
    }

    /// Create an optional field that references an external resource.
    pub const fn optional_ref(
        name: &'static str,
        description: &'static str,
        ref_type: ReferenceType,
    ) -> Self {
        Self {
            name,
            required: false,
            description,
            ref_type: Some(ref_type),
        }
    }
}

// ============================================================================
// Latency Classification
// ============================================================================

/// Expected latency tier for an operation, used by the scheduler for
/// critical-path analysis and by agents for cost estimation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationLatency {
    /// No external I/O — executes in microseconds (control flow, sync).
    None,
    /// Local I/O only — millisecond range (memory, code sandbox).
    Low,
    /// Single LLM call or tool invocation — seconds range.
    Medium,
    /// Extended thinking / multi-step LLM — tens of seconds.
    High,
}

impl OperationLatency {
    /// Returns a human-readable label.
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationLatency::None => "none",
            OperationLatency::Low => "low",
            OperationLatency::Medium => "medium",
            OperationLatency::High => "high",
        }
    }
}

impl fmt::Display for OperationLatency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// Operation Specification
// ============================================================================

/// Complete specification of an AIS operation.
///
/// This is the single source of truth for operation semantics. Both the
/// compiler (for validation and code generation) and runtime (for dispatch
/// and execution) use these specifications.
#[derive(Debug, Clone, Serialize)]
pub struct OperationSpec {
    /// Operation type identifier.
    pub op_type: AISOperationType,
    /// Human-readable name (e.g., "QueryMemory", "Reason").
    pub name: &'static str,
    /// Operation category.
    pub category: OperationCategory,
    /// Short description of what the operation does (one line).
    pub description: &'static str,
    /// Extended description with usage guidance (for agents and docs).
    pub long_description: &'static str,
    /// Expected latency tier.
    pub latency: OperationLatency,
    /// Minimal JSON example showing typical usage (for agents).
    pub example_json: Option<&'static str>,
    /// Required and optional input fields.
    pub fields: &'static [OperationField],
    /// Whether this operation needs async execution (submission to executor).
    pub needs_submission: bool,
    /// Minimum number of input tokens required.
    pub min_inputs: u32,
    /// Whether operation produces output tokens.
    pub produces_output: bool,
    /// MLIR emission configuration for code generation.
    pub emission: MlirEmissionSpec,
}

impl OperationSpec {
    /// Get a field by name.
    pub fn get_field(&self, name: &str) -> Option<&OperationField> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Get all required fields.
    pub fn required_fields(&self) -> impl Iterator<Item = &OperationField> {
        self.fields.iter().filter(|f| f.required)
    }
}

// ============================================================================
// MLIR Emission Helpers
// ============================================================================

/// Standard emission spec: Token result, bracketed context, no primary attr.
const EMISSION_TOKEN_BRACKETED: MlirEmissionSpec = MlirEmissionSpec {
    primary_attr: None,
    context_style: ContextStyle::Bracketed,
    result_type: MlirResultType::Token,
    positional_attrs: &[],
    keywords: &[],
    syntactic_keywords: &[],
};

/// Emission spec for LOOP_START: token result, no operands, with the loop
/// bound (`max_iterations`) and `label` carried in the trailing attr-dict so
/// the iteration bound round-trips through text-AIR.
const EMISSION_LOOP_START: MlirEmissionSpec = MlirEmissionSpec {
    primary_attr: None,
    context_style: ContextStyle::None,
    result_type: MlirResultType::Token,
    positional_attrs: &[],
    keywords: &["max_iterations", "label"],
    syntactic_keywords: &[],
};

/// Emission spec for NEGOTIATE: `proposal` primary, parties/max_rounds in the
/// attr-dict, parenthesized token inputs, token result.
const EMISSION_NEGOTIATE: MlirEmissionSpec = MlirEmissionSpec {
    primary_attr: Some("proposal"),
    context_style: ContextStyle::Parenthesized,
    result_type: MlirResultType::Token,
    positional_attrs: &[],
    keywords: &["parties", "max_rounds"],
    syntactic_keywords: &[],
};

/// Standard emission spec: Void result, no context.
const EMISSION_VOID_NONE: MlirEmissionSpec = MlirEmissionSpec {
    primary_attr: None,
    context_style: ContextStyle::None,
    result_type: MlirResultType::Void,
    positional_attrs: &[],
    keywords: &[],
    syntactic_keywords: &[],
};

/// Standard emission spec: Handle result, parenthesized context.

// ============================================================================
// Operation Registry
// ============================================================================

/// All AIS operations (metadata + public + internal) with complete specs.
pub static AIS_OPERATIONS: &[OperationSpec] = &[
    // ========== Metadata Operation (1) ==========
    OperationSpec {
        op_type: AISOperationType::Agent,
        name: "Agent",
        category: OperationCategory::Metadata,
        description: "Agent structural declaration (memory, beliefs, goals, capabilities)",
        long_description: "Declares an agent's identity and initial AAM state. Every graph must \
            have exactly one AGENT node. It configures memory tiers (STM/LTM/Episodic), initial \
            beliefs and goals, and the capabilities the agent can invoke. The runtime uses this \
            to initialize the agent's AAM before executing any other node.",
        latency: OperationLatency::None,
        example_json: Some(
            r#"{"id": 0, "op": "AGENT", "attributes": {"memory": {"stm": true}, "beliefs": {"role": "analyst"}, "goals": ["summarize data"], "capabilities": ["search", "calculate"]}}"#,
        ),
        fields: &[
            OperationField::optional("memory", "Memory configuration"),
            OperationField::optional("beliefs", "Initial beliefs"),
            OperationField::optional("goals", "Initial goals"),
            OperationField::optional("capabilities", "Available capabilities"),
        ], // Agent fields are structural, not graph attrs
        needs_submission: false,
        min_inputs: 0,
        produces_output: false,
        emission: EMISSION_VOID_NONE,
    },
    // ========== Memory Operations (2) ==========
    OperationSpec {
        op_type: AISOperationType::QMem,
        name: "QueryMemory",
        category: OperationCategory::Memory,
        description: "Retrieve data from memory (STM, LTM, or Episodic)",
        long_description: "Reads from the agent's memory system. Supports three tiers: \
            STM (short-term, per-execution scratch), LTM (long-term, persists across runs), \
            and Episodic (execution traces). The query string is matched against stored keys. \
            Returns the stored value or null if not found.",
        latency: OperationLatency::Low,
        example_json: Some(
            r#"{"id": 2, "op": "QMEM", "attributes": {"query": "user_name", "memory_tier": "stm"}}"#,
        ),
        fields: &[
            OperationField::required(attrs::QUERY, "Query string or key to search for"),
            OperationField::optional(
                attrs::MEMORY_TIER,
                "Target memory tier: stm, ltm, or episodic",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::QUERY),
            context_style: ContextStyle::Bracketed,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[attrs::MEMORY_TIER],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::UMem,
        name: "UpdateMemory",
        category: OperationCategory::Memory,
        description: "Persist or update data in memory",
        long_description: "Writes a key-value pair to the agent's memory system. If the key \
            already exists, the value is overwritten. Supports the same three memory tiers as \
            QMEM. Use FENCE after UMEM if subsequent QMEM nodes must see the write.",
        latency: OperationLatency::Low,
        example_json: Some(
            r#"{"id": 3, "op": "UMEM", "attributes": {"key": "summary", "value": "Rust favors explicit ownership and borrowing.", "memory_tier": "stm"}}"#,
        ),
        fields: &[
            OperationField::required(attrs::KEY, "Key to store the value under"),
            OperationField::required(attrs::VALUE, "Value to store"),
            OperationField::optional(
                attrs::MEMORY_TIER,
                "Target memory tier: stm, ltm, or episodic",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::KEY),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[attrs::MEMORY_TIER],
            syntactic_keywords: &[],
        },
    },
    // ========== LLM Operations (3) ==========
    OperationSpec {
        op_type: AISOperationType::Ask,
        name: "Ask",
        category: OperationCategory::Reasoning,
        description: "Simple Q&A with LLM (no extended thinking) - LOW latency",
        long_description: "Sends a prompt to the configured LLM and returns the response. \
            The lightest LLM operation — no chain-of-thought or extended thinking. Use for \
            straightforward questions, classifications, extractions, or reformulations. \
            Template strings support named `{input}` interpolation for dataflow inputs via \
            the node's `input_names` array.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 1, "op": "ASK", "attributes": {"template_str": "Summarize: {source}", "input_names": ["source"]}}"#,
        ),
        fields: &[
            OperationField::required(attrs::TEMPLATE_STR, "Prompt template for the question"),
            OperationField::optional(attrs::TEMPERATURE, "Sampling temperature (0.0-1.0)"),
            OperationField::optional_ref(
                attrs::MODEL,
                "LLM model override (uses config default)",
                ReferenceType::Model,
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::TEMPLATE_STR),
            context_style: ContextStyle::Bracketed,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[
                attrs::TEMPERATURE,
                attrs::MODEL,
                attrs::PROVIDER,
                attrs::BACKEND,
                attrs::SYSTEM_PROMPT,
            ],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Think,
        name: "Think",
        category: OperationCategory::Reasoning,
        description: "Extended thinking with token_budget - HIGH latency",
        long_description: "Activates extended thinking (chain-of-thought) with a configurable \
            token budget. The LLM produces internal reasoning before the final answer. Use for \
            complex multi-step problems, math, code generation, or planning that benefits from \
            deliberate reasoning. The budget controls how many tokens the model can spend thinking.",
        latency: OperationLatency::High,
        example_json: Some(
            r#"{"id": 1, "op": "THINK", "attributes": {"template_str": "Solve step by step: {problem}", "input_names": ["problem"], "budget": 4096}}"#,
        ),
        fields: &[
            OperationField::required(attrs::TEMPLATE_STR, "Prompt template for deep reasoning"),
            OperationField::optional(attrs::BUDGET, "Token budget for extended thinking"),
            OperationField::optional(attrs::TEMPERATURE, "Sampling temperature (0.0-1.0)"),
            OperationField::optional_ref(
                attrs::MODEL,
                "LLM model override (uses config default)",
                ReferenceType::Model,
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::TEMPLATE_STR),
            context_style: ContextStyle::Bracketed,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[
                attrs::BUDGET,
                attrs::TEMPERATURE,
                attrs::MODEL,
                attrs::PROVIDER,
                attrs::BACKEND,
                attrs::SYSTEM_PROMPT,
            ],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Reason,
        name: "Reason",
        category: OperationCategory::Reasoning,
        description: "Structured reasoning with belief/goal updates - MEDIUM latency",
        long_description: "Performs structured reasoning that can update the agent's beliefs \
            and goals (AAM state). Unlike ASK, the runtime parses the LLM response for belief \
            and goal mutations. Use when the agent needs to update its internal state based on \
            new information. Supports structured JSON output mode.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 1, "op": "REASON", "attributes": {"template_str": "Given {evidence}, update your analysis", "input_names": ["evidence"], "structured": true}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::TEMPLATE_STR,
                "Prompt template for structured reasoning",
            ),
            OperationField::optional(attrs::TEMPERATURE, "Sampling temperature (0.0-1.0)"),
            OperationField::optional_ref(
                attrs::MODEL,
                "LLM model override (uses config default)",
                ReferenceType::Model,
            ),
            OperationField::optional(attrs::STRUCTURED, "Enable structured JSON output"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::TEMPLATE_STR),
            context_style: ContextStyle::Bracketed,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[
                attrs::TEMPERATURE,
                attrs::MODEL,
                attrs::PROVIDER,
                attrs::BACKEND,
                attrs::SYSTEM_PROMPT,
                attrs::STRUCTURED,
            ],
            syntactic_keywords: &[],
        },
    },
    // ========== Planning & Analysis Operations (3) ==========
    OperationSpec {
        op_type: AISOperationType::Plan,
        name: "Plan",
        category: OperationCategory::Reasoning,
        description: "Decompose goal into AIS subgraph",
        long_description: "Uses the LLM to decompose a high-level goal into a sequence of \
            concrete steps. The output is a structured plan that can be used to drive subsequent \
            nodes. Supports optional constraints to bound the plan space.",
        latency: OperationLatency::High,
        example_json: Some(
            r#"{"id": 1, "op": "PLAN", "attributes": {"goal": "Research and summarize recent AI papers", "constraints": "max 5 steps"}}"#,
        ),
        fields: &[
            OperationField::required(attrs::GOAL, "Goal to decompose into steps"),
            OperationField::optional("constraints", "Constraints on the plan"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::GOAL),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Reflect,
        name: "Reflect",
        category: OperationCategory::Reasoning,
        description: "Analyze execution trace for self-improvement",
        long_description: "Retrieves past execution traces and asks the LLM to analyze them \
            for patterns, failures, or improvements. Useful for iterative refinement loops \
            where the agent learns from its own execution history.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 4, "op": "REFLECT", "attributes": {"trace_query": "last_execution"}}"#,
        ),
        fields: &[
            OperationField::required(attrs::TRACE_QUERY, "Query to retrieve trace for reflection"),
            OperationField::optional("reflection_prompt", "Custom prompt for reflection"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::TRACE_QUERY),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Verify,
        name: "Verify",
        category: OperationCategory::Reasoning,
        description: "Fact-check outputs against evidence",
        long_description: "Cross-references a claim against provided evidence using the LLM. \
            Returns a verification result with confidence score. Use after ASK/THINK/REASON \
            nodes to validate outputs before acting on them. Evidence may be supplied either \
            as a literal attribute or via an incoming Data edge.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 5, "op": "VERIFY", "attributes": {"claim": "The solar system has eight planets."}}"#,
        ),
        fields: &[
            OperationField::required(attrs::CLAIM_TEXT, "Claim to verify"),
            OperationField::optional(attrs::EVIDENCE, "Evidence to check against"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    // ========== Tool Operations (3) ==========
    OperationSpec {
        op_type: AISOperationType::InvTool,
        name: "InvokeTool",
        category: OperationCategory::Tools,
        description: "Call external tool with structured params; store result",
        long_description: "Invokes a registered capability (tool or function) by name. \
            The capability must be declared in the AGENT node's capabilities list or \
            registered in the runtime's CapabilityRegistry. Parameters are passed via the \
            `params_json` attribute as a JSON object. The tool's return value becomes this \
            node's output token.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 2, "op": "INV_TOOL", "attributes": {"capability": "web_search", "params_json": "{\"query\":\"rust ownership\"}"}}"#,
        ),
        fields: &[
            OperationField::required_ref(
                attrs::CAPABILITY,
                "Name of the capability/tool to invoke",
                ReferenceType::Capability,
            ),
            OperationField::optional(
                attrs::PARAMS_JSON,
                "JSON-encoded parameters to pass to the tool",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::CAPABILITY),
            context_style: ContextStyle::Bracketed,
            result_type: MlirResultType::Token,
            positional_attrs: &[attrs::PARAMS_JSON],
            keywords: &[],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Exc,
        name: "ExecuteCode",
        category: OperationCategory::Tools,
        description: "Run code in a sandboxed environment; update Beliefs",
        long_description: "Executes arbitrary code in a sandboxed environment. The sandbox \
            prevents file system access, network calls, and other side effects unless \
            explicitly allowed. The code's stdout/return value becomes the output token. \
            Execution results are also written to the agent's beliefs.",
        latency: OperationLatency::Low,
        example_json: Some(r#"{"id": 3, "op": "EXC", "attributes": {"code": "print(2 + 2)"}}"#),
        fields: &[
            OperationField::required(attrs::CODE, "Code to execute"),
            OperationField::optional("sandbox_config", "Sandbox configuration"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::CODE),
            context_style: ContextStyle::Bracketed,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Print,
        name: "PrintOutput",
        category: OperationCategory::Tools,
        description: "Print output to stdout for debugging or user display",
        long_description: "Writes a message to stdout. Supports named `{input}` template \
            interpolation via `input_names`. Useful for debugging graphs during development \
            or displaying final results to the user. The message is also stored as the output token.",
        latency: OperationLatency::None,
        example_json: Some(
            r#"{"id": 4, "op": "PRINT", "attributes": {"message": "Result: {result}", "input_names": ["result"]}}"#,
        ),
        fields: &[OperationField::required(attrs::MESSAGE, "Message to print")],
        needs_submission: true,
        min_inputs: 0,
        produces_output: false,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::MESSAGE),
            context_style: ContextStyle::Bracketed,
            result_type: MlirResultType::Void,
            positional_attrs: &[],
            keywords: &[],
            syntactic_keywords: &[],
        },
    },
    // ========== Control Flow Operations (7) ==========
    OperationSpec {
        op_type: AISOperationType::Jump,
        name: "Jump",
        category: OperationCategory::ControlFlow,
        description: "Unconditional jump to a labeled instruction",
        long_description: "Transfers control flow unconditionally to a target label. \
            The label must correspond to a node ID in the graph. Edges from this node \
            use Control dependency type.",
        latency: OperationLatency::None,
        example_json: Some(r#"{"id": 5, "op": "JUMP", "attributes": {"label": "7"}}"#),
        fields: &[OperationField::required(
            attrs::LABEL,
            "Target label to jump to",
        )],
        needs_submission: false,
        min_inputs: 0,
        produces_output: false,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::BranchOnValue,
        name: "BranchOnValue",
        category: OperationCategory::ControlFlow,
        description: "Conditional branch based on token value comparison",
        long_description: "Evaluates an input token against a value and branches to one of \
            two labels. If the token matches the value, control goes to label_true; otherwise \
            to label_false. Used for if/else patterns in agent workflows.",
        latency: OperationLatency::None,
        example_json: Some(
            r#"{"id": 5, "op": "BRANCH_ON_VALUE", "attributes": {"value": "yes", "true_label": "6", "false_label": "7"}}"#,
        ),
        fields: &[
            OperationField::required(attrs::VALUE, "Value to compare against"),
            OperationField::required(attrs::TRUE_LABEL, "Label if comparison is true"),
            OperationField::required(attrs::FALSE_LABEL, "Label if comparison is false"),
        ],
        needs_submission: false,
        min_inputs: 1,
        produces_output: false,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::LoopStart,
        name: "LoopStart",
        category: OperationCategory::ControlFlow,
        description: "Begin bounded loop",
        long_description: "Marks the beginning of a bounded loop. The count_token specifies \
            how many iterations to execute. Must be paired with a LOOP_END node. The compiler \
            verifies loop bounds at compile time to prevent infinite loops.",
        latency: OperationLatency::None,
        example_json: Some(r#"{"id": 3, "op": "LOOP_START", "attributes": {"count_token": "3"}}"#),
        fields: &[OperationField::required(
            "count_token", // structural, not a graph attr
            "Token containing iteration count",
        )],
        needs_submission: false,
        min_inputs: 0,
        produces_output: false,
        emission: EMISSION_LOOP_START,
    },
    OperationSpec {
        op_type: AISOperationType::LoopEnd,
        name: "LoopEnd",
        category: OperationCategory::ControlFlow,
        description: "End bounded loop",
        long_description: "Marks the end of a bounded loop started by LOOP_START. The runtime \
            decrements the loop counter and branches back to LOOP_START if iterations remain.",
        latency: OperationLatency::None,
        example_json: None,
        fields: &[],
        needs_submission: false,
        min_inputs: 0,
        produces_output: false,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::Return,
        name: "Return",
        category: OperationCategory::ControlFlow,
        description: "Return from subgraph with result token",
        long_description: "Returns a value from a subgraph or flow. The result value is \
            provided via an incoming Data edge.",
        latency: OperationLatency::None,
        example_json: Some(r#"{"id": 6, "op": "RETURN", "attributes": {}}"#),
        fields: &[],
        needs_submission: false,
        min_inputs: 1,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::Switch,
        name: "Switch",
        category: OperationCategory::ControlFlow,
        description: "Multi-way branch based on string value comparison",
        long_description: "Routes execution to one of several branches based on matching \
            a discriminant token against case labels. Each case specifies a label string \
            and a destination node. If no case matches, the default destination is used. \
            The matched branch's result becomes the output token.",
        latency: OperationLatency::None,
        example_json: Some(
            r#"{"id": 3, "op": "SWITCH", "attributes": {"discriminant": "topic_kind", "cases": [{"label": "math", "node_id": 4}, {"label": "code", "node_id": 5}], "default": "6"}}"#,
        ),
        fields: &[
            OperationField::required("discriminant", "Token to match against case labels"),
            OperationField::required("cases", "Array of case label/destination pairs"),
            OperationField::optional("default", "Default destination if no case matches"),
        ], // structural field names
        needs_submission: false,
        min_inputs: 1,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::FlowCall,
        name: "FlowCall",
        category: OperationCategory::ControlFlow,
        description: "Call a flow on another agent with implicit parallelism",
        long_description: "Invokes a named flow on a target agent. The target agent executes \
            its flow graph independently and returns the result. Multiple FLOW_CALL nodes can \
            execute in parallel if they have no data dependencies between them. This is the \
            primary mechanism for multi-agent composition.",
        latency: OperationLatency::High,
        example_json: Some(
            r#"{"id": 4, "op": "FLOW_CALL", "attributes": {"agent_name": "researcher", "flow_name": "analyze", "args": {"topic": "{topic}"}, "input_names": ["topic"]}}"#,
        ),
        fields: &[
            OperationField::required(attrs::AGENT_NAME, "Name of the agent to call"),
            OperationField::required(attrs::FLOW_NAME, "Name of the flow to invoke"),
            OperationField::optional(attrs::ARGS, "Arguments to pass to the flow"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::AGENT_NAME),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[attrs::FLOW_NAME],
            keywords: &[attrs::ARGS, attrs::INPUT_NAMES],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::WorkflowSpawn,
        name: "WorkflowSpawn",
        category: OperationCategory::ControlFlow,
        description: "Spawn an external AIR file, artifact, or workflow as a child execution",
        long_description: "Invokes an external AIR file, precompiled artifact, or workflow file as a child execution boundary. \
            Arguments are passed by name through the args map. The child run resolves its session root from the explicit node attribute when present, \
            otherwise it inherits the parent execution root.",
        latency: OperationLatency::High,
        example_json: Some(
            r#"{"id": 5, "op": "WORKFLOW_SPAWN", "attributes": {"target_kind": "workflow_path", "target": "workflows/review.apxmw", "args": {"topic": "{topic}"}, "input_names": ["topic"], "await_result": true}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::TARGET_KIND,
                "Invocation target kind: air_path, artifact_path, or workflow_path",
            ),
            OperationField::required(
                attrs::TARGET,
                "Path of the AIR file, artifact, or workflow to execute",
            ),
            OperationField::optional(attrs::ARGS, "Arguments to pass to the child execution"),
            OperationField::optional(
                attrs::SESSION_ROOT,
                "Explicit session root for the child execution",
            ),
            OperationField::optional(
                attrs::AWAIT_RESULT,
                "Whether to wait for the child result (must be true in the current runtime)",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::TARGET_KIND),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[attrs::TARGET],
            keywords: &[
                attrs::ARGS,
                attrs::INPUT_NAMES,
                attrs::SESSION_ROOT,
                attrs::AWAIT_RESULT,
            ],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::CallSkill,
        name: "CallSkill",
        category: OperationCategory::ControlFlow,
        description: "Call another skill by manifest identity (id or id@version)",
        long_description: "Invokes another skill resolved by manifest identity through the live \
            SkillLibrary, rather than by raw artifact path. Resolution is lazy: the runtime parses \
            `<skill_id>[@<version>]`, looks up the matching .apxmobj, admits the child's required \
            capabilities against the parent's grant, dispatches the child entry DAG, and records \
            the resolved (skill_id, version, artifact_hash) triple in the parent's provenance. \
            Failure modes are typed: InvalidSkillId, SkillNotFound, SkillVersionNotFound, \
            CapabilityWiden, CallSkillDepthExceeded, ChildExecutionFailed.",
        latency: OperationLatency::High,
        example_json: Some(
            r#"{"id": 6, "op": "CALL_SKILL", "attributes": {"skill_id": "apxm-orient@0.2.0", "args": ["context"], "input_names": ["context"]}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::SKILL_ID,
                "Skill identifier: \"id\" (latest) or \"id@version\" (pinned)",
            ),
            OperationField::optional(
                attrs::ARGS,
                "Positional arguments forwarded to the child's entry-flow input vector",
            ),
            OperationField::optional(
                attrs::INPUT_NAMES,
                "Optional name vector mapping parent outputs onto the child's positional args",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::SKILL_ID),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[attrs::ARGS, attrs::INPUT_NAMES],
            syntactic_keywords: &[],
        },
    },
    // ========== Synchronization Operations (3) ==========
    OperationSpec {
        op_type: AISOperationType::Merge,
        name: "Merge",
        category: OperationCategory::Synchronization,
        description: "Sync parallel paths; aggregate tokens into one",
        long_description: "Waits for multiple parallel branches to complete and combines \
            their output tokens into a single aggregated result. Inputs are provided via \
            incoming Data edges.",
        latency: OperationLatency::None,
        example_json: Some(r#"{"id": 6, "op": "MERGE", "attributes": {}}"#),
        fields: &[],
        needs_submission: true,
        min_inputs: 1,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: None,
            context_style: ContextStyle::Direct,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Fence,
        name: "Fence",
        category: OperationCategory::Synchronization,
        description: "Memory barrier; order prior QMEM/UMEM operations",
        long_description: "Ensures all preceding memory operations (UMEM writes) are visible \
            to subsequent QMEM reads. Without a FENCE, the scheduler may reorder memory \
            operations for parallelism. Place between UMEM and QMEM when ordering matters.",
        latency: OperationLatency::None,
        example_json: None,
        fields: &[OperationField::optional(
            "ordering", // structural
            "Memory ordering constraint",
        )],
        needs_submission: true,
        min_inputs: 0,
        produces_output: false,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::WaitAll,
        name: "WaitAll",
        category: OperationCategory::Synchronization,
        description: "Block until all specified tokens are available",
        long_description: "Blocks execution until all listed input tokens are ready. Unlike \
            MERGE, it does not combine the tokens — it simply acts as a synchronization barrier. \
            Inputs are provided via incoming Data edges.",
        latency: OperationLatency::None,
        example_json: Some(r#"{"id": 5, "op": "WAIT_ALL", "attributes": {}}"#),
        fields: &[],
        needs_submission: true,
        min_inputs: 1,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: None,
            context_style: ContextStyle::Direct,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[],
            syntactic_keywords: &[],
        },
    },
    // ========== Error Handling Operations (2) ==========
    OperationSpec {
        op_type: AISOperationType::TryCatch,
        name: "TryCatch",
        category: OperationCategory::ErrorHandling,
        description: "Structured exception handling with recovery subgraph",
        long_description: "Wraps a try subgraph with a catch recovery subgraph. If any node \
            in the try subgraph fails, execution transfers to the catch subgraph. The catch \
            subgraph receives the error context and can attempt recovery or graceful degradation.",
        latency: OperationLatency::None,
        example_json: Some(
            r#"{"id": 2, "op": "TRY_CATCH", "attributes": {"try_label": "3", "catch_label": "4"}}"#,
        ),
        fields: &[
            OperationField::required(attrs::TRY_LABEL, "Subgraph to try executing"),
            OperationField::required(attrs::CATCH_LABEL, "Recovery subgraph on failure"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::Err,
        name: "HandleError",
        category: OperationCategory::ErrorHandling,
        description: "Invoke recovery template on failure; update Goals/Beliefs",
        long_description: "Handles an error by invoking a recovery template. The error handler \
            can update the agent's goals and beliefs to reflect the failure and adapt the agent's \
            strategy. Typically used inside TRY_CATCH catch subgraphs.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 4, "op": "ERR", "attributes": {"error_handler": "retry_with_fallback"}}"#,
        ),
        fields: &[OperationField::required(
            "error_handler", // structural
            "Error handler to invoke",
        )],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    // ========== Communication Operations (1) ==========
    OperationSpec {
        op_type: AISOperationType::Communicate,
        name: "Communicate",
        category: OperationCategory::Communication,
        description: "Send message to recipient agent via selected protocol",
        long_description: "Sends a message from this agent to another agent. The message is \
            received via input tokens from upstream edges. Supports four protocol dispatch modes: \
            'local' (default, in-process sub-flow), 'http'/'https' (external APXM agent), \
            'acp' (ACP subprocess via ProcessTable), and 'broadcast' (fan-out to all agents). \
            The recipient attribute key is 'recipient'.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 3, "op": "COMMUNICATE", "attributes": {"recipient": "reviewer", "protocol": "acp"}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::RECIPIENT,
                "Target agent name (or URL for http protocol)",
            ),
            OperationField::optional(attrs::MESSAGE, "Message content to send"),
            OperationField::optional(
                attrs::PROTOCOL,
                "Dispatch protocol: local (default), http, https, acp, broadcast",
            ),
            OperationField::optional(
                attrs::LLM_OPERATION,
                "Semantic LLM operation for analysis when this communication sends an agent prompt",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::MESSAGE),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[attrs::PROTOCOL],
            syntactic_keywords: &[(super::mlir_keywords::TO, attrs::RECIPIENT)],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Handoff,
        name: "Handoff",
        category: OperationCategory::Communication,
        description: "Hand off execution from one agent to another with optional state transfer",
        long_description: "Transfers execution control from a source agent to a target agent. \
            When transfer_state is true, the source agent's context-stack frames are copied to \
            the target agent so it can continue with full conversational context. Emits \
            HANDOFF_START and HANDOFF_END events with span continuity for tracing. The target \
            agent's response becomes this node's output token.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 3, "op": "HANDOFF", "attributes": {"handoff_from": "agent_a", "handoff_to": "agent_b", "transfer_state": true}}"#,
        ),
        fields: &[
            OperationField::required(attrs::HANDOFF_FROM, "Source agent name"),
            OperationField::required(attrs::HANDOFF_TO, "Target agent name"),
            OperationField::optional("payload", "Message payload to pass to the target agent"),
            OperationField::optional(
                attrs::TRANSFER_STATE,
                "Whether to copy context-stack frames from source to target (default: true)",
            ),
        ],
        needs_submission: true,
        min_inputs: 1,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::HANDOFF_FROM),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[attrs::TRANSFER_STATE],
            syntactic_keywords: &[(super::mlir_keywords::TO, attrs::HANDOFF_TO)],
        },
    },
    // ========== Coordination Extensions ==========
    OperationSpec {
        op_type: AISOperationType::UpdateGoal,
        name: "UpdateGoal",
        category: OperationCategory::Memory,
        description: "Modify AAM goals at runtime: set, remove, or clear",
        long_description: "Dynamically modifies the agent's goal set during execution. \
            Supports three actions: 'set' (upsert a goal with priority), 'remove' (delete \
            a specific goal), and 'clear' (remove all goals). Goal changes are visible to \
            subsequent REASON and REFLECT nodes.",
        latency: OperationLatency::None,
        example_json: Some(
            r#"{"id": 3, "op": "UPDATE_GOAL", "attributes": {"goal_id": "optimize_latency", "action": "set", "priority": 2}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::GOAL_ID,
                "Goal identifier (used as description key for upsert/remove)",
            ),
            OperationField::optional(
                attrs::ACTION,
                "Action to perform: set (default), remove, clear",
            ),
            OperationField::optional(attrs::PRIORITY, "Goal priority (u32, default: 1)"),
        ],
        needs_submission: false,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::Guard,
        name: "Guard",
        category: OperationCategory::ControlFlow,
        description: "Enforce preconditions: halt or skip based on condition",
        long_description: "Evaluates a condition expression against the input token. If the \
            condition fails, the guard either halts execution with an error or skips the \
            downstream subgraph (configurable via on_fail). Use to enforce invariants like \
            confidence thresholds, non-null checks, or content validation.",
        latency: OperationLatency::None,
        example_json: Some(
            r#"{"id": 3, "op": "GUARD", "attributes": {"condition": "> 0.8", "on_fail": "skip", "error_message": "Confidence too low"}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::CONDITION,
                "Condition expression: '> 0.8', '!= null', 'not_empty', etc.",
            ),
            OperationField::optional(attrs::ERROR_MESSAGE, "Message on failure"),
            OperationField::optional(attrs::ON_FAIL, "Failure mode: halt (default) or skip"),
        ],
        needs_submission: false,
        min_inputs: 1,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::Claim,
        name: "Claim",
        category: OperationCategory::Communication,
        description: "Atomically claim a task from a shared work queue via APXM server",
        long_description: "Claims a task from a distributed work queue managed by the APXM \
            server. The claim is atomic — only one agent gets each task. The claimed task \
            is leased for a configurable duration. If the agent doesn't complete within the \
            lease, the task returns to the queue for other agents.",
        latency: OperationLatency::Low,
        example_json: Some(
            r#"{"id": 2, "op": "CLAIM", "attributes": {"queue": "review_tasks", "lease_ms": 30000}}"#,
        ),
        fields: &[
            OperationField::required(attrs::QUEUE, "Queue name to claim from"),
            OperationField::optional(attrs::LEASE_MS, "Lease duration in ms (default: 60000)"),
            OperationField::optional(
                attrs::MAX_WAIT_MS,
                "Max time to wait for a task (default: 5000)",
            ),
            OperationField::optional(attrs::SERVER_URL, "Override APXM_SERVER_URL env var"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::Pause,
        name: "Pause",
        category: OperationCategory::Communication,
        description: "Suspend execution pending human-in-the-loop review via checkpoint",
        long_description: "Creates a checkpoint and suspends execution until a human resumes \
            it via the APXM server API. The pause message is displayed to the human reviewer. \
            Optionally sends a webhook notification. The human can provide input that becomes \
            this node's output token when RESUME is called.",
        latency: OperationLatency::High,
        example_json: Some(
            r#"{"id": 5, "op": "PAUSE", "attributes": {"message": "Please review the analysis before proceeding"}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::MESSAGE,
                "Human-readable message explaining the pause",
            ),
            OperationField::optional(
                attrs::CHECKPOINT_ID,
                "Stable checkpoint ID (auto-generated if omitted)",
            ),
            OperationField::optional(
                attrs::TIMEOUT_MS,
                "Max wait in ms (0 = indefinite, default: 0)",
            ),
            OperationField::optional(
                attrs::POLL_INTERVAL_MS,
                "Polling interval in ms (default: 2000)",
            ),
            OperationField::optional(
                attrs::NOTIFICATION_URL,
                "Webhook URL to notify on pause creation",
            ),
            OperationField::optional(attrs::SERVER_URL, "Override APXM_SERVER_URL env var"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::Resume,
        name: "Resume",
        category: OperationCategory::ControlFlow,
        description: "Resume a suspended PAUSE checkpoint; polls server until human resumes; returns human_input",
        long_description: "Polls the APXM server for a specific checkpoint until a human \
            resumes it. When resumed, the human's input (if any) becomes this node's output \
            token. Configurable polling interval and max attempts prevent indefinite blocking.",
        latency: OperationLatency::High,
        example_json: Some(
            r#"{"id": 6, "op": "RESUME", "attributes": {"checkpoint": "review_checkpoint_1"}}"#,
        ),
        fields: &[
            OperationField::required(attrs::CHECKPOINT, "Checkpoint ID to resume from"),
            OperationField::optional(
                attrs::POLL_MAX_ATTEMPTS,
                "Max polling attempts (default 60 × 5s = 5 min)",
            ),
            OperationField::optional(
                attrs::POLL_INTERVAL_MS,
                "Interval between polls in ms (default 5000)",
            ),
            OperationField::optional(attrs::SERVER_URL, "Override APXM_SERVER_URL env var"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    // ========== Multi-Agent Coordination Operations ==========
    OperationSpec {
        op_type: AISOperationType::Delegate,
        name: "Delegate",
        category: OperationCategory::Coordination,
        description: "Delegate a task to a sub-agent for execution",
        long_description: "Creates a sub-task from a task specification and assigns it to a \
            target agent. Returns a task handle that can be used to track the delegated work. \
            The target agent executes the task independently and reports results back.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{\"id\": 3, \"op\": \"DELEGATE\", \"attributes\": {\"task_spec\": \"Analyze the dataset\", \"target_agent\": \"analyst\"}}"#,
        ),
        fields: &[
            OperationField::required(attrs::TASK_SPEC, "Description of the task to delegate"),
            OperationField::required(attrs::TARGET_AGENT, "Name of the agent to delegate to"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::TASK_SPEC),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[],
            syntactic_keywords: &[(super::mlir_keywords::TO, attrs::TARGET_AGENT)],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Negotiate,
        name: "Negotiate",
        category: OperationCategory::Coordination,
        description: "Multi-agent negotiation protocol for consensus building",
        long_description: "Initiates a multi-party negotiation protocol among a set of agents. \
            A proposal is circulated to all parties for a configurable number of rounds. \
            Returns the consensus result or a timeout if no agreement is reached.",
        latency: OperationLatency::High,
        example_json: Some(
            r#"{\"id\": 4, \"op\": \"NEGOTIATE\", \"attributes\": {\"parties\": [\"agent_a\", \"agent_b\"], \"proposal\": \"Choose the best approach\", \"max_rounds\": 3}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::PARTIES,
                "List of agent names participating in negotiation",
            ),
            OperationField::required(attrs::PROPOSAL, "The proposal to negotiate on"),
            OperationField::optional(attrs::MAX_ROUNDS, "Maximum negotiation rounds (default: 3)"),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_NEGOTIATE,
    },
    // ========== Identity Operations (2) ==========
    OperationSpec {
        op_type: AISOperationType::Nop,
        name: "Nop",
        category: OperationCategory::Identity,
        description: "No-op passthrough with no side effects or AAM transition",
        long_description: "Pure passthrough operation with no side effects and no AAM state \
            transition. Passes through its first input unchanged, or returns Null if no inputs. \
            Useful as a placeholder, sync point, or structural node in graph composition.",
        latency: OperationLatency::None,
        example_json: Some(r#"{\"id\": 2, \"op\": \"NOP\"}"#),
        fields: &[],
        needs_submission: false,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    OperationSpec {
        op_type: AISOperationType::Identity,
        name: "Identity",
        category: OperationCategory::Identity,
        description: "Identity passthrough that records an AAM identity transition",
        long_description: "Like NOP but produces an AAM identity transition (state unchanged \
            but recorded in the execution trace). Useful for observability when you want to \
            mark a point in the graph without changing state.",
        latency: OperationLatency::None,
        example_json: Some(r#"{\"id\": 2, \"op\": \"IDENTITY\"}"#),
        fields: &[],
        needs_submission: false,
        min_inputs: 0,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
    // ========== Self-Organization Operations (2) ==========
    OperationSpec {
        op_type: AISOperationType::SpawnAgent,
        name: "SpawnAgent",
        category: OperationCategory::Coordination,
        description: "Create a new agent instance at runtime, optionally as an ACP subprocess",
        long_description: "Spawns a new agent instance. Without profile or agent_route, registers \
            a local process for flow-based agents. With profile, spawns that ACP profile. With \
            agent_route='auto', APXM selects an ACP profile from host-supplied route candidates \
            using required_capabilities and preferred_profiles. The agent can then receive \
            COMMUNICATE (protocol 'acp' or 'local') or DELEGATE messages. Returns the agent's \
            identifier and metadata.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 1, "op": "SPAWN_AGENT", "attributes": {"agent_name": "worker", "profile": "example-acp-profile", "mode": "architect"}}"#,
        ),
        fields: &[
            OperationField::required(attrs::AGENT_NAME, "Name for the new agent"),
            OperationField::optional_ref(
                attrs::PROFILE,
                "APXM ACP agent profile. When present, spawns an ACP subprocess",
                ReferenceType::Profile,
            ),
            OperationField::optional(
                attrs::AGENT_ROUTE,
                "Set to 'auto' to let APXM select an ACP profile when profile is omitted",
            ),
            OperationField::optional(
                attrs::REQUIRED_CAPABILITIES,
                "Abstract route capabilities required from the selected ACP profile",
            ),
            OperationField::optional(
                attrs::PREFERRED_PROFILES,
                "Preferred APXM ACP profiles used as a tie-breaker after capability fit",
            ),
            OperationField::optional(
                attrs::MODE,
                "Agent mode to set after spawn (e.g. 'architect', 'code')",
            ),
            OperationField::optional(
                attrs::MODEL,
                "Agent-specific model hint accepted by the selected ACP profile",
            ),
            OperationField::optional(
                attrs::CWD,
                "Working directory for the agent subprocess (defaults to current dir)",
            ),
            OperationField::optional("capabilities", "List of capabilities for the new agent"),
            OperationField::optional("goals", "Initial goals for the new agent"),
            OperationField::optional(
                attrs::SYSTEM_PROMPT,
                "System prompt / instructions for inline agents — enables HANDOFF/COMMUNICATE \
                 dispatch without registering the agent as a separate compiled flow",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::AGENT_NAME),
            context_style: ContextStyle::None,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[
                attrs::PROFILE,
                attrs::AGENT_ROUTE,
                attrs::REQUIRED_CAPABILITIES,
                attrs::PREFERRED_PROFILES,
                attrs::MODE,
                attrs::MODEL,
                attrs::CWD,
                attrs::SYSTEM_PROMPT,
            ],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::SpawnTeam,
        name: "SpawnTeam",
        category: OperationCategory::Coordination,
        description: "Spawn all members of a team (expands to N SPAWN_AGENT operations)",
        long_description: "Spawns all members of a named team definition from ~/.apxm/teams.toml. \
            Each member is spawned with its configured role, profile, and optional system_prompt. \
            Returns an object containing all spawned agent identifiers. Team definitions are loaded \
            from the TeamRegistry at runtime.",
        latency: OperationLatency::Medium,
        example_json: Some(
            r#"{"id": 1, "op": "SPAWN_TEAM", "attributes": {"team_name": "ultrathink", "cwd": "/path/to/project"}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::TEAM_NAME,
                "Name of the team to spawn (from ~/.apxm/teams.toml)",
            ),
            OperationField::optional(
                attrs::CWD,
                "Working directory for all team member subprocesses (defaults to current dir)",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::TEAM_NAME),
            context_style: ContextStyle::None,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[attrs::CWD],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::RegisterCapability,
        name: "RegisterCapability",
        category: OperationCategory::Coordination,
        description: "Register a new capability (tool) in the runtime registry",
        long_description: "Dynamically registers a new capability in the runtime's capability \
            registry. The capability becomes available for INV_TOOL operations after registration. \
            Returns a confirmation with the registered capability name.",
        latency: OperationLatency::Low,
        example_json: Some(
            r#"{\"id\": 3, \"op\": \"REGISTER_CAPABILITY\", \"attributes\": {\"capability_name\": \"custom_tool\", \"description\": \"A custom analysis tool\"}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::CAPABILITY_NAME,
                "Name for the capability to register",
            ),
            OperationField::optional(
                attrs::DESCRIPTION,
                "Human-readable description of the capability",
            ),
            OperationField::optional(
                attrs::PARAMETERS_SCHEMA,
                "JSON schema for capability parameters",
            ),
            OperationField::optional(
                attrs::PYTHON_HANDLER_ID,
                "Stable content-addressed id (sha256:<hex64>) for a Python-backed tool handler",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::CAPABILITY_NAME),
            context_style: ContextStyle::None,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[
                attrs::DESCRIPTION,
                attrs::PARAMETERS_SCHEMA,
                attrs::PYTHON_HANDLER_ID,
            ],
            syntactic_keywords: &[],
        },
    },
    // ========== Autonomous Execution ==========
    OperationSpec {
        op_type: AISOperationType::Autonomous,
        name: "Autonomous",
        category: OperationCategory::Coordination,
        description: "Run a goal-directed autonomous loop with the configured model",
        long_description: "Runs an iterative plan / act / evaluate loop against a goal prompt. \
            The node keeps calling the configured model until the goal is achieved or \
            `max_iterations` is reached. Optional backend, model, system prompt, provider, \
            and temperature attributes follow the same routing contract as the other LLM \
            operations.",
        latency: OperationLatency::High,
        example_json: Some(
            r#"{"id": 3, "op": "AUTONOMOUS", "attributes": {"prompt": "Find the root cause and propose a fix", "max_iterations": 6}}"#,
        ),
        fields: &[
            OperationField::required(attrs::PROMPT, "Goal or objective for the autonomous loop"),
            OperationField::optional(
                attrs::MAX_ITERATIONS,
                "Maximum number of plan / act / evaluate iterations before stopping",
            ),
            OperationField::optional_ref(
                attrs::BACKEND,
                "Backend override for the autonomous loop",
                ReferenceType::Backend,
            ),
            OperationField::optional_ref(
                attrs::MODEL,
                "Model override for the autonomous loop",
                ReferenceType::Model,
            ),
            OperationField::optional(
                attrs::PROVIDER,
                "Provider override when backend routing is not used",
            ),
            OperationField::optional(
                attrs::SYSTEM_PROMPT,
                "System prompt applied to each model call in the loop",
            ),
            OperationField::optional(
                attrs::TEMPERATURE,
                "Sampling temperature for the loop's model calls",
            ),
        ],
        needs_submission: true,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::PROMPT),
            context_style: ContextStyle::Parenthesized,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[
                attrs::MAX_ITERATIONS,
                attrs::BACKEND,
                attrs::MODEL,
                attrs::PROVIDER,
                attrs::SYSTEM_PROMPT,
                attrs::TEMPERATURE,
            ],
            syntactic_keywords: &[],
        },
    },
    // ========== Durable Execution (1) ==========
    OperationSpec {
        op_type: AISOperationType::Checkpoint,
        name: "Checkpoint",
        category: OperationCategory::Synchronization,
        description: "Create a durable execution checkpoint and emit a manifest token",
        long_description: "Serializes execution state at a barrier point and persists it under \
            a stable checkpoint identifier. The snapshot captures AAM state plus runtime \
            checkpoint metadata, and emits a manifest token containing the checkpoint id, \
            timestamp, and byte size so downstream nodes can reference the saved state. \
            Execution continues immediately after the snapshot — this is NOT a suspend point \
            (use PAUSE when you need human-gated suspension).",
        latency: OperationLatency::Low,
        example_json: Some(
            r#"{"id": 4, "op": "CHECKPOINT", "attributes": {"checkpoint_id": "before_analysis"}}"#,
        ),
        fields: &[
            OperationField::required(
                attrs::CHECKPOINT_ID,
                "Stable identifier for this checkpoint",
            ),
            OperationField::optional(attrs::SCOPE, "Snapshot scope: full (default) or local"),
            OperationField::optional(
                attrs::STORAGE,
                "Storage backend: fs (default), memory, or custom",
            ),
            OperationField::optional(
                attrs::TTL_SECONDS,
                "Time-to-live for the checkpoint in seconds",
            ),
            OperationField::optional(attrs::ON_FAIL, "Failure mode: halt (default) or continue"),
        ],
        needs_submission: false,
        min_inputs: 1,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::CHECKPOINT_ID),
            context_style: ContextStyle::None,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[],
            syntactic_keywords: &[],
        },
    },
    // ========== Internal Operations (2) ==========
    OperationSpec {
        op_type: AISOperationType::ConstStr,
        name: "ConstStr",
        category: OperationCategory::Internal,
        description: "String constant (compiler internal for string literals)",
        long_description: "Compiler-internal operation that produces a constant string value. \
            Not available in the public AIS. The compiler generates CONST_STR nodes when \
            lowering template strings to explicit dataflow.",
        latency: OperationLatency::None,
        example_json: None,
        fields: &[OperationField::required(
            attrs::VALUE,
            "The string constant value",
        )],
        needs_submission: false,
        min_inputs: 0,
        produces_output: true,
        emission: MlirEmissionSpec {
            primary_attr: Some(attrs::VALUE),
            context_style: ContextStyle::None,
            result_type: MlirResultType::Token,
            positional_attrs: &[],
            keywords: &[],
            syntactic_keywords: &[],
        },
    },
    OperationSpec {
        op_type: AISOperationType::Yield,
        name: "Yield",
        category: OperationCategory::Internal,
        description: "Yield value from switch case region (terminates region)",
        long_description: "Compiler-internal operation that terminates a switch case region \
            and yields a value to the parent SWITCH node. Not available in the public AIS.",
        latency: OperationLatency::None,
        example_json: None,
        fields: &[OperationField::required(
            attrs::VALUE,
            "The value to yield from the region",
        )],
        needs_submission: false,
        min_inputs: 1,
        produces_output: true,
        emission: EMISSION_TOKEN_BRACKETED,
    },
];

// ============================================================================
// Lookup Functions
// ============================================================================

/// Get the specification for an operation type.
pub fn get_operation_spec(op_type: AISOperationType) -> &'static OperationSpec {
    AIS_OPERATIONS
        .iter()
        .find(|s| s.op_type == op_type)
        .unwrap_or_else(|| {
            panic!(
                "Operation {:?} has no specification. This is a bug - all operations must have specs.",
                op_type
            )
        })
}

/// Get all operation specifications.
pub fn get_all_operations() -> impl Iterator<Item = &'static OperationSpec> {
    AIS_OPERATIONS.iter()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const ARTIFACT_OPERATION_KIND_ENTRIES_FILE: &str = "OperationKind.generated.inc";
    const ARTIFACT_OPERATION_KIND_CASES_FILE: &str = "OperationKindCases.generated.inc";
    const ARTIFACT_EMITTER_CPP: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../compiler/apxm-compiler/mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp"
    );
    const AIS_OPS_TD: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../compiler/apxm-compiler/mlir/include/ais/Dialect/AIS/IR/AISOps.td"
    );

    fn artifact_emitter_source() -> String {
        std::fs::read_to_string(ARTIFACT_EMITTER_CPP)
            .unwrap_or_else(|err| panic!("read {ARTIFACT_EMITTER_CPP}: {err}"))
    }

    fn ais_ops_td_source() -> String {
        std::fs::read_to_string(AIS_OPS_TD).unwrap_or_else(|err| panic!("read {AIS_OPS_TD}: {err}"))
    }

    fn tablegen_mnemonics(source: &str) -> HashSet<String> {
        source
            .lines()
            .filter_map(|line| line.split_once("AIS_Op<\""))
            .filter_map(|(_, rest)| rest.split_once('"'))
            .map(|(mnemonic, _)| mnemonic.to_string())
            .collect()
    }

    #[test]
    fn test_all_ops_have_specs() {
        for op_type in AISOperationType::all_operations() {
            let spec = get_operation_spec(*op_type);
            assert_eq!(spec.op_type, *op_type);
        }
    }

    #[test]
    fn test_operation_counts() {
        assert_eq!(
            AIS_OPERATIONS.len(),
            44,
            "Expected 44 total operations (1 metadata + 41 public + 2 internal)"
        );
        assert_eq!(
            AISOperationType::all_operations().len(),
            44,
            "Expected 44 total operation types"
        );
    }

    #[test]
    fn test_mlir_mnemonics() {
        assert_eq!(AISOperationType::Agent.mlir_mnemonic(), "agent");
        assert_eq!(AISOperationType::Ask.mlir_mnemonic(), "ask");
        assert_eq!(AISOperationType::Think.mlir_mnemonic(), "think");
        assert_eq!(AISOperationType::Reason.mlir_mnemonic(), "reason");
        assert_eq!(AISOperationType::QMem.mlir_mnemonic(), "qmem");
        assert_eq!(AISOperationType::WaitAll.mlir_mnemonic(), "wait_all");
    }

    #[test]
    fn test_from_wire_index() {
        // Spot-check key indices matching ArtifactEmitter.cpp OperationKind
        assert_eq!(
            AISOperationType::from_wire_index(0),
            Some(AISOperationType::InvTool)
        );
        assert_eq!(
            AISOperationType::from_wire_index(1),
            Some(AISOperationType::Ask)
        );
        assert_eq!(
            AISOperationType::from_wire_index(19),
            Some(AISOperationType::ConstStr)
        );
        assert_eq!(
            AISOperationType::from_wire_index(20),
            Some(AISOperationType::Switch)
        );
        assert_eq!(
            AISOperationType::from_wire_index(24),
            Some(AISOperationType::Reason)
        );
        assert_eq!(
            AISOperationType::from_wire_index(25),
            Some(AISOperationType::UpdateGoal)
        );
        assert_eq!(
            AISOperationType::from_wire_index(29),
            Some(AISOperationType::Resume)
        );
        // 30 is reserved (unassigned)
        assert_eq!(
            AISOperationType::from_wire_index(30),
            None,
            "Index 30 is unassigned and must return None"
        );
        // Multi-agent coordination wire indices (31-37)
        assert_eq!(
            AISOperationType::from_wire_index(31),
            Some(AISOperationType::Delegate)
        );
        assert_eq!(
            AISOperationType::from_wire_index(37),
            Some(AISOperationType::Autonomous)
        );
        // Durable execution
        assert_eq!(
            AISOperationType::from_wire_index(38),
            Some(AISOperationType::Checkpoint)
        );
        // Team operations
        assert_eq!(
            AISOperationType::from_wire_index(39),
            Some(AISOperationType::SpawnTeam)
        );
        // Handoff
        assert_eq!(
            AISOperationType::from_wire_index(40),
            Some(AISOperationType::Handoff)
        );
        // Workflow spawn
        assert_eq!(
            AISOperationType::from_wire_index(41),
            Some(AISOperationType::WorkflowSpawn)
        );
        // Skill linkage
        assert_eq!(
            AISOperationType::from_wire_index(42),
            Some(AISOperationType::CallSkill)
        );
        // Out-of-range returns None
        assert_eq!(AISOperationType::from_wire_index(u32::MAX), None);
    }

    #[test]
    fn test_wire_index_round_trip_coverage() {
        let mut seen_ops = HashSet::new();
        let mut seen_indexes = HashSet::new();
        for &(wire_index, expected_op) in AISOperationType::wire_indexed_operations() {
            assert!(
                seen_indexes.insert(wire_index),
                "duplicate wire index {wire_index}"
            );
            let op = AISOperationType::from_wire_index(wire_index)
                .unwrap_or_else(|| panic!("wire index {wire_index} should be {expected_op:?}"));
            assert_eq!(op, expected_op);
            assert!(
                seen_ops.insert(op),
                "from_wire_index({wire_index}) returned duplicate {op:?}"
            );
            assert_eq!(op.to_wire_index(), Some(wire_index));
        }
        assert_eq!(
            seen_ops.len(),
            AISOperationType::wire_indexed_operations().len(),
            "Every wire-indexed operation should map round-trip exactly once"
        );
    }

    #[test]
    fn test_wire_indexed_ops_subset_of_all_ops() {
        let all_ops: HashSet<AISOperationType> =
            AISOperationType::all_operations().iter().copied().collect();
        for &(wire_index, op) in AISOperationType::wire_indexed_operations() {
            assert!(
                all_ops.contains(&op),
                "Wire-indexed op {op:?} (index {wire_index}) is not in all_operations()"
            );
        }
    }

    #[test]
    fn artifact_emitter_consumes_generated_wire_fragments() {
        let source = artifact_emitter_source();
        for file_name in [
            ARTIFACT_OPERATION_KIND_ENTRIES_FILE,
            ARTIFACT_OPERATION_KIND_CASES_FILE,
        ] {
            let include = format!("#include \"ais/Dialect/AIS/Conversion/Artifact/{file_name}\"");
            assert!(
                source.contains(&include),
                "ArtifactEmitter.cpp must consume generated artifact wire fragment {file_name}"
            );
        }
    }

    #[test]
    fn mlir_tablegen_declares_all_rust_operations() {
        let source = ais_ops_td_source();
        let tablegen_ops = tablegen_mnemonics(&source);
        let rust_ops: HashSet<String> = AISOperationType::all_operations()
            .iter()
            .map(|op| op.mlir_mnemonic().to_string())
            .collect();

        assert_eq!(
            tablegen_ops, rust_ops,
            "AISOps.td declarations must match AISOperationType::all_operations()"
        );
    }

    #[test]
    fn operation_fields_reference_attrs_constants() {
        let all_names = attrs::ALL_ATTR_NAMES;
        for spec in AIS_OPERATIONS {
            for field in spec.fields {
                assert!(
                    all_names.contains(&field.name),
                    "Field '{}' on op {} not found in attrs::ALL_ATTR_NAMES",
                    field.name,
                    spec.name
                );
            }
        }
    }

    #[test]
    fn test_operation_type_parses_identifiers() {
        assert_eq!(
            "plan".parse::<AISOperationType>().unwrap(),
            AISOperationType::Plan
        );
        assert_eq!(
            "PLAN".parse::<AISOperationType>().unwrap(),
            AISOperationType::Plan
        );
        assert_eq!(
            "flow-call".parse::<AISOperationType>().unwrap(),
            AISOperationType::FlowCall
        );
        assert_eq!(
            "spawn_agent".parse::<AISOperationType>().unwrap(),
            AISOperationType::SpawnAgent
        );
        assert!("not_real".parse::<AISOperationType>().is_err());
    }
}
