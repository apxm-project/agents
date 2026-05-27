use std::sync::Arc;
use std::time::SystemTime;

use apxm_core::events::{ApxmEvent, EventCategory, EventKind};
use apxm_core::impl_event_payload;
use apxm_rollout::{IndexDb, RolloutPaths};
use apxm_runtime::Runtime;
use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::{Mutex, mpsc};

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
