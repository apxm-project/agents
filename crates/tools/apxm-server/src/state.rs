use std::sync::Arc;
use std::time::SystemTime;

use apxm_core::events::{ApxmEvent, EventCategory, EventKind};
use apxm_core::impl_event_payload;
use apxm_runtime::Runtime;
use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::a2a::A2aTaskRecord;
use crate::agent::AgentRegistration;
use crate::checkpoints::CheckpointStore;
use crate::execute::ExecuteResponse;
use crate::executions::ExecutionStore;
use crate::skills::SkillLibrary;
use crate::tasks::TaskQueueManager;

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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ExecuteCompletePayload {
    pub(crate) result: ExecuteResponse,
}
impl_event_payload!(ExecuteCompletePayload, EXECUTE_COMPLETE);
