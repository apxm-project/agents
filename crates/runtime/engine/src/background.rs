//! Typed completion for executions that continue after returning a parked state.

use apxm_core::error::RuntimeError;
use serde::{Deserialize, Serialize};
use tokio::task::{JoinError, JoinHandle};

/// Runtime task whose join failed while completing a parked execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundExecutionTask {
    /// One scheduler worker panicked or was aborted.
    SchedulerWorker,
    /// The scheduler-owned parked finalizer panicked or was aborted.
    SchedulerFinalizer,
    /// The runtime-owned lifecycle finalizer panicked or was aborted.
    RuntimeFinalizer,
}

/// Structured details for a failed background task join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundJoinFailure {
    /// Task layer that failed to join.
    pub task: BackgroundExecutionTask,
    /// Diagnostic supplied by Tokio's join error.
    pub message: String,
    /// Whether the task was aborted.
    pub cancelled: bool,
    /// Whether the task panicked.
    pub panicked: bool,
}

impl BackgroundJoinFailure {
    /// Preserve Tokio's typed cancellation and panic flags with task context.
    pub(crate) fn from_join_error(task: BackgroundExecutionTask, error: JoinError) -> Self {
        Self {
            task,
            message: error.to_string(),
            cancelled: error.is_cancelled(),
            panicked: error.is_panic(),
        }
    }
}

/// Terminal state of an execution that continued after its first park.
#[derive(Debug)]
pub enum BackgroundExecutionOutcome {
    /// The graph completed without failed nodes.
    Success,
    /// Runtime execution reached a typed domain error.
    DomainFailure {
        /// Original runtime error returned by the failed graph.
        error: RuntimeError,
    },
    /// Host or parent cancellation terminated execution.
    Cancellation,
    /// A task required to finish execution failed to join.
    JoinFailure {
        /// Structured join diagnostics for the failed task layer.
        failure: BackgroundJoinFailure,
    },
}

impl BackgroundExecutionOutcome {
    /// Whether the outcome represents successful graph completion.
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// Build the serializable projection hosts store with execution records.
    pub fn persisted(&self) -> PersistedBackgroundExecutionOutcome {
        match self {
            Self::Success => PersistedBackgroundExecutionOutcome::Success,
            Self::DomainFailure { error } => PersistedBackgroundExecutionOutcome::DomainFailure {
                error: error.to_value(),
            },
            Self::Cancellation => PersistedBackgroundExecutionOutcome::Cancellation,
            Self::JoinFailure { failure } => PersistedBackgroundExecutionOutcome::JoinFailure {
                failure: failure.clone(),
            },
        }
    }

    /// Separate cancellation from ordinary runtime domain errors.
    pub(crate) fn from_runtime_error(error: RuntimeError) -> Self {
        match error {
            RuntimeError::SchedulerCancelled => Self::Cancellation,
            error => Self::DomainFailure { error },
        }
    }

    /// Build a join-failure outcome without converting it to a domain error.
    pub(crate) fn join_failure(task: BackgroundExecutionTask, error: JoinError) -> Self {
        Self::JoinFailure {
            failure: BackgroundJoinFailure::from_join_error(task, error),
        }
    }
}

/// Serializable terminal state for host-owned execution records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PersistedBackgroundExecutionOutcome {
    /// The graph completed without failed nodes.
    Success,
    /// Runtime execution failed with its structured error payload.
    DomainFailure {
        /// Stable runtime error payload suitable for JSON persistence.
        error: serde_json::Value,
    },
    /// Host or parent cancellation terminated execution.
    Cancellation,
    /// A task required to finish execution failed to join.
    JoinFailure {
        /// Structured join diagnostics for the failed task layer.
        failure: BackgroundJoinFailure,
    },
}

/// Awaitable completion handle returned with a parked execution.
#[must_use = "wait for terminal outcome before closing host-owned execution state"]
pub struct BackgroundExecution {
    handle: JoinHandle<BackgroundExecutionOutcome>,
}

impl BackgroundExecution {
    /// Wrap the runtime-owned completion task.
    pub(crate) fn new(handle: JoinHandle<BackgroundExecutionOutcome>) -> Self {
        Self { handle }
    }

    /// Wait for true graph completion and classify finalizer join failure.
    pub async fn wait(self) -> BackgroundExecutionOutcome {
        match self.handle.await {
            Ok(outcome) => outcome,
            Err(error) => BackgroundExecutionOutcome::join_failure(
                BackgroundExecutionTask::RuntimeFinalizer,
                error,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_cancellation_has_its_own_outcome() {
        assert!(matches!(
            BackgroundExecutionOutcome::from_runtime_error(RuntimeError::SchedulerCancelled),
            BackgroundExecutionOutcome::Cancellation
        ));
    }

    #[tokio::test]
    async fn wait_classifies_runtime_finalizer_join_failure() {
        let handle: JoinHandle<BackgroundExecutionOutcome> =
            tokio::spawn(async { panic!("finalizer panic") });

        let outcome = BackgroundExecution::new(handle).wait().await;

        assert!(matches!(
            outcome,
            BackgroundExecutionOutcome::JoinFailure {
                failure: BackgroundJoinFailure {
                    task: BackgroundExecutionTask::RuntimeFinalizer,
                    panicked: true,
                    ..
                }
            }
        ));
    }
}

impl std::fmt::Debug for BackgroundExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundExecution")
            .finish_non_exhaustive()
    }
}
