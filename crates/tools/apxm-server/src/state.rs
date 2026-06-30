use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use apxm_core::events::payload::WarningPayload;
use apxm_core::events::{ApxmEvent, EventSource};
use apxm_core::types::consent::ConsentBroker;
use apxm_driver::ServerConfig;
use apxm_rollout::{IndexDb, RolloutPaths};
use apxm_runtime::Runtime;
use apxm_runtime::host_dispatch::HostDispatchGateway;
use apxm_server_api::AgentRuntimeApi;
use dashmap::DashMap;
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore, mpsc};

use crate::a2a::A2aTaskRecord;
use crate::agent::AgentRegistration;
use crate::checkpoints::CheckpointStore;
use crate::executions::ExecutionStore;
use crate::goal_runs::GoalRunRegistry;
use crate::rollout::RolloutRegistry;
use crate::runs::RunEventBus;
use crate::safety::SafetyState;
use crate::shutdown::ShutdownCoordinator;
use crate::skills::SkillLibrary;
use crate::tasks::TaskQueueManager;
use crate::webhook::WebhookDispatcher;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) runtime: Arc<dyn AgentRuntimeApi>,
    /// Host dispatch gateway for kind=host capability routing.
    /// Replaced by HostDispatchGatewayImpl in `os` when a real Link relay is active.
    pub(crate) host_dispatch: Arc<dyn HostDispatchGateway>,
    /// Per-call consent broker. NoOpConsentBroker until os wires the real broker.
    pub(crate) consent_broker: Arc<dyn ConsentBroker>,
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
    /// Per-execution event log + live broadcast.
    pub(crate) run_event_bus: RunEventBus,
    /// Optional outbound lifecycle webhook dispatcher.
    pub(crate) webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
    /// Filesystem resolver for rollout JSONL transcripts.
    pub(crate) rollout_paths: Arc<RolloutPaths>,
    /// Thread index sidecar — derived view, fast list+lookup.
    pub(crate) rollout_index: Arc<Mutex<IndexDb>>,
    /// Open rollout recorders keyed by execution_id.
    pub(crate) rollout_registry: RolloutRegistry,
    /// Process-local cap for expensive inference/runtime work.
    pub(crate) inference_limiter: InferenceLimiter,
    /// Layered server configuration for streaming handlers.
    pub(crate) server_config: ServerConfig,
    /// Resolved listen address (for auth policy and ops signals).
    pub(crate) bind_addr: SocketAddr,
    /// Auth-on when non-loopback or explicitly configured.
    pub(crate) effective_require_auth: bool,
    /// HTTP safety middleware state (rate limits).
    pub(crate) safety_state: SafetyState,
    /// Graceful shutdown in-flight tracking.
    pub(crate) shutdown: ShutdownCoordinator,
    /// In-flight streaming executions keyed by `execution_id`, each holding a
    /// `Notify` that `POST /v1/runs/{id}/cancel` trips to abort the run at the
    /// next await boundary. Entries are deleted when the execution settles.
    pub(crate) cancel_registry: Arc<DashMap<String, Arc<Notify>>>,
    /// Server-owned multi-pass goal runs keyed by `goal_id`.
    pub(crate) goal_runs: GoalRunRegistry,
    /// Runtime-minted capability grants keyed by opaque `grant_*` id.
    pub(crate) capability_grants: crate::capability_grants::CapabilityGrantStore,
    /// `session_id` → running conversation execution. Backs the turn-input
    /// endpoint (`POST /v1/conversations/{id}/message`) so the host stays a
    /// dumb pipe (constitution #2).
    pub(crate) session_registry: crate::conversations::SessionRegistry,
}

impl AppState {
    /// Access the underlying Runtime for the few wiring sites that need it
    /// (capability registration, execution dispatch). Handlers should prefer
    /// the trait methods on `self.runtime` instead of calling this.
    pub(crate) fn runtime(&self) -> Arc<Runtime> {
        self.runtime.runtime_arc()
    }
}

/// Operational defaults derived from layered server config.
pub(crate) struct HardeningDefaults {
    pub(crate) bind_addr: SocketAddr,
    pub(crate) effective_require_auth: bool,
    pub(crate) safety_state: SafetyState,
    pub(crate) shutdown: ShutdownCoordinator,
}

impl HardeningDefaults {
    pub(crate) fn for_config(server_config: &ServerConfig) -> Self {
        let bind_addr: SocketAddr = crate::DEFAULT_ADDR
            .parse()
            .expect("built-in default address");
        Self {
            bind_addr,
            effective_require_auth: crate::bind::effective_require_auth(
                &bind_addr,
                &server_config.auth,
            ),
            safety_state: SafetyState::from_config(&server_config.safety),
            shutdown: ShutdownCoordinator::new(),
        }
    }
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

/// Thin [`EventEmitter`] that forwards events to a tokio MPSC channel.
///
/// On backpressure (`try_send` full), emits an explicit lag warning instead of
/// silently dropping.
pub(crate) struct TokioChannelEmitter {
    tx: mpsc::Sender<ApxmEvent>,
    dropped: AtomicUsize,
}

const EXECUTE_STREAM_LAG_CODE: &str = "execute_stream_lag";
const EXECUTE_STREAM_LAG_TRACE: &str = "execute-stream-emitter";

impl TokioChannelEmitter {
    pub(crate) fn new(tx: mpsc::Sender<ApxmEvent>) -> Self {
        Self {
            tx,
            dropped: AtomicUsize::new(0),
        }
    }

    fn emit_lag_signal(&self, dropped: usize) {
        let lag = ApxmEvent::root(
            WarningPayload {
                code: EXECUTE_STREAM_LAG_CODE.to_string(),
                message: format!(
                    "execute event stream lagged; consumer is slower than producer ({dropped} event(s) not delivered)"
                ),
            },
            EventSource::Server,
            EXECUTE_STREAM_LAG_TRACE,
        );
        match self.tx.try_send(lag) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(lag)) => {
                // Channel saturated: block on a helper thread until the consumer
                // makes room so the lag signal is never silently dropped.
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.blocking_send(lag);
                });
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }
}

impl apxm_core::events::EventEmitter for TokioChannelEmitter {
    fn emit(&self, event: ApxmEvent) {
        match self.tx.try_send(event) {
            Ok(()) => {
                self.dropped.store(0, Ordering::Relaxed);
            }
            Err(mpsc::error::TrySendError::Full(dropped_event)) => {
                let count = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                if count == 1 {
                    self.emit_lag_signal(count);
                }
                drop(dropped_event);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }
}

#[cfg(test)]
mod emitter_tests {
    use super::*;
    use apxm_core::events::EventEmitter;
    use apxm_core::events::payload::TokenPayload;

    #[test]
    fn full_channel_emits_lag_warning_instead_of_silent_drop() {
        let (tx, mut rx) = mpsc::channel(1);
        let emitter = TokioChannelEmitter::new(tx);

        emitter.emit(ApxmEvent::root(
            TokenPayload {
                text: "first".to_string(),
            },
            EventSource::Runtime,
            "trace",
        ));
        emitter.emit(ApxmEvent::root(
            TokenPayload {
                text: "second".to_string(),
            },
            EventSource::Runtime,
            "trace",
        ));

        let first = rx.try_recv().expect("first event delivered");
        assert_eq!(first.payload.event_kind().name(), "token");

        let lag = rx
            .blocking_recv()
            .expect("lag warning delivered after consumer makes room");
        assert_eq!(lag.payload.event_kind().name(), "warning");
    }
}
