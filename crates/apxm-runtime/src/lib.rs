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
pub mod capability;
pub mod executor;
pub mod memory;
pub mod observability;
mod runtime;
pub mod sandbox;
pub mod scheduler;
pub mod process;
pub mod process_table;
pub mod thread;
pub mod workspace;

pub use aam::{
    Aam, AamCheckpoint, CapabilityRecord, Goal, GoalId, GoalStatus, STAGED_BELIEF_PREFIX,
    ScopePolicy, ScopeSpec, TransitionLabel,
    effects::{AamComponent, OperationEffects, operation_effects},
    session::SessionManager,
};
pub use capability::{
    CapabilitySystem,
    flow_registry::FlowRegistry,
    interceptor::{CapabilityInterceptor, InterceptDecision},
};
pub use executor::{
    CancellationToken, ExecutionContext, ExecutionEvent, ExecutionEventEmitter, ExecutorEngine,
    InnerPlanLinker, NoOpLinker,
};
pub use memory::{Fact, FactFilter, FactResult, MemoryConfig, MemorySpace, MemorySystem};
pub use observability::{MetricsCollector, SchedulerMetrics};
pub use runtime::{Runtime, RuntimeConfig, RuntimeExecutionResult};
pub use process::{AgentProcess, ProcessId, ProcessKind, ProcessState};
pub use process_table::{AgentPrompter, AgentSpawner, ProcessTable};
pub use scheduler::{DataflowScheduler, SchedulerConfig};
pub use thread::{AgentThread, ThreadId, ThreadState};

// Re-export sandbox interface for host applications
pub use apxm_sandbox;

pub type RuntimeResult<T> = std::result::Result<T, RuntimeError>;

// Re-export commonly used types
pub use apxm_core::{
    error::RuntimeError,
    types::{
        execution::{ExecutionDag, ExecutionStats, Node},
        values::Value,
    },
};
