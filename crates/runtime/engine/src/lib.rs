//! APxM Runtime - Production-ready execution engine for APxM programs
//!
//! The runtime provides:
//! - **Memory System**: Three-tier memory (STM, LTM, Episodic)
//! - **Executor**: Operation dispatch and execution
//! - **Observability**: Tracing and metrics integration
//!
//! # Example
//!
//! ```no_run
//! use apxm_runtime::{Runtime, RuntimeConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = RuntimeConfig::default();
//!     let runtime = Runtime::new(config).await?;
//!
//!     // Execute a DAG
//!     // let result = runtime.execute(dag).await?;
//!
//!     Ok(())
//! }
//! ```

#![allow(
    clippy::assigning_clones,
    clippy::cast_possible_wrap,
    clippy::collapsible_if,
    clippy::elidable_lifetime_names,
    clippy::explicit_iter_loop,
    clippy::format_push_string,
    clippy::if_not_else,
    clippy::ignored_unit_patterns,
    clippy::implicit_clone,
    clippy::manual_let_else,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::needless_borrow,
    clippy::needless_continue,
    clippy::needless_return,
    clippy::non_std_lazy_statics,
    clippy::redundant_else,
    clippy::ref_option,
    clippy::semicolon_if_nothing_returned,
    clippy::single_char_add_str,
    clippy::too_many_arguments,
    clippy::unnecessary_literal_bound,
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    clippy::unused_self,
    clippy::unwrap_or_default
)]

/// Agent Abstract Machine — re-exported from the standalone `apxm-aam` crate
/// so existing `apxm_runtime::aam::...` import paths keep working after the
///  sub-crate extraction.
pub use apxm_aam as aam;
pub mod agent_pool;
pub mod agent_router;
pub mod agent_scoring;
/// Capability system — re-exported from the standalone `apxm-capability`
/// crate so existing `apxm_runtime::capability::...` import paths keep
/// working after the  sub-crate extraction.
pub use apxm_capability as capability;
mod constants;
pub mod context_stack;
mod dispatch;
pub mod executor;
pub mod flow_names;
pub mod graph_lifecycle;
pub mod host_dispatch;
/// Three-tier memory system — re-exported from the standalone `apxm-memory`
/// crate so existing `apxm_runtime::memory::...` import paths keep working
/// after the  sub-crate extraction.
pub use apxm_memory as memory;
pub mod metadata_keys;
pub mod model_router;
pub mod observability;
pub mod process;
pub mod process_table;
pub mod python_tools;
mod runtime;
/// Shared script-artifact admission policy (`python_tools` /
/// `typescript_tools` trust+sandbox gate) — consulted identically by this
/// crate's `Runtime`, the driver's attach step, and the server's raw execute
/// admission so all three embeddings enforce one policy. See
/// `the shared script-artifact admission policy` in the coordinator workspace.
pub mod script_admission;
pub mod typescript_tools;
// `sandbox` moved to `apxm-capability-iface` — it had zero dependencies on
// other `apxm-runtime` internals, so it was a clean relocation. Re-exported
// under the same module name so `crate::sandbox::*` and
// `apxm_runtime::sandbox::*` (external consumers: driver, acp, cli,
// observability) keep working unchanged.
pub use apxm_capability_iface::sandbox;
pub mod scheduler;
pub mod team;
#[cfg(any(test, feature = "test-utils"))]
pub mod testing;
pub mod thread;
pub mod workflow;
pub mod workspace;

pub use aam::{
    Aam, AamCheckpoint, CapabilityRecord, Goal, GoalId, GoalStatus, STAGED_BELIEF_PREFIX,
    ScopePolicy, ScopeSpec, TransitionLabel,
    effects::{AamComponent, OperationEffects, operation_effects},
    session::SessionManager,
};
pub use agent_pool::{AgentPool, PoolStats, ProfileStats};
pub use agent_router::{
    AGENT_ROUTE_CAPABILITIES, AGENT_ROUTE_SELECTOR_DETERMINISTIC, AgentRouteCandidate,
    AgentRouteDecision, AgentRouteRejection, AgentRouteRequest, AgentRouteScore, AgentRouteSource,
    AgentRouter, AgentRoutingError,
};
pub use capability::{
    CapabilitySystem,
    flow_registry::FlowRegistry,
    interceptor::{
        CapabilityInterceptor, InterceptDecision, PermissionInterceptor, PreInvokeContext,
        pre_invoke_ctx,
    },
};
pub use context_stack::{
    ContextAssembly, ContextDisposition, ContextFrame, ContextPermissionScope, ContextPlan,
    ContextPlanSegment, ContextScope, ContextSensitivity, ContextStack, ContextStackConfig,
};
pub use executor::{
    CallSkillRequest, CallSkillResult, CancellationToken, CapabilityEffectReceiptPayload,
    ConversationMemoryMiddleware, EmitterAdapter, ExecutionContext, ExecutionEvent,
    ExecutionEventEmitter, ExecutionHook, ExecutionHookContext, ExecutorEngine,
    GraphFinishedEvent, GraphMetricsTracker, GraphStartedEvent, InnerPlanLinker,
    LoopGuardMiddleware, ModelContextCallKind, ModelContextMetrics, ModelContextPlanStatus, Next,
    NoOpLinker, NoOpSkillResolver, NoOpWorkflowSpawner, NodeFinishedEvent, NodeReadyEvent,
    NodeStartedEvent, OperationMiddleware, SkillResolver, TimeoutMiddleware,
    TokenBudgetMiddleware, TokenUsageSummary, WorkflowSpawnResult, WorkflowSpawner,
};
pub use graph_lifecycle::BackendGraphLifecycle;
pub use memory::{MemoryConfig, MemorySpace, MemorySystem, parse_memory_space};
pub use model_router::{
    BackendHealth, CircuitBreakerConfig, CircuitState, ModelEntry, ModelRouter, ModelRouterConfig,
    OperationPolicy, RoutingDecision, RoutingTarget,
};
pub use observability::{MetricsCollector, SchedulerMetrics};
pub use process::{AgentProcess, ProcessId, ProcessKind, ProcessState};
pub use process_table::{
    AgentPromptResponse, AgentPromptTokenUsage, AgentPrompter, AgentSpawner, ProcessTable,
};
pub use runtime::{
    ExecutionOutcome, LlmToolDispatchConfig, Runtime, RuntimeConfig, RuntimeExecutionResult,
};
pub use scheduler::{DataflowScheduler, SchedulerConfig};
pub use script_admission::{script_artifacts_trusted, script_sandbox_required};
pub use thread::{AgentThread, ThreadId, ThreadState};

pub use sandbox::{
    DefaultBackend, ExecRequest, ExecResult, IsolationLevel, NodeSandboxReq, SandboxBackend,
    SandboxCapabilities, SandboxContext, SandboxError, SandboxRegistry, SandboxSelection,
    SecurityManifest, ValidationResult,
};

pub use python_tools::{PythonHandlerBridge, PythonHandlerRegistry, PythonHandlerWorker};
pub use typescript_tools::{
    TypeScriptHandlerBridge, TypeScriptHandlerRegistry, TypeScriptHandlerWorker,
};

pub use host_dispatch::{
    AgentChannelHandle, HostDispatchError, HostDispatchGateway, HostProxyRequest, HostProxyResult,
    HostToolCall, HostToolError, HostToolResult, NoOpHostDispatchGateway, SpawnOffer,
};

pub type RuntimeResult<T> = std::result::Result<T, RuntimeError>;

pub use apxm_core::{
    error::RuntimeError,
    types::{
        execution::{ExecutionDag, ExecutionStats, Node},
        values::Value,
    },
};
