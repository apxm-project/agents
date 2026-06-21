use std::collections::VecDeque;
use std::sync::Arc;

use apxm_runtime::capability::builtins::{FiredSchedule, OnFire};
use axum::Json;
use axum::extract::{Path, State};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::sync::{Mutex, Notify};
use tracing::info;

use crate::error::ApiError;
use crate::helpers::now_ms;
use crate::state::AppState;
use crate::types::responses::{OkAckId, TaskClaimResponse, TaskCreatedResponse, TaskListResponse};

const SCHEDULED_PROMPT_QUEUE: &str = apxm_core::constants::agent_tools::SCHEDULED_PROMPT_QUEUE;
const PAYLOAD_QUEUE: &str = apxm_core::constants::agent_tools::PAYLOAD_QUEUE;

/// Status of a queued task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TaskStatus {
    Pending,
    Claimed,
    Completed,
    Failed,
}

/// A task in the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct QueuedTask {
    pub(crate) id: String,
    pub(crate) queue: String,
    pub(crate) data: JsonValue,
    pub(crate) status: TaskStatus,
    #[serde(default)]
    pub(crate) claimed_by: Option<String>,
    #[serde(default, skip_serializing)]
    pub(crate) claim_token: Option<String>,
    #[serde(default)]
    pub(crate) lease_expires_ms: Option<u64>,
    #[serde(default)]
    pub(crate) result: Option<JsonValue>,
    pub(crate) created_at_ms: u64,
    #[serde(default)]
    pub(crate) completed_at_ms: Option<u64>,
}

/// In-memory task queue manager.
///
/// Uses a `DashMap<queue_name, Mutex<VecDeque<QueuedTask>>>` for O(1) queue
/// lookup and ordered insertion.
#[derive(Clone)]
pub(crate) struct TaskQueueManager {
    inner: Arc<DashMap<String, Arc<Mutex<VecDeque<QueuedTask>>>>>,
    waiters: Arc<DashMap<String, Arc<Notify>>>,
    pub(crate) all_tasks: Arc<DashMap<String, QueuedTask>>,
}

impl TaskQueueManager {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            waiters: Arc::new(DashMap::new()),
            all_tasks: Arc::new(DashMap::new()),
        }
    }

    pub(crate) async fn enqueue(&self, task: QueuedTask) {
        let queue_name = task.queue.clone();
        let queue: Arc<Mutex<VecDeque<QueuedTask>>> = {
            let entry = self
                .inner
                .entry(queue_name.clone())
                .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())));
            entry.value().clone()
        };
        self.all_tasks.insert(task.id.clone(), task.clone());
        queue.lock().await.push_back(task);
        self.notify_queue(&queue_name);
    }

    fn queue_notify(&self, queue_name: &str) -> Arc<Notify> {
        self.waiters
            .entry(queue_name.to_string())
            .or_insert_with(|| Arc::new(Notify::new()))
            .value()
            .clone()
    }

    fn notify_queue(&self, queue_name: &str) {
        self.queue_notify(queue_name).notify_waiters();
    }

    /// Atomically claim the next pending task from `queue_name`.
    pub(crate) async fn claim(
        &self,
        queue_name: &str,
        agent_id: &str,
        lease_ms: u64,
    ) -> Option<QueuedTask> {
        let queue: Arc<Mutex<VecDeque<QueuedTask>>> = self.inner.get(queue_name)?.value().clone();
        let mut guard = queue.lock().await;

        // Expire stale claims so their tasks become available again.
        let now = now_ms();
        for task in guard.iter_mut() {
            if task.status == TaskStatus::Claimed
                && let Some(expires) = task.lease_expires_ms
                && now > expires
            {
                task.status = TaskStatus::Pending;
                task.claim_token = None;
                task.claimed_by = None;
                task.lease_expires_ms = None;
                // Mirror into all_tasks index.
                if let Some(mut indexed) = self.all_tasks.get_mut(&task.id) {
                    indexed.status = TaskStatus::Pending;
                    indexed.claim_token = None;
                    indexed.claimed_by = None;
                    indexed.lease_expires_ms = None;
                }
            }
        }

        let pos = guard.iter().position(|t| t.status == TaskStatus::Pending)?;
        let task = guard.get_mut(pos)?;
        let claim_token = uuid::Uuid::new_v4().to_string();
        let deadline = now_ms() + lease_ms;
        task.status = TaskStatus::Claimed;
        task.claimed_by = Some(agent_id.to_string());
        task.claim_token = Some(claim_token.clone());
        task.lease_expires_ms = Some(deadline);
        let claimed = task.clone();
        self.all_tasks.insert(claimed.id.clone(), claimed.clone());
        Some(claimed)
    }

    pub(crate) async fn claim_or_wait(
        &self,
        queue_name: &str,
        agent_id: &str,
        lease_ms: u64,
        max_wait_ms: u64,
    ) -> Option<QueuedTask> {
        if max_wait_ms == 0 {
            return self.claim(queue_name, agent_id, lease_ms).await;
        }

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(max_wait_ms);
        loop {
            let notify = self.queue_notify(queue_name);
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if let Some(task) = self.claim(queue_name, agent_id, lease_ms).await {
                return Some(task);
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return None;
            }
            if tokio::time::timeout_at(deadline, notified.as_mut())
                .await
                .is_err()
            {
                return None;
            }
        }
    }

    pub(crate) async fn complete(
        &self,
        task_id: &str,
        claim_token: &str,
        result: JsonValue,
        success: bool,
    ) -> Result<(), String> {
        let mut task = self
            .all_tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("Task '{}' not found", task_id))?;
        if task.claim_token.as_deref() != Some(claim_token) {
            return Err("Invalid claim token".to_string());
        }
        if let Some(expires) = task.lease_expires_ms
            && now_ms() > expires
        {
            return Err("lease_expired: Task lease has expired. Task may have been reclaimed by another worker.".to_string());
        }
        let status = if success {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed
        };
        task.status = status.clone();
        task.result = Some(result.clone());
        task.completed_at_ms = Some(now_ms());
        let queue_name = task.queue.clone();
        let tid = task.id.clone();
        drop(task);
        if let Some(queue_ref) = self.inner.get(&queue_name) {
            let queue: Arc<Mutex<VecDeque<QueuedTask>>> = queue_ref.value().clone();
            drop(queue_ref);
            let mut guard = queue.lock().await;
            if let Some(t) = guard.iter_mut().find(|t| t.id == tid) {
                t.status = status;
                t.result = Some(result);
                t.completed_at_ms = Some(now_ms());
            }
        }
        Ok(())
    }

    pub(crate) fn list_queue(&self, queue_name: &str) -> Vec<QueuedTask> {
        match self.inner.get(queue_name) {
            Some(q) => {
                let arc = q.value().clone();
                drop(q);
                match arc.try_lock() {
                    Ok(guard) => guard.iter().cloned().collect(),
                    Err(_) => vec![],
                }
            }
            None => vec![],
        }
    }
}

pub(crate) fn scheduled_prompt_on_fire(task_manager: TaskQueueManager) -> OnFire {
    Arc::new(move |fired| {
        log_schedule_fire(fired.clone());
        let Some(prompt) = fired
            .prompt
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        else {
            return;
        };
        let payload_json = parse_schedule_payload(&fired.payload);
        let queue = schedule_queue(&payload_json);
        let task = QueuedTask {
            id: uuid::Uuid::new_v4().to_string(),
            queue,
            data: serde_json::json!({
                "kind": "scheduled_prompt",
                "schedule_id": fired.id,
                "schedule_kind": fired.kind,
                "recurring": fired.recurring,
                "prompt": prompt,
                "payload": payload_json,
            }),
            status: TaskStatus::Pending,
            claimed_by: None,
            claim_token: None,
            lease_expires_ms: None,
            result: None,
            created_at_ms: now_ms(),
            completed_at_ms: None,
        };
        let manager = task_manager.clone();
        tokio::spawn(async move {
            let id = task.id.clone();
            let queue = task.queue.clone();
            manager.enqueue(task).await;
            info!(%id, %queue, "scheduled prompt enqueued");
        });
    })
}

fn log_schedule_fire(fired: FiredSchedule) {
    info!(
        target: "apxm::schedule",
        schedule_id = %fired.id,
        kind = %fired.kind,
        recurring = fired.recurring,
        prompt = fired.prompt.as_deref().unwrap_or(""),
        payload = %fired.payload,
        "schedule fired"
    );
}

fn parse_schedule_payload(payload: &str) -> JsonValue {
    serde_json::from_str(payload).unwrap_or_else(|_| serde_json::json!({}))
}

fn schedule_queue(payload: &JsonValue) -> String {
    payload
        .get(PAYLOAD_QUEUE)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|queue| !queue.is_empty())
        .unwrap_or(SCHEDULED_PROMPT_QUEUE)
        .to_string()
}

// ─── Task Queue Handlers (CLAIM op backend) ─────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct CreateTaskRequest {
    queue: String,
    data: JsonValue,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaimTaskRequest {
    agent_id: String,
    #[serde(default = "default_lease_ms")]
    lease_ms: u64,
    /// Maximum milliseconds to long-poll for a task. 0 = return immediately.
    #[serde(default)]
    max_wait_ms: u64,
}

pub(crate) fn default_lease_ms() -> u64 {
    60_000
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompleteTaskRequest {
    claim_token: String,
    #[serde(default)]
    result: JsonValue,
    #[serde(default = "default_complete_success")]
    success: bool,
}

fn default_complete_success() -> bool {
    true
}

pub(crate) async fn create_task(
    State(state): State<AppState>,
    Json(req): Json<CreateTaskRequest>,
) -> Json<TaskCreatedResponse> {
    let id = req.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let task = QueuedTask {
        id: id.clone(),
        queue: req.queue.clone(),
        data: req.data,
        status: TaskStatus::Pending,
        claimed_by: None,
        claim_token: None,
        lease_expires_ms: None,
        result: None,
        created_at_ms: now_ms(),
        completed_at_ms: None,
    };
    state.task_manager.enqueue(task).await;
    info!(id = %id, queue = %req.queue, "Task enqueued");
    Json(TaskCreatedResponse {
        ok: true,
        id,
        queue: req.queue,
    })
}

pub(crate) async fn list_tasks(
    State(state): State<AppState>,
    Path(queue): Path<String>,
) -> Json<TaskListResponse> {
    let tasks = state.task_manager.list_queue(&queue);
    Json(TaskListResponse {
        queue,
        count: tasks.len(),
        tasks,
    })
}

pub(crate) async fn claim_task(
    State(state): State<AppState>,
    Path(queue): Path<String>,
    Json(req): Json<ClaimTaskRequest>,
) -> Result<Json<TaskClaimResponse>, ApiError> {
    let task = state
        .task_manager
        .claim_or_wait(&queue, &req.agent_id, req.lease_ms, req.max_wait_ms)
        .await;
    match task {
        Some(t) => {
            info!(id = %t.id, queue = %queue, agent_id = %req.agent_id, "Task claimed");
            Ok(Json(TaskClaimResponse {
                task_id: t.id,
                queue: t.queue,
                data: t.data,
                claim_token: t.claim_token,
                expires_at_ms: t.lease_expires_ms,
            }))
        }
        None => Err(ApiError::not_found(format!(
            "No pending tasks in queue '{queue}'"
        ))),
    }
}

pub(crate) async fn complete_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CompleteTaskRequest>,
) -> Result<Json<OkAckId>, ApiError> {
    state
        .task_manager
        .complete(&id, &req.claim_token, req.result, req.success)
        .await
        .map_err(|e| {
            if e.starts_with("lease_expired:") {
                ApiError::conflict(e)
            } else {
                ApiError::bad_request(e)
            }
        })?;
    info!(id = %id, success = %req.success, "Task completed");
    Ok(Json(OkAckId::new(id)))
}
