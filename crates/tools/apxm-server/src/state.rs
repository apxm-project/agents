use std::sync::Arc;
use std::time::{Duration, SystemTime};

use apxm_core::events::ApxmEvent;
use apxm_driver::ServerConfig;
use apxm_rollout::{IndexDb, RolloutPaths};
use apxm_runtime::Runtime;
use dashmap::DashMap;
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore, mpsc};

use crate::a2a::A2aTaskRecord;
use crate::agent::AgentRegistration;
use crate::checkpoints::CheckpointStore;
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
    /// Layered server configuration used by streaming handlers.
    pub(crate) server_config: ServerConfig,
    /// In-flight streaming executions keyed by `execution_id`, each holding a
    /// `Notify` that `POST /v1/runs/{id}/cancel` trips to abort the run at the
    /// next await boundary. Entries are removed when the execution settles.
    pub(crate) cancel_registry: Arc<DashMap<String, Arc<Notify>>>,
}

#[derive(Clone)]
pub(crate) struct InferenceLimiter {
    semaphore: Arc<Semaphore>,
    acquire_timeout: std::time::Duration,
}

impl InferenceLimiter {
    pub(crate) fn from_config(config: &apxm_driver::ServerInferenceConfig) -> Self {
        Self::with_limits(
            config.max_concurrent,
            Duration::from_millis(config.acquire_timeout_ms),
        )
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

impl InferencePermit {
    /// Consume into the raw owned permit (handed to an [`AdmissionHandle`] so a
    /// parked execution can release/reacquire its admission slot).
    pub(crate) fn into_inner(self) -> OwnedSemaphorePermit {
        self._permit
    }
}

impl InferenceLimiter {
    /// The underlying semaphore, for best-effort reacquire after a parked
    /// execution released its slot.
    pub(crate) fn semaphore(&self) -> Arc<Semaphore> {
        Arc::clone(&self.semaphore)
    }
}

/// Admission slot for one execution, manipulable by the runtime: a PARKED
/// execution releases its inference-admission permit (freeing capacity for active
/// work) and best-effort reacquires it on wake. Registered in the runtime
/// `admission_registry` under the execution's `admission_id`; dropping it (on
/// unregister at completion) finalizes the slot. Implements the runtime's
/// `ParkAdmission` trait so the runtime stays decoupled from the limiter.
pub(crate) struct AdmissionHandle {
    permit: std::sync::Mutex<Option<OwnedSemaphorePermit>>,
    semaphore: Arc<Semaphore>,
}

impl AdmissionHandle {
    pub(crate) fn new(permit: OwnedSemaphorePermit, semaphore: Arc<Semaphore>) -> Self {
        Self {
            permit: std::sync::Mutex::new(Some(permit)),
            semaphore,
        }
    }
}

impl apxm_runtime::scheduler::admission_registry::ParkAdmission for AdmissionHandle {
    fn on_park(&self) {
        // Release the admission slot by dropping the permit.
        *self.permit.lock().expect("admission permit poisoned") = None;
    }

    fn on_unpark(&self) {
        // Best-effort, non-blocking reacquire; if saturated, proceed un-admitted
        // (a bounded transient overshoot beats stalling a resumed agent).
        let mut p = self.permit.lock().expect("admission permit poisoned");
        if p.is_none() {
            *p = self.semaphore.clone().try_acquire_owned().ok();
        }
    }
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

    #[test]
    fn inference_limiter_uses_config_values() {
        let limiter = InferenceLimiter::from_config(&apxm_driver::ServerInferenceConfig {
            max_concurrent: 3,
            acquire_timeout_ms: 750,
        });

        assert_eq!(limiter.semaphore.available_permits(), 3);
        assert_eq!(limiter.acquire_timeout, Duration::from_millis(750));
    }
}

/// Thin [`EventEmitter`] that forwards events to a tokio MPSC channel.
pub(crate) struct TokioChannelEmitter(pub(crate) mpsc::Sender<ApxmEvent>);

impl apxm_core::events::EventEmitter for TokioChannelEmitter {
    fn emit(&self, event: ApxmEvent) {
        let _ = self.0.try_send(event);
    }
}
