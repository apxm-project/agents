//! Core type definitions for the APXM system, organized in three tiers.
//!
//! - **Foundation**: Primitive types and identifiers.
//! - **Execution**: DAG, node, edge, and task types for the scheduler.
//! - **Domain**: Compiler, LLM models, backends, providers, and sessions.

// ── Foundation ──────────────────────────────────────────────
pub mod capability;
pub mod conformance;
pub mod consent;
pub mod goal;
pub mod host;
pub mod identifiers;
pub mod operations;
pub mod principal;
pub mod values;

// ── Execution ──────────────────────────────────────────────
pub mod communicate;
pub mod config;
#[path = "generated_context_contracts.rs"]
pub mod context_contracts;
pub mod conversation;
pub mod execution;
pub mod graph_hints;
pub mod graph_metrics;
pub mod handler_manifest;
pub mod intents;
pub mod metrics;

// ── Domain ─────────────────────────────────────────────────
pub mod compiler;
pub mod models;
pub mod source_format;

pub use capability::{
    CapabilitySchemaError, GrantStatus, PermissionDecision, PermissionOperation, PermissionScope,
    ResourceHandle, RuntimeCapabilityGrant, RuntimeLimits, RuntimeSurfaceLimit, RuntimeSurfaceMode,
    RuntimeSurfacePolicy,
};
pub use communicate::{CommunicateProtocol, UnknownProtocol};
pub use compiler::{
    CodegenOptions, CompilationStage, EmitFormat, OptimizationLevel, OptimizationTarget, PassInfo,
    PassMetadata, PipelineConfig, find_pass_metadata, list_pass_metadata, stage_rank,
};
pub use conversation::{
    ConversationMessageContext, ConversationMessageInput, ConversationMessageInputError,
};
pub use execution::{
    Agent, AgentFlow, AgentId, AgentMetadata, CapabilityDeclaration, ChildExecutionAdmission,
    DagMetadata, DependencyType, Edge, ExecutionDag, ExecutionStats, GraphMetricAggregates,
    GraphMetricTotals, GraphMetricsSnapshot, LatencyTierConfig, MemoryDeclaration, Node, NodeId,
    NodeMetadata, NodeMetrics, NodeProcessMetrics, NodeStatus, ObservedCriticalPath,
    ObservedGraphMetrics, ObservedQueueWait, OpStatus, OperationMetric, OperationMetricTotals,
    ProcessMetricTotals, ProcessPromptMetric, ProcessSpawnMetric, SpawnedProcessKind, Task,
    TaskDag, TaskId, TaskMetadata, WORKFLOW_SPAWN_PATH_TARGET_KINDS, WORKFLOW_TARGET_KIND_AIR_PATH,
    WORKFLOW_TARGET_KIND_ARTIFACT_PATH, WORKFLOW_TARGET_KIND_REGISTERED_FLOW,
    WORKFLOW_TARGET_KIND_WORKFLOW_PATH, WorkflowInvocation, WorkflowInvocationKind, WorkflowNode,
    WorkflowTarget,
};
pub use goal::{Goal, GoalId, GoalStatus};
pub use graph_hints::{
    ApxmGraphDescriptor, ApxmGraphDescriptorNode, ApxmGraphHints, BackendMechanismRef,
    GRAPH_HINTS_SCHEMA, GraphExecutionIntents, GraphHintCapabilities, GraphHintDispatchProjection,
    GraphHintField, GraphHintFieldCapability, GraphHintPlan, GraphHintProjection,
    GraphHintProjector, GraphHintScope, GraphLifecycleCapability, GraphMetadata,
    GraphPreparationRef, GraphPrepareOutcome, GraphReleaseOutcome, GraphStatusSnapshot,
    LifecycleReasonCode, MAX_BENEFIT_HORIZON_MS, MAX_ESTIMATED_TOKENS, MAX_EXPECTED_USES,
    MAX_OPAQUE_REF_LEN, MAX_PATH_COORDINATE, MAX_SUCCESSOR_REFS, NodeGraphFacts, NodeSpec,
    OptimizationObjective, ProjectionOutcome, ReasonCode, ReusableContextIntent, ReusePreference,
    WorkClass,
};
pub use graph_metrics::{LatencyClass, NodeGraphMetrics};
pub use handler_manifest::{
    HANDLER_MANIFEST_AIR_SIDECAR_PREFIX, HANDLER_MANIFEST_ARTIFACT_SECTION,
    HANDLER_MANIFEST_HANDLER_ID_HEX_LENGTH, HANDLER_MANIFEST_HANDLER_ID_PREFIX,
    HANDLER_MANIFEST_IDENTIFIER_MAX_LENGTH, HANDLER_MANIFEST_SOURCE_DIRECTORY,
    HANDLER_MANIFEST_VERSION, HandlerDescriptor, HandlerKind, HandlerLanguage, HandlerManifest,
    HandlerManifestError, HandlerSource,
};
pub use identifiers::{
    BackendId, CapabilityName, ExecutionId, ModelId, NodeIdType, OpIdType, ProfileId, TokenIdType,
    TraceId,
};
pub use intents::{
    Entity, EntityType, ExportFormat, InspectTarget, Intent, MemoryQueryType, ProgramBuildStep,
};
pub use metrics::MetricsLevel;
pub use models::{
    FinishReason, LLMResponse, ModelCapabilities, ModelInfo, TimingBreakdown, TokenUsage, ToolCall,
    ToolResult,
};
pub use operations::metadata::{
    AIS_OPERATIONS, ContextStyle, MlirEmissionSpec, MlirResultType, OperationField,
    OperationLatency, OperationSpec, ReferenceType, SemanticOpKind, StructuralOpKind,
    ValidationError, WIRE_INDEXED_OPERATIONS, get_all_operations, get_operation_spec,
};
pub use operations::{AISOperation, AISOperationType, OperationCategory, validate_operation};

pub use source_format::{ApxmPathFormat, ArtifactFormat, GraphSourceFormat};
pub use values::{Number, Token, TokenId, TokenStatus, Value};

pub use config::InstructionConfig;
