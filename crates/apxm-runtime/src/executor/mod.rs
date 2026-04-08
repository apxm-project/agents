//! Executor module - Orchestrates operation execution
//!
//! The executor is responsible for:
//! - Dispatching operations to appropriate handlers
//! - Managing execution context
//! - Coordinating subsystems (memory, models, etc.)

mod cancellation;
mod context;
pub mod dag_splicer;
mod dispatcher;
mod engine;
mod events;
mod handlers;
pub mod inner_plan_linker;
pub mod memoization;
pub mod pipeline;
pub mod token_accounting;

pub use cancellation::CancellationToken;
pub use context::ExecutionContext;
pub use dag_splicer::{DagSplicer, NoOpSplicer};
pub use dispatcher::OperationDispatcher;
pub use engine::{ExecutionResult, ExecutorEngine};
pub use events::{ExecutionEvent, ExecutionEventEmitter};
pub use handlers::warmup::{WarmupConfig, WarmupMetrics};
pub use inner_plan_linker::{InnerPlanLinker, NoOpLinker};
pub use memoization::ResponseCache;
pub use token_accounting::TokenAccountant;

use apxm_core::error::RuntimeError;

pub type Result<T> = std::result::Result<T, RuntimeError>;
