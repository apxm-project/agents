//! Core type definitions for the APXM system, organized in three tiers.
//!
//! - **Foundation**: Primitive types, identifiers, and AAM model.
//! - **Execution**: DAG, node, edge, and task types for the scheduler.
//! - **Domain**: Compiler, LLM models, backends, providers, and sessions.

// ── Foundation ──────────────────────────────────────────────
pub mod aam;
pub mod goal;
pub mod identifiers;
pub mod operations;
pub mod values;

// ── Execution ──────────────────────────────────────────────
pub mod config;
pub mod execution;
pub mod graph_hints;
pub mod intents;

// ── Domain ─────────────────────────────────────────────────
pub mod backend;
pub mod compiler;
pub mod llm_control_plane;
pub mod model_spec;
pub mod models;
pub mod provider_spec;
pub mod session;

pub use aam::{
    AamContext, CapabilityProjection, CapabilityRecord, CompletionPolicy, GoalProjection, GoalTree,
    ScopePolicy, ScopeSpec,
};
pub use compiler::{
    CodegenOptions, CompilationStage, EmitFormat, OptimizationLevel, OptimizationTarget, PassInfo,
    PassMetadata, PipelineConfig, find_pass_metadata, list_pass_metadata, stage_rank,
};
pub use execution::{
    Agent, AgentFlow, AgentId, AgentMetadata, CapabilityDeclaration, DagMetadata, DependencyType,
    Edge, ExecutionDag, ExecutionStats, LatencyTierConfig, MemoryDeclaration, Node, NodeId,
    NodeMetadata, NodeStatus, OpStatus, Task, TaskDag, TaskId, TaskMetadata,
    WORKFLOW_SPAWN_PATH_TARGET_KINDS, WORKFLOW_TARGET_KIND_ARTIFACT_PATH,
    WORKFLOW_TARGET_KIND_GRAPH_PATH, WORKFLOW_TARGET_KIND_REGISTERED_FLOW,
    WORKFLOW_TARGET_KIND_WORKFLOW_PATH, WorkflowInvocation, WorkflowInvocationKind, WorkflowNode,
    WorkflowTarget,
};
pub use goal::{Goal, GoalId, GoalStatus};
pub use graph_hints::{
    ApxmGraphHints, CompilerHints, GraphBackendKind, GraphMetadata, GraphStatusSnapshot, NodeSpec,
    PinMode, PinPolicy, PriorityClass,
};
pub use identifiers::{
    BackendId, CapabilityName, CheckpointId, ExecutionId, MessageId, ModelId, NodeIdType, OpIdType,
    ProfileId, SessionId, TokenIdType, TraceId,
};
pub use intents::{
    Entity, EntityType, ExportFormat, InspectTarget, Intent, MemoryQueryType, ProgramBuildStep,
};
pub use llm_control_plane::{
    APXM_CONFIG_ENV_VAR, APXM_LLM_BACKEND_ENV_VAR, APXM_MODEL_ENV_VAR, APXM_USE_LLM_ENV_VAR,
    ApxmBackendFallbackConfig, ApxmCredentialConfig, ApxmCredentialsFile, ApxmLlmBackendConfig,
    ApxmLlmChatConfig, ApxmLlmConfigFile, ApxmLlmControlPlane, ApxmLlmRoutingConfig,
    ApxmModelAliasConfig, ApxmOperationRouteConfig, ApxmRegisteredModelConfig,
    ResolvedApxmBackendConfig, ResolvedApxmModelAlias, ResolvedApxmModelConfig,
};
pub use models::{
    FinishReason, LLMResponse, ModelCapabilities, ModelInfo, TimingBreakdown, TokenUsage, ToolCall,
    ToolResult,
};

pub use operations::metadata::{
    AIS_OPERATIONS, ContextStyle, MlirEmissionSpec, MlirResultType, OperationField,
    OperationLatency, OperationSpec, ReferenceType, ValidationError, get_all_operations,
    get_operation_spec,
};
pub use operations::{AISOperation, AISOperationType, OperationCategory, validate_operation};

pub use session::{CompletedNodeInfo, LiveSessionState, NodeInfo, SessionManifest, SessionStatus};
pub use values::{Number, Token, TokenId, TokenStatus, Value};

pub use backend::{BackendConfig, BackendType, DockerConfig, ModelConfig};
pub use config::InstructionConfig;
pub use model_spec::{
    BUILTIN_MODELS, BuiltinModelSpec, default_model_for_provider, models_for_provider,
    resolve_builtin_model,
};
pub use provider_spec::{
    BUILTIN_PROVIDERS, BuiltinProviderSpec, DEFAULT_VLLM_BASE_URL, ProviderProtocol, ProviderSpec,
    normalize_endpoint_for_protocol, resolve_builtin_provider, resolve_provider_spec,
};
