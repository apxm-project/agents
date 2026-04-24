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

pub mod aam;
pub mod agent_pool;
pub mod capability;
pub mod context_stack;
pub mod executor;
pub mod memory;
pub mod model_router;
pub mod observability;
pub mod process;
pub mod process_table;
pub mod python_tools;
mod runtime;
pub mod sandbox;
pub mod scheduler;
pub mod team;
pub mod thread;
pub mod vllm_attr_derivation;
pub mod vllm_lifecycle;
pub mod workflow;
pub mod workspace;

pub use aam::{
    Aam, AamCheckpoint, CapabilityRecord, Goal, GoalId, GoalStatus, STAGED_BELIEF_PREFIX,
    ScopePolicy, ScopeSpec, TransitionLabel,
    effects::{AamComponent, OperationEffects, operation_effects},
    session::SessionManager,
};
pub use agent_pool::{AgentPool, PoolStats, ProfileStats};
pub use capability::{
    CapabilitySystem,
    flow_registry::FlowRegistry,
    interceptor::{CapabilityInterceptor, InterceptDecision},
};
pub use context_stack::{
    ContextAssembly, ContextFrame, ContextScope, ContextStack, ContextStackConfig,
};
pub use executor::{
    CancellationToken, EmitterAdapter, ExecutionContext, ExecutionEvent, ExecutionEventEmitter,
    ExecutorEngine, InnerPlanLinker, LoopGuardMiddleware, Next, NoOpLinker, NoOpWorkflowSpawner,
    OperationMiddleware, TimeoutMiddleware, TokenUsageSummary, WorkflowSpawnResult,
    WorkflowSpawner,
};
pub use memory::{Fact, FactFilter, FactResult, MemoryConfig, MemorySpace, MemorySystem};
pub use model_router::{
    BackendHealth, CircuitBreakerConfig, CircuitState, ModelEntry, ModelRouter, ModelRouterConfig,
    OperationPolicy, RoutingDecision, RoutingTarget,
};
pub use observability::{MetricsCollector, SchedulerMetrics};
pub use process::{AgentProcess, ProcessId, ProcessKind, ProcessState};
pub use process_table::{AgentPrompter, AgentSpawner, ProcessTable};
pub use runtime::{Runtime, RuntimeConfig, RuntimeExecutionResult};
pub use scheduler::{DataflowScheduler, SchedulerConfig};
pub use thread::{AgentThread, ThreadId, ThreadState};
pub use vllm_lifecycle::VllmGraphLifecycle;

// Re-export sandbox interface for host applications
pub use sandbox::{
    DefaultBackend, ExecRequest, ExecResult, IsolationLevel, NodeSandboxReq, SandboxBackend,
    SandboxCapabilities, SandboxContext, SandboxError, SandboxRegistry, SandboxSelection,
    SecurityManifest, ValidationResult,
};

// Re-export Python tool bridge
pub use python_tools::{PythonToolBridge, PythonToolRegistry, PythonToolWorker};

pub type RuntimeResult<T> = std::result::Result<T, RuntimeError>;

// Re-export commonly used types
pub use apxm_core::{
    error::RuntimeError,
    types::{
        execution::{ExecutionDag, ExecutionStats, Node},
        values::Value,
    },
};
