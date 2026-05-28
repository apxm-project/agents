use std::sync::Arc;
use std::time::{Duration, SystemTime};

use apxm_core::events::{ApxmEvent, EventCategory, EventKind};
use apxm_core::impl_event_payload;
use apxm_rollout::{IndexDb, RolloutPaths};
use apxm_runtime::Runtime;
use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc};

use crate::a2a::A2aTaskRecord;
use crate::agent::AgentRegistration;
use crate::checkpoints::CheckpointStore;
use crate::execute::ExecuteResponse;
use crate::executions::ExecutionStore;
use crate::rollout::RolloutRegistry;
use crate::runs::RunEventBus;
use crate::skills::SkillLibrary;
use crate::tasks::TaskQueueManager;
use crate::webhook::WebhookDispatcher;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) runtime: Arc<Runtime>,
    /// In-memory agent registry: name → registration record
    pub(crate) agent_registry: Arc<DashMap<String, AgentRegistration>>,
    /// Task queue manager backing the CLAIM op.
    pub(crate) task_manager: TaskQueueManager,
    /// Checkpoint store for PAUSE/RESUME HITL workflow.
    pub(crate) checkpoint_store: CheckpointStore,
    /// Server start time for uptime reporting.
    pub(crate) start_time: SystemTime,
    /// In-flight A2A task records (task_id → record).
    pub(crate) a2a_tasks: Arc<DashMap<String, A2aTaskRecord>>,
    /// Server-owned APXM skill inventory.
    pub(crate) skill_library: SkillLibrary,
    /// In-memory execution records for server-owned skill runs.
    pub(crate) execution_store: ExecutionStore,
    /// Per-execution event log + live broadcast (Phase 14.8.B).
    pub(crate) run_event_bus: RunEventBus,
    /// Optional outbound lifecycle webhook dispatcher (Phase 14.8.C).
    pub(crate) webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
    /// Filesystem resolver for rollout JSONL transcripts (Phase 14.8.E).
    pub(crate) rollout_paths: Arc<RolloutPaths>,
    /// Thread index sidecar — derived view, fast list+lookup.
    pub(crate) rollout_index: Arc<Mutex<IndexDb>>,
    /// Open rollout recorders keyed by execution_id.
    pub(crate) rollout_registry: RolloutRegistry,
    /// Process-local cap for expensive inference/runtime work.
    pub(crate) inference_limiter: InferenceLimiter,
}

#[derive(Clone)]
pub(crate) struct InferenceLimiter {
    semaphore: Arc<Semaphore>,
    acquire_timeout: std::time::Duration,
}

impl InferenceLimiter {
    const DEFAULT_MAX_CONCURRENT: usize = 2;
    const DEFAULT_ACQUIRE_TIMEOUT_MS: u64 = 250;

    pub(crate) fn from_env() -> Self {
        let max_concurrent = std::env::var("APXM_SERVER_MAX_INFERENCE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(Self::DEFAULT_MAX_CONCURRENT);
        let acquire_timeout = std::env::var("APXM_SERVER_INFERENCE_WAIT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_millis(Self::DEFAULT_ACQUIRE_TIMEOUT_MS));

        Self::with_limits(max_concurrent, acquire_timeout)
    }

    fn with_limits(max_concurrent: usize, acquire_timeout: Duration) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(
                max_concurrent.clamp(1, Semaphore::MAX_PERMITS),
            )),
            acquire_timeout,
        }
    }

    #[cfg(test)]
    pub(crate) fn unlimited_for_tests() -> Self {
        Self::with_limits(Semaphore::MAX_PERMITS, Duration::from_secs(30))
    }

    #[cfg(test)]
    pub(crate) fn limited_for_tests(max_concurrent: usize, acquire_timeout: Duration) -> Self {
        Self::with_limits(max_concurrent, acquire_timeout)
    }

    pub(crate) async fn acquire(&self) -> Result<InferencePermit, crate::error::ApiError> {
        match tokio::time::timeout(self.acquire_timeout, self.semaphore.clone().acquire_owned())
            .await
        {
            Ok(Ok(permit)) => Ok(InferencePermit { _permit: permit }),
            Ok(Err(_)) => Err(crate::error::ApiError::internal_message(
                "inference limiter is closed",
            )),
            Err(_) => Err(crate::error::ApiError::too_many_requests(
                "server inference capacity is saturated",
            )),
        }
    }
}

pub(crate) struct InferencePermit {
    _permit: OwnedSemaphorePermit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inference_limiter_returns_429_when_saturated() {
        let limiter = InferenceLimiter::limited_for_tests(1, Duration::from_millis(1));
        let _first = limiter.acquire().await.expect("first permit");

        let error = match limiter.acquire().await {
            Ok(_) => panic!("second permit should time out"),
            Err(error) => error,
        };

        assert_eq!(error.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert!(error.message.contains("capacity"));
    }

    #[test]
    fn inference_limiter_clamps_excessive_limits() {
        let limiter = InferenceLimiter::limited_for_tests(usize::MAX, Duration::from_secs(1));
        assert_eq!(
            limiter.semaphore.available_permits(),
            Semaphore::MAX_PERMITS
        );
    }
}

/// Thin [`EventEmitter`] that forwards events to a tokio MPSC channel.
pub(crate) struct TokioChannelEmitter(pub(crate) mpsc::Sender<ApxmEvent>);

impl apxm_core::events::EventEmitter for TokioChannelEmitter {
    fn emit(&self, event: ApxmEvent) {
        let _ = self.0.try_send(event);
    }
}

pub(crate) const EXECUTE_COMPLETE: EventKind =
    EventKind::new("execute_complete", EventCategory::Lifecycle, true);

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub(crate) struct ExecuteCompletePayload {
    pub(crate) result: ExecuteResponse,
}
impl_event_payload!(ExecuteCompletePayload, EXECUTE_COMPLETE);
