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
    CodegenOptions, CompilationStage, EmitFormat, OptimizationLevel, OptimizationTarget,
    PipelineConfig, stage_rank,
};
pub use execution::{
    Agent, AgentFlow, AgentId, AgentMetadata, CapabilityDeclaration, DagMetadata, DependencyType,
    Edge, ExecutionDag, ExecutionStats, LatencyTierConfig, MemoryDeclaration, Node, NodeId,
    NodeMetadata, NodeStatus, OpStatus, Task, TaskDag, TaskId, TaskMetadata, WorkflowNode,
};
pub use goal::{Goal, GoalId, GoalStatus};
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
    FinishReason, LLMResponse, ModelCapabilities, ModelInfo, TokenUsage, ToolCall, ToolResult,
};

// Re-export from operations (which re-exports from apxm-ais)
pub use operations::metadata::{
    AIS_OPERATIONS, OperationField, OperationLatency, OperationSpec, ReferenceType,
    ValidationError, get_operation_spec,
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
    BUILTIN_PROVIDERS, BuiltinProviderSpec, ProviderProtocol, ProviderSpec,
    resolve_builtin_provider, resolve_provider_spec,
};
