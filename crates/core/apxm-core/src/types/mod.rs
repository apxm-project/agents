//! Core type definitions for the APXM system, organized in three tiers.
//!
//! - **Foundation**: Primitive types, identifiers, and AAM model.
//! - **Execution**: DAG, node, edge, and task types for the scheduler.
//! - **Domain**: Compiler, LLM models, backends, providers, and sessions.

// ── Foundation ──────────────────────────────────────────────
pub mod aam;
pub mod capability;
pub mod goal;
pub mod identifiers;
pub mod operations;
pub mod values;

// ── Execution ──────────────────────────────────────────────
pub mod communicate;
pub mod config;
pub mod execution;
pub mod graph_hints;
pub mod graph_metrics;
pub mod intents;
pub mod metrics;
pub mod orchestration;

// ── Domain ─────────────────────────────────────────────────
pub mod compiler;
pub mod models;
pub mod session;
pub mod source_format;

pub use aam::{
    AamContext, CapabilityProjection, CapabilityRecord, CompletionPolicy, GoalProjection, GoalTree,
    ScopePolicy, ScopeSpec,
};
pub use capability::{
    AuthMethod, CapabilityDefinition, CapabilityGrant, CapabilityMetadata,
    CapabilitySchemaError, CapabilityTemplateV1, CAPABILITY_DEFINITION_SCHEMA_V1,
    CAPABILITY_GRANT_SCHEMA_V1, CAPABILITY_TEMPLATE_SCHEMA_V1, Delegability, GrantProvenance,
    GrantStatus, Lifecycle, LifecycleBound, PermissionEffect, PermissionOperation,
    PermissionPolicy, PermissionRule, PermissionScope, PlannerVisibility, Principal,
    PrincipalKind, PromptMode, PromptPolicy, ResourceHandle, ResourceSelector, RoleAssignment,
    RoleDefinition, RuntimeLimits, RuntimeSurfaceLimit, RuntimeSurfaceMode, RuntimeSurfacePolicy,
    Sensitivity,     SubjectContext, SubjectSelector, ToolBinding, ToolBindingHandler, ToolBindingMetadata,
    RuntimeCapabilityGrant,
};
pub use communicate::{CommunicateProtocol, UnknownProtocol};
pub use compiler::{
    CodegenOptions, CompilationStage, EmitFormat, OptimizationLevel, OptimizationTarget, PassInfo,
    PassMetadata, PipelineConfig, find_pass_metadata, list_pass_metadata, stage_rank,
};
pub use execution::{
    Agent, AgentFlow, AgentId, AgentMetadata, CapabilityDeclaration, DagMetadata, DependencyType,
    Edge, ExecutionDag, ExecutionStats, GraphMetricAggregates, GraphMetricTotals,
    GraphMetricsSnapshot, LatencyTierConfig, MemoryDeclaration, Node, NodeId, NodeMetadata,
    NodeMetrics, NodeProcessMetrics, NodeStatus, ObservedCriticalPath, ObservedGraphMetrics,
    ObservedQueueWait, OpStatus, OperationMetric, OperationMetricTotals, ProcessMetricTotals,
    ProcessPromptMetric, ProcessSpawnMetric, SpawnedProcessKind, Task, TaskDag, TaskId,
    TaskMetadata, WORKFLOW_SPAWN_PATH_TARGET_KINDS, WORKFLOW_TARGET_KIND_AIR_PATH,
    WORKFLOW_TARGET_KIND_ARTIFACT_PATH, WORKFLOW_TARGET_KIND_REGISTERED_FLOW,
    WORKFLOW_TARGET_KIND_WORKFLOW_PATH, WorkflowInvocation, WorkflowInvocationKind, WorkflowNode,
    WorkflowTarget,
};
pub use goal::{
    GateStatus, GateVerdict, Goal, GoalDecision, GoalId, GoalStatus, decide as decide_goal,
};
pub use graph_hints::{
    ApxmGraphHints, BackendGraphCapabilities, CompilerHints, GraphBackendKind, GraphMetadata,
    GraphStatusSnapshot, NodeSpec, PinMode, PinPolicy, PriorityClass,
};
pub use graph_metrics::{LatencyClass, NodeGraphMetrics};
pub use identifiers::{
    BackendId, CapabilityName, CheckpointId, ExecutionId, MessageId, ModelId, NodeIdType, OpIdType,
    ProfileId, SessionId, TokenIdType, TraceId,
};
pub use intents::{
    Entity, EntityType, ExportFormat, InspectTarget, Intent, MemoryQueryType, ProgramBuildStep,
};
pub use metrics::{GraphStatusKey, MetricsLevel};
pub use models::{
    FinishReason, LLMResponse, ModelCapabilities, ModelInfo, TimingBreakdown, TokenUsage, ToolCall,
    ToolResult,
};
pub use orchestration::{
    OrchestrationStartStatus, OrchestrationTransport, OrchestrationWakeOutcome,
    OrchestrationWorkspaceCleanup, OrchestrationWorkspaceMode, UnknownOrchestrationTransport,
    UnknownOrchestrationWorkspaceCleanup, UnknownOrchestrationWorkspaceMode,
};

pub use operations::metadata::{
    AIS_OPERATIONS, ContextStyle, MlirEmissionSpec, MlirResultType, OperationField,
    OperationLatency, OperationSpec, ReferenceType, ValidationError, WIRE_INDEXED_OPERATIONS,
    get_all_operations, get_operation_spec,
};
pub use operations::{AISOperation, AISOperationType, OperationCategory, validate_operation};

pub use session::{CompletedNodeInfo, LiveSessionState, NodeInfo, SessionManifest, SessionStatus};
pub use source_format::{ApxmPathFormat, ArtifactFormat, GraphSourceFormat};
pub use values::{Number, Token, TokenId, TokenStatus, Value};

pub use config::InstructionConfig;
