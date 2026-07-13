//! Dataflow Scheduler.
//!
//! This module implements a work-stealing dataflow scheduler with:
//! - Token-based automatic parallelism
//! - 4-level priority queues (Critical, High, Normal, Low)
//! - O(1) readiness tracking
//! - Exponential backoff retry logic
//! - Deadlock detection
//! - Semaphore-based backpressure

pub mod admission_registry;
pub mod concurrency_control;
pub mod config;
pub mod context_view;
pub mod dataflow;
pub mod lane_queue;
pub mod park_registry;
pub mod queue;
pub mod ready_set;
pub mod replay;
pub mod snapshot;
pub mod splicing;
pub mod state;
pub mod work_stealing;
pub mod worker;

// Internal state types (not part of public API)
pub(crate) mod internal_state;

// Public exports
pub use config::SchedulerConfig;
pub use context_view::SchedulerCtx;
pub use dataflow::{DataflowScheduler, SchedulerOutcome};
pub use lane_queue::SessionLaneGuard;
pub use queue::{Priority, PriorityQueue};
pub use replay::ReplaySeed;
pub use snapshot::{
    SchedulerSnapshot, SchedulerSnapshotCounters, SchedulerSnapshotDelegatedToken,
    SchedulerSnapshotEdge, SchedulerSnapshotExecutionFrame, SchedulerSnapshotNodeOutputs,
    SchedulerSnapshotOp, SchedulerSnapshotPendingInput, SchedulerSnapshotPromise,
    SchedulerSnapshotToken,
};
pub use splicing::SpliceConfig;

pub use apxm_core::types::{ExecutionStats, NodeStatus, OpStatus};
