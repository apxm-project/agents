//! Shared core crate providing canonical types, error definitions, and constants
//! used across both the APXM compiler and runtime.
//!
//! - **`types`** -- Execution graph primitives (`Node`, `Edge`, `ExecutionDag`),
//!   value representations (`Value`, `Token`, `Number`), compiler options,
//!   session/message models, and provider specifications.
//! - **`error`** -- Structured error types (`RuntimeError`, `CompilerError`,
//!   `CompileError`, `CliError`, `SecurityError`) with error codes, source
//!   locations, and diagnostic suggestions.
//! - **`constants`** -- Centralised string keys for graph attributes, inner-plan
//!   payloads, and diagnostic modes so all front-ends and back-ends stay in sync.

pub mod agent_profile;
pub mod constants;
pub mod env;
pub mod error;
pub mod events;
pub mod logging;
pub mod metrics;
pub mod model_profiles;
pub mod observability;
pub mod paths;
pub mod plan;
pub mod toolchain_env;
pub mod types;
pub mod utils;

pub use error::{
    cli::{CliError, CliResult},
    common::{ErrorContext, OpId, SourceLocation, TraceId},
    compile::CompileError,
    compiler::CompilerError,
    runtime::RuntimeError,
    security::SecurityError,
};

pub use metrics::{MetricsReport, MetricsSource};
pub use plan::{InnerPlanPayload, Plan, PlanStep};

pub use types::{
    AISOperation, AISOperationType, ApxmGraphHints, ApxmPathFormat, ArtifactFormat,
    CapabilityRecord, CompilerHints, CompletionPolicy, DependencyType, Edge, GoalTree,
    GraphMetadata, GraphSourceFormat, InstructionConfig, LatencyClass, Node, NodeGraphMetrics,
    NodeId, NodeMetadata, NodeSpec, Number, PinMode, PinPolicy, PriorityClass, ScopePolicy,
    ScopeSpec, Token, TokenId, TokenStatus, Value,
};

pub use types::host;
pub use types::principal;
