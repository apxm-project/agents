//! Executor module - Orchestrates operation execution
//!
//! The executor is responsible for:
//! - Dispatching operations to appropriate handlers
//! - Managing execution context
//! - Coordinating subsystems (memory, models, etc.)

pub mod agent_scope;
mod cancellation;
pub(crate) mod capability_admission;
mod context;
pub(crate) mod correlated_batch;
pub mod dag_splicer;
mod dispatcher;
pub mod emitter_adapter;
mod engine;
mod events;
pub mod fields_honored;
pub mod graph_metrics;
mod handlers;
pub mod hook_driver;
pub mod hooks;
pub mod inner_plan_linker;
pub mod memoization;
mod middleware;
pub mod middlewares;
pub mod op_usage;
pub mod pipeline;
pub mod session_ledger;
pub mod timing_tracker;
pub mod token_accounting;
pub mod workflow_spawner;

pub use agent_scope::{AgentScope, AgentScopeStack};
pub use cancellation::CancellationToken;
pub use context::ExecutionContext;
pub use dag_splicer::{DagSplicer, NoOpSplicer};
pub use dispatcher::OperationDispatcher;
pub use emitter_adapter::EmitterAdapter;
pub use engine::{ExecutionResult, ExecutorEngine};
pub use events::{
    CapabilityEffectReceiptPayload, EventScopeState, ExecutionEvent, ExecutionEventEmitter,
    ModelContextCallKind, ModelContextMetrics, ModelContextPlanStatus,
};
pub use graph_metrics::GraphMetricsTracker;
pub use handlers::warmup::{WarmupConfig, WarmupMetrics};
pub use hooks::{
    ExecutionHook, ExecutionHookContext, GraphFinishedEvent, GraphStartedEvent, HookBinding,
    HookEvent, HookMode, HookRegistry, NodeFinishedEvent, NodeReadyEvent, NodeStartedEvent,
};
pub use inner_plan_linker::{InnerPlanLinker, NoOpLinker};
pub use middleware::{Next, OperationMiddleware};
pub use middlewares::{LoopGuardMiddleware, TimeoutMiddleware, TokenBudgetMiddleware};
pub use timing_tracker::TimingTracker;
pub use token_accounting::{TokenAccountant, TokenUsageSummary};
pub use workflow_spawner::{NoOpWorkflowSpawner, WorkflowSpawnResult, WorkflowSpawner};

use apxm_core::error::RuntimeError;

pub type Result<T> = std::result::Result<T, RuntimeError>;
