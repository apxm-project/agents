//! Observer endpoints (`/v1/runs/...`).
//!
//! Read-only consumer-facing surface that turns the apxm event bus into:
//!   - a listing of recent runs
//!   - per-run summary + graph topology
//!   - per-node detail drill-down
//!   - SSE tail with replay-from-seq support
//!   - bulk event pull
//!
//! The in-memory `RunEventBus` retains every event a run emits so a
//! late-attaching observer can replay from `seq=0` then tail live.
//! Events also fan out to a tokio broadcast channel for streaming.
//!
//! This observer surface is additive: producers keep writing normal execution
//! events, and `/v1/runs` gives clients a read-only view over those records.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use apxm_core::constants;
use apxm_core::events::payload::{
    AgentSpawnedPayload, CommunicateDispatchedPayload, GraphEdgePayload, OperationEndPayload,
    OperationStartPayload, ToolEndPayload, ToolStartPayload,
};
use apxm_core::events::{ApxmEvent, EventEmitter, kind as event_kind};
use apxm_core::paths::session_node_dir_name;
use apxm_core::types::operations::AISOperationType;
use apxm_driver::RunEventsConfig;
use apxm_rollout::load_rollout;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::http::header;
use axum::response::Response;
use axum::response::sse::{Event, KeepAlive, Sse};
use dashmap::DashMap;
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::error::ApiError;
use crate::executions::{ExecutionRecord, ExecutionStatus, NodeMetricsRecord, NodeOutputRecord};
use crate::state::AppState;

const MIN_RETAINED_EVENTS: usize = 128;
const LAST_EVENT_ID_HEADER: &str = "Last-Event-ID";
const LAST_EVENT_ID_HEADER_LOWER: &str = "last-event-id";

// ────────────────────────────────────────────────────────────────────
// RunEventBus — owns retained events + live broadcast per execution.
// ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct RunEventBus {
    inner: Arc<DashMap<String, RunEventState>>,
    retained_events: usize,
    stream_buffer: usize,
}

struct RunEventState {
    events: VecDeque<ApxmEvent>,
    tx: broadcast::Sender<ApxmEvent>,
    next_seq: u64,
}

impl RunEventState {
    fn new(stream_buffer: usize) -> Self {
        let (tx, _rx) = broadcast::channel(stream_buffer.max(1));
        Self {
            events: VecDeque::new(),
            tx,
            next_seq: 0,
        }
    }
}

impl RunEventBus {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_config(&RunEventsConfig::default())
    }

    pub(crate) fn with_config(config: &RunEventsConfig) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            retained_events: config.retained_events.max(MIN_RETAINED_EVENTS),
            stream_buffer: config.stream_buffer.max(1),
        }
    }

    /// Record an event into the per-execution ring and fan out to live
    /// subscribers. The returned event has the bus-owned per-run sequence
    /// number applied for durable/live downstream sinks.
    pub(crate) fn record(&self, execution_id: &str, mut event: ApxmEvent) -> ApxmEvent {
        let mut entry = self
            .inner
            .entry(execution_id.to_string())
            .or_insert_with(|| RunEventState::new(self.stream_buffer));
        event.meta.seq = entry.next_seq;
        entry.next_seq = entry.next_seq.checked_add(1).unwrap_or_else(|| {
            tracing::warn!(execution_id, "run event sequence saturated");
            u64::MAX
        });
        entry.events.push_back(event.clone());
        while entry.events.len() > self.retained_events {
            entry.events.pop_front();
        }
        // No-subscriber send errors are non-fatal — only live observers care.
        let _ = entry.tx.send(event);
        entry.events.back().cloned().expect("recorded event")
    }

    /// All events recorded for `execution_id` so far, in arrival order.
    pub(crate) fn snapshot(&self, execution_id: &str) -> Vec<ApxmEvent> {
        self.inner
            .get(execution_id)
            .map(|entry| entry.events.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Subscribe to live events for `execution_id`. Subscribers get
    /// only events emitted after the subscription is taken; replay is
    /// done via the snapshot above.
    pub(crate) fn subscribe(&self, execution_id: &str) -> broadcast::Receiver<ApxmEvent> {
        let entry = self
            .inner
            .entry(execution_id.to_string())
            .or_insert_with(|| RunEventState::new(self.stream_buffer));
        entry.tx.subscribe()
    }

    /// Execution ids with recorded events (integration-test discovery).
    pub fn list_execution_ids(&self) -> Vec<String> {
        self.inner.iter().map(|entry| entry.key().clone()).collect()
    }
}

/// EventEmitter that funnels every event into the run bus, keyed by
/// the event's trace_id (which is the execution_id for runtime events).
#[cfg(test)]
pub(crate) struct RunBusEmitter {
    bus: RunEventBus,
    execution_id: String,
}

#[cfg(test)]
impl RunBusEmitter {
    pub(crate) fn new(bus: RunEventBus, execution_id: impl Into<String>) -> Self {
        Self {
            bus,
            execution_id: execution_id.into(),
        }
    }
}

#[cfg(test)]
impl EventEmitter for RunBusEmitter {
    fn emit(&self, event: ApxmEvent) {
        self.bus.record(&self.execution_id, event);
    }
}

/// Records to `RunEventBus`, then fans the normalized event to downstream
/// sinks such as rollout, webhook, and streaming response channels.
pub(crate) struct RunBusFanOutEmitter {
    bus: RunEventBus,
    execution_id: String,
    downstream: Vec<Arc<dyn EventEmitter>>,
}

impl RunBusFanOutEmitter {
    pub(crate) fn new(
        bus: RunEventBus,
        execution_id: impl Into<String>,
        downstream: Vec<Arc<dyn EventEmitter>>,
    ) -> Self {
        Self {
            bus,
            execution_id: execution_id.into(),
            downstream,
        }
    }
}

impl EventEmitter for RunBusFanOutEmitter {
    fn emit(&self, event: ApxmEvent) {
        let event = self.bus.record(&self.execution_id, event);
        for emitter in &self.downstream {
            emitter.emit(event.clone());
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// Response shapes.
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunListResponse {
    pub(crate) object: &'static str,
    pub(crate) data: Vec<RunSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunSummary {
    pub(crate) execution_id: String,
    pub(crate) skill_id: String,
    pub(crate) skill_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) workflow_id: Option<String>,
    pub(crate) status: ExecutionStatus,
    pub(crate) started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at_ms: Option<u64>,
    pub(crate) session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) run_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) root_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) trace_id: Option<String>,
    pub(crate) totals: RunTotals,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RunTotals {
    pub(crate) events: usize,
    pub(crate) nodes: usize,
    pub(crate) edges: usize,
    pub(crate) tool_calls: usize,
    pub(crate) input_tokens: usize,
    pub(crate) output_tokens: usize,
    pub(crate) total_tokens: usize,
    pub(crate) total_requests: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunGraph {
    /// Bumped whenever the on-wire graph shape changes.
    pub(crate) graph_schema_version: u32,
    pub(crate) execution_id: String,
    pub(crate) nodes: Vec<RunGraphNode>,
    pub(crate) edges: Vec<RunGraphEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunGraphNode {
    pub(crate) id: u64,
    pub(crate) kind: RunNodeKind,
    pub(crate) op_type: AISOperationType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) agent_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_name: Option<String>,
    pub(crate) status: NodeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) started_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) layer: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) context: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunNodeKind {
    Agent,
    Tool,
    Llm,
    Op,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NodeStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunGraphEdge {
    pub(crate) from: u64,
    pub(crate) to: u64,
    pub(crate) kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunNodeDetail {
    pub(crate) execution_id: String,
    pub(crate) node_id: u64,
    pub(crate) op_type: Option<AISOperationType>,
    pub(crate) status: NodeStatus,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) events: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) outputs: Vec<NodeOutputRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) metrics: Vec<NodeMetricsRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) artifacts: Option<RunNodeArtifactRefs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<RunNodeAgentDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool: Option<RunNodeToolDetail>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunNodeArtifactRefs {
    pub(crate) run_root: String,
    pub(crate) node_dir: String,
    pub(crate) node_json: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metrics_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunArtifactList {
    pub(crate) execution_id: String,
    pub(crate) run_root: String,
    pub(crate) artifacts: Vec<RunArtifactEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunArtifactEntry {
    pub(crate) path: String,
    pub(crate) size_bytes: u64,
    pub(crate) media_type: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunNodeAgentDetail {
    pub(crate) agent_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) profile: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RunNodeToolDetail {
    pub(crate) tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) args: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EventsQuery {
    #[serde(default)]
    pub(crate) since: Option<u64>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EventsBulkResponse {
    pub(crate) execution_id: String,
    pub(crate) events: Vec<serde_json::Value>,
    pub(crate) next_seq: u64,
    pub(crate) done: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum EventReplayCursor {
    FromSeq(u64),
    AfterSeq(u64),
}

impl EventReplayCursor {
    pub(crate) fn from_seq(seq: u64) -> Self {
        Self::FromSeq(seq)
    }

    pub(crate) fn after_seq(seq: u64) -> Self {
        Self::AfterSeq(seq)
    }

    pub(crate) fn from_request(headers: &HeaderMap, query: &EventsQuery) -> Self {
        headers
            .get(LAST_EVENT_ID_HEADER)
            .or_else(|| headers.get(LAST_EVENT_ID_HEADER_LOWER))
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Self::AfterSeq)
            .unwrap_or_else(|| Self::FromSeq(query.since.unwrap_or(0)))
    }

    pub(crate) fn accepts(self, seq: u64) -> bool {
        match self {
            Self::FromSeq(since) => seq >= since,
            Self::AfterSeq(last_event_id) => seq > last_event_id,
        }
    }

    pub(crate) fn required_first_seq(self) -> Option<u64> {
        match self {
            Self::FromSeq(since) => Some(since),
            Self::AfterSeq(last_event_id) => last_event_id.checked_add(1),
        }
    }
}

pub(crate) enum RunSseItem {
    Event(ApxmEvent),
    Lagged(u64),
}

/// Which SSE surface is emitting run events — controls the lag/overflow frame copy.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RunEventStreamKind {
    RunObserver,
    ExecuteStream,
}

impl RunEventStreamKind {
    fn lag_payload(self, missed: u64) -> serde_json::Value {
        let message = match self {
            Self::RunObserver => "run event stream lagged; reconnect with Last-Event-ID to replay",
            Self::ExecuteStream => {
                "execute event stream lagged; reconnect with Last-Event-ID to replay retained events"
            }
        };
        serde_json::json!({ "message": message, "missed": missed, "code": "stream_lag" })
    }
}

pub(crate) struct RunEventStreamOptions {
    pub kind: RunEventStreamKind,
    pub keep_alive_secs: u64,
    /// Close the live SSE after a terminal lifecycle event (execute stream path).
    pub close_on_terminal: bool,
}

/// Record a lifecycle/server event on the observer bus and mirror to rollout when active.
pub(crate) fn record_run_lifecycle_event(
    bus: &RunEventBus,
    rollout_registry: &crate::rollout::RolloutRegistry,
    execution_id: &str,
    event: ApxmEvent,
    rollout_recording: bool,
) -> ApxmEvent {
    let event = bus.record(execution_id, event);
    if rollout_recording {
        rollout_registry.try_record(execution_id, event.clone());
    }
    event
}

fn prepare_replay_events(
    mut replay: Vec<ApxmEvent>,
    cursor: EventReplayCursor,
) -> (Vec<ApxmEvent>, Option<u64>) {
    let mut seen = std::collections::HashSet::new();
    replay.retain(|event| cursor.accepts(event.meta.seq) && seen.insert(event.meta.seq));
    replay.sort_by_key(|event| event.meta.seq);
    let high_water = replay.iter().map(|event| event.meta.seq).max();
    (replay, high_water)
}

fn is_terminal_run_event(event: &ApxmEvent) -> bool {
    matches!(
        event.kind().name(),
        "execute_complete" | "error" | "turn_aborted" | "skill_execute_complete"
    )
}

fn combined_run_sse_items(
    replay: Vec<ApxmEvent>,
    cursor: EventReplayCursor,
    rx: broadcast::Receiver<ApxmEvent>,
    close_on_terminal: bool,
) -> impl Stream<Item = RunSseItem> {
    let (replay, replay_high_water) = prepare_replay_events(replay, cursor);
    use futures::StreamExt as _;
    let live = BroadcastStream::new(rx).filter_map(move |item| async move {
        match item {
            Ok(event)
                if cursor.accepts(event.meta.seq)
                    && replay_high_water.is_none_or(|seq| event.meta.seq > seq) =>
            {
                Some(RunSseItem::Event(event))
            }
            Ok(_) => None,
            Err(BroadcastStreamRecvError::Lagged(missed)) => Some(RunSseItem::Lagged(missed)),
        }
    });
    futures::stream::iter(replay.into_iter().map(RunSseItem::Event))
        .chain(live)
        .scan(false, move |closed, item| {
            let emit = if *closed {
                None
            } else {
                if matches!(item, RunSseItem::Lagged(_)) {
                    *closed = true;
                } else if close_on_terminal
                    && matches!(&item, RunSseItem::Event(event) if is_terminal_run_event(event))
                {
                    *closed = true;
                }
                Some(item)
            };
            async move { emit }
        })
}

fn run_sse_item_to_axum_event(item: RunSseItem, kind: RunEventStreamKind) -> Event {
    match item {
        RunSseItem::Event(event) => {
            let id = event.meta.seq.to_string();
            let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
            Event::default()
                .event(event.kind().name())
                .id(id)
                .data(data)
        }
        RunSseItem::Lagged(missed) => Event::default()
            .event(event_kind::ERROR.sse_event_type())
            .data(kind.lag_payload(missed).to_string()),
    }
}

/// Build a resumable SSE stream from an observer-bus subscription + replay snapshot.
pub(crate) fn run_sse_stream(
    rx: broadcast::Receiver<ApxmEvent>,
    replay: Vec<ApxmEvent>,
    cursor: EventReplayCursor,
    options: RunEventStreamOptions,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    use futures::StreamExt as _;
    let kind = options.kind;
    let stream = combined_run_sse_items(replay, cursor, rx, options.close_on_terminal)
        .map(move |item| Ok(run_sse_item_to_axum_event(item, kind)));
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(options.keep_alive_secs.max(1))))
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RunsListQuery {
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) trace_id: Option<String>,
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

// ────────────────────────────────────────────────────────────────────
// Handlers.
// ────────────────────────────────────────────────────────────────────

pub(crate) async fn list_runs(
    State(state): State<AppState>,
    Query(query): Query<RunsListQuery>,
) -> Json<RunListResponse> {
    let limit = run_list_limit(&state.server_config.run_events, query.limit);
    let status_filter = query.status.as_deref().and_then(parse_status_filter);
    let mut runs: Vec<RunSummary> = state
        .execution_store
        .list()
        .into_iter()
        .filter(|record| {
            query
                .session_id
                .as_deref()
                .is_none_or(|sid| record.session_id == sid)
        })
        .filter(|record| {
            query
                .trace_id
                .as_deref()
                .is_none_or(|tid| record.trace_id.as_deref() == Some(tid))
        })
        .filter(|record| match &status_filter {
            Some(filter) => record.status == *filter,
            None => true,
        })
        .map(|record| record_to_summary(&state, record))
        .take(limit)
        .collect();
    // fall back to the SQLite index when the in-memory
    // execution store has nothing for this filter. Useful after a
    // restart: the rollout JSONLs survive and the index points at them.
    if runs.is_empty() && query.trace_id.is_none() {
        let index = state.rollout_index.lock().await;
        let from_index = index.list_recent(limit).unwrap_or_default();
        for entry in from_index {
            if let Some(sid) = query.session_id.as_deref()
                && entry.session_id != sid
            {
                continue;
            }
            runs.push(index_entry_to_summary(entry));
        }
    }
    runs.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
    Json(RunListResponse {
        object: "list",
        data: runs,
    })
}

fn index_entry_to_summary(entry: apxm_rollout::ThreadIndexEntry) -> RunSummary {
    // Index rows don't carry a started_at_ms — parse RFC3339 once for
    // sortability; failures fall through to 0 (sorts last).
    let started_at_ms = chrono::DateTime::parse_from_rfc3339(&entry.started_at)
        .map(|ts| ts.timestamp_millis() as u64)
        .unwrap_or(0);
    let status = match entry.status.as_str() {
        "succeeded" | "done" => ExecutionStatus::Succeeded,
        "failed" => ExecutionStatus::Failed,
        // Default + explicit "running" both map to Running.
        _ => ExecutionStatus::Running,
    };
    RunSummary {
        execution_id: entry.thread_id,
        skill_id: String::new(),
        skill_version: String::new(),
        workflow_id: None,
        status,
        started_at_ms,
        completed_at_ms: None,
        session_id: entry.session_id,
        session_dir: None,
        run_root: None,
        root_agent: entry.agent_code,
        correlation_id: None,
        trace_id: None,
        totals: RunTotals::default(),
    }
}

pub(crate) async fn get_run(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Result<Json<RunSummary>, ApiError> {
    let record = state
        .execution_store
        .get(&execution_id)
        .ok_or_else(|| ApiError::not_found(format!("run not found: {execution_id}")))?;
    Ok(Json(record_to_summary(&state, record)))
}

#[derive(Debug, Serialize)]
pub(crate) struct CancelResponse {
    pub(crate) execution_id: String,
    pub(crate) cancelled: bool,
}

/// `POST /v1/runs/{execution_id}/cancel` — trip the in-flight run's abort
/// signal. Returns 404 if the run is unknown or already settled (its registry
/// entry is deleted on completion), so a cancel after the answer lands is a
/// no-op rather than an error the caller must special-case.
pub(crate) async fn cancel_run(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Result<Json<CancelResponse>, ApiError> {
    match state.cancel_registry.get(&execution_id) {
        Some(notify) => {
            notify.notify_one();
            Ok(Json(CancelResponse {
                execution_id,
                cancelled: true,
            }))
        }
        None => Err(ApiError::not_found(format!(
            "no in-flight run to cancel: {execution_id}"
        ))),
    }
}

pub(crate) async fn get_run_graph(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Result<Json<RunGraph>, ApiError> {
    let events = events_for_run(&state, &execution_id).await;
    // Surface 404 cleanly when neither the execution store, the in-memory
    // bus, nor the on-disk rollout knows about this run.
    if state.execution_store.get(&execution_id).is_none() && events.is_empty() {
        return Err(ApiError::not_found(format!(
            "run not found: {execution_id}"
        )));
    }
    Ok(Json(build_graph(&execution_id, &events)))
}

pub(crate) async fn get_run_node(
    State(state): State<AppState>,
    Path((execution_id, node_id)): Path<(String, u64)>,
) -> Result<Json<RunNodeDetail>, ApiError> {
    let events = events_for_run(&state, &execution_id).await;
    let record = state.execution_store.get(&execution_id);
    if events.is_empty() && record.is_none() {
        return Err(ApiError::not_found(format!(
            "run not found: {execution_id}"
        )));
    }
    build_node_detail(&execution_id, node_id, &events, record.as_ref())
        .map(Json)
        .ok_or_else(|| {
            ApiError::not_found(format!("node {node_id} not found for run {execution_id}"))
        })
}

/// `GET /v1/runs/{execution_id}/artifacts` — list the durable files under
/// the run root. This lets Studio/operator tooling inspect the recorded
/// `run.json` and per-node JSON files without direct volume access.
pub(crate) async fn list_run_artifacts(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Result<Json<RunArtifactList>, ApiError> {
    let record = state
        .execution_store
        .get(&execution_id)
        .ok_or_else(|| ApiError::not_found(format!("run not found: {execution_id}")))?;
    let run_root = record
        .run_root
        .clone()
        .ok_or_else(|| ApiError::not_found(format!("run has no artifact root: {execution_id}")))?;
    let root = FsPath::new(&run_root);
    if !root.is_dir() {
        return Err(ApiError::not_found(format!(
            "run artifacts not found for {execution_id}"
        )));
    }

    let mut artifacts = Vec::new();
    collect_artifact_entries(root, root, &mut artifacts).map_err(|error| {
        ApiError::internal_message(format!("failed to list run artifacts: {error}"))
    })?;
    artifacts.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Json(RunArtifactList {
        execution_id,
        run_root,
        artifacts,
    }))
}

/// `GET /v1/runs/{execution_id}/artifacts/{path}` — read one durable artifact
/// file by the relative path returned by `list_run_artifacts`.
pub(crate) async fn get_run_artifact(
    State(state): State<AppState>,
    Path((execution_id, artifact_path)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let record = state
        .execution_store
        .get(&execution_id)
        .ok_or_else(|| ApiError::not_found(format!("run not found: {execution_id}")))?;
    let run_root = record
        .run_root
        .as_deref()
        .ok_or_else(|| ApiError::not_found(format!("run has no artifact root: {execution_id}")))?;
    let path = resolve_artifact_path(run_root, &artifact_path)?;
    let bytes = std::fs::read(&path).map_err(|error| {
        ApiError::internal_message(format!("failed to read run artifact: {error}"))
    })?;
    let media_type = artifact_media_type(&path);
    Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(header::CONTENT_TYPE, media_type)
        .body(Body::from(bytes))
        .map_err(|error| ApiError::internal_message(format!("failed to build response: {error}")))
}

pub(crate) async fn get_run_events_bulk(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventsBulkResponse>, ApiError> {
    let since = query.since.unwrap_or(0);
    let events = events_for_run_since(&state, &execution_id, since).await;
    if events.is_empty() && state.execution_store.get(&execution_id).is_none() {
        return Err(ApiError::not_found(format!(
            "run not found: {execution_id}"
        )));
    }
    let limit = run_events_limit(&state.server_config.run_events, query.limit);

    let filtered: Vec<&ApxmEvent> = events.iter().filter(|e| e.meta.seq >= since).collect();
    let page: Vec<&ApxmEvent> = filtered.iter().take(limit).copied().collect();
    let next_seq = page.last().map_or(since, |e| e.meta.seq + 1);
    let done = filtered.len() <= limit;

    let serialized: Vec<serde_json::Value> = page
        .iter()
        .map(|event| serde_json::to_value(event).unwrap_or(serde_json::Value::Null))
        .collect();
    Ok(Json(EventsBulkResponse {
        execution_id,
        events: serialized,
        next_seq,
        done,
    }))
}

pub(crate) async fn stream_run_events(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let cursor = EventReplayCursor::from_request(&headers, &query);

    // Subscribe before snapshotting so an event emitted during the replay setup
    // is either in the snapshot or waiting in the broadcast receiver.
    let rx = state.run_event_bus.subscribe(&execution_id);
    let snapshot = state.run_event_bus.snapshot(&execution_id);

    // Fall back to the rollout when the in-memory bounded ring is empty or the
    // reconnect cursor predates the first retained in-memory event.
    let bus_empty = snapshot.is_empty();
    let ring_missed_cursor = cursor
        .required_first_seq()
        .zip(snapshot.first().map(|event| event.meta.seq))
        .is_some_and(|(required_first_seq, first_seq)| required_first_seq < first_seq);
    let disk_events = if bus_empty || ring_missed_cursor {
        events_from_disk(&state, &execution_id).await
    } else {
        Vec::new()
    };
    if state.execution_store.get(&execution_id).is_none() && bus_empty && disk_events.is_empty() {
        return Err(ApiError::not_found(format!(
            "run not found: {execution_id}"
        )));
    }

    let mut seen = std::collections::HashSet::new();
    let replay: Vec<ApxmEvent> = disk_events
        .into_iter()
        .chain(snapshot)
        .filter(|event| cursor.accepts(event.meta.seq))
        .filter(|event| seen.insert(event.meta.seq))
        .collect();

    use futures::StreamExt as _;
    let combined = combined_run_sse_items(replay, cursor, rx, false);

    let stream = combined.map(|item| {
        Ok(run_sse_item_to_axum_event(
            item,
            RunEventStreamKind::RunObserver,
        ))
    });

    Ok(
        Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(
            state.server_config.run_events.keep_alive_secs.max(1),
        ))),
    )
}

// ────────────────────────────────────────────────────────────────────
// Helpers.
// ────────────────────────────────────────────────────────────────────

fn parse_status_filter(raw: &str) -> Option<ExecutionStatus> {
    // Match the wire labels apxm-server uses elsewhere.
    match raw {
        "running" => Some(ExecutionStatus::Running),
        "succeeded" | "done" => Some(ExecutionStatus::Succeeded),
        "failed" => Some(ExecutionStatus::Failed),
        _ => None,
    }
}

fn run_list_limit(config: &RunEventsConfig, raw: Option<usize>) -> usize {
    clamp_limit(raw, config.default_list_limit, config.max_list_limit)
}

pub(crate) fn run_events_limit(config: &RunEventsConfig, raw: Option<usize>) -> usize {
    clamp_limit(raw, config.default_events_limit, config.max_events_limit)
}

fn clamp_limit(raw: Option<usize>, default: usize, max: usize) -> usize {
    let max = max.max(1);
    let default = default.clamp(1, max);
    raw.unwrap_or(default).clamp(1, max)
}

fn collect_artifact_entries(
    root: &FsPath,
    dir: &FsPath,
    out: &mut Vec<RunArtifactEntry>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_artifact_entries(root, &path, out)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let relative = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        if relative.ends_with(".tmp") || relative.is_empty() {
            continue;
        }
        let metadata = entry.metadata()?;
        out.push(RunArtifactEntry {
            media_type: artifact_media_type(&path),
            path: relative,
            size_bytes: metadata.len(),
        });
    }
    Ok(())
}

fn resolve_artifact_path(run_root: &str, artifact_path: &str) -> Result<PathBuf, ApiError> {
    if artifact_path.is_empty() {
        return Err(ApiError::bad_request("artifact path is required"));
    }
    let relative = FsPath::new(artifact_path);
    if relative.is_absolute() {
        return Err(ApiError::bad_request("artifact path must be relative"));
    }
    let mut clean = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => clean.push(value),
            _ => {
                return Err(ApiError::bad_request(
                    "artifact path contains invalid segments",
                ));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(ApiError::bad_request("artifact path is required"));
    }
    let root = FsPath::new(run_root);
    let root = root
        .canonicalize()
        .map_err(|_| ApiError::not_found("run artifact root not found"))?;
    let candidate = root.join(clean);
    let candidate = candidate
        .canonicalize()
        .map_err(|_| ApiError::not_found("run artifact not found"))?;
    if !candidate.starts_with(&root) {
        return Err(ApiError::bad_request("artifact path escapes run root"));
    }
    if !candidate.is_file() {
        return Err(ApiError::not_found("run artifact not found"));
    }
    Ok(candidate)
}

fn artifact_media_type(path: &FsPath) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => "application/json",
        Some("jsonl") | Some("ndjson") => "application/x-ndjson",
        Some("txt") | Some("log") | Some("md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn record_to_summary(state: &AppState, record: ExecutionRecord) -> RunSummary {
    let events = state.run_event_bus.snapshot(&record.execution_id);
    let mut totals = run_totals(&events);
    if let Some(result) = record.result.as_ref() {
        totals.input_tokens = result.llm_usage.input_tokens;
        totals.output_tokens = result.llm_usage.output_tokens;
        totals.total_tokens = result
            .llm_usage
            .input_tokens
            .saturating_add(result.llm_usage.output_tokens);
        totals.total_requests = result.llm_usage.total_requests;
        totals.cost_usd = None;
    }
    let root_agent = events.iter().find_map(|event| {
        event
            .payload
            .downcast_ref::<AgentSpawnedPayload>()
            .map(|payload| payload.agent_code.clone())
    });
    RunSummary {
        execution_id: record.execution_id,
        skill_id: record.skill_id,
        skill_version: record.skill_version,
        workflow_id: record.workflow_id,
        status: record.status,
        started_at_ms: record.started_at_ms,
        completed_at_ms: record.completed_at_ms,
        session_id: record.session_id,
        session_dir: Some(record.session_dir),
        run_root: record.run_root,
        root_agent,
        correlation_id: record.correlation_id.clone(),
        trace_id: record.trace_id.clone(),
        totals,
    }
}

fn run_totals(events: &[ApxmEvent]) -> RunTotals {
    let mut nodes = std::collections::HashSet::new();
    let mut edges = 0usize;
    let mut tool_calls = 0usize;
    for event in events {
        if let Some(payload) = event.payload.downcast_ref::<OperationStartPayload>() {
            nodes.insert(payload.node_id);
        }
        if event.payload.downcast_ref::<GraphEdgePayload>().is_some() {
            edges += 1;
        }
        if event.payload.downcast_ref::<ToolStartPayload>().is_some() {
            tool_calls += 1;
        }
    }
    RunTotals {
        events: events.len(),
        nodes: nodes.len(),
        edges,
        tool_calls,
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        total_requests: 0,
        cost_usd: None,
    }
}

fn build_graph(execution_id: &str, events: &[ApxmEvent]) -> RunGraph {
    let mut nodes: HashMap<u64, RunGraphNode> = HashMap::new();
    let mut edges: Vec<RunGraphEdge> = Vec::new();
    let mut agent_codes: HashMap<u64, String> = HashMap::new();
    let mut tool_names: HashMap<u64, String> = HashMap::new();

    for event in events {
        if let Some(payload) = event.payload.downcast_ref::<AgentSpawnedPayload>() {
            agent_codes.insert(payload.node_id, payload.agent_code.clone());
        }
        if let Some(payload) = event.payload.downcast_ref::<ToolStartPayload>() {
            // Tool events are not bound to a graph node id; fold them
            // into the active op span via the event's parent_span_id
            // is overkill here. For the graph view we still emit a
            // pseudo node so observers can see the tool call.
            let node_id = u64::MAX - (tool_names.len() as u64);
            tool_names.insert(node_id, payload.name.clone());
            nodes.insert(
                node_id,
                RunGraphNode {
                    id: node_id,
                    kind: RunNodeKind::Tool,
                    op_type: AISOperationType::InvTool,
                    agent_code: None,
                    tool_name: Some(payload.name.clone()),
                    status: NodeStatus::Running,
                    started_at_ms: Some(event.meta.timestamp.timestamp_millis()),
                    completed_at_ms: None,
                    duration_ms: None,
                    layer: None,
                    context: None,
                },
            );
        }
        if let Some(payload) = event.payload.downcast_ref::<OperationStartPayload>() {
            let kind = classify_op(payload.op_type);
            nodes
                .entry(payload.node_id)
                .and_modify(|node| {
                    if matches!(node.status, NodeStatus::Pending) {
                        node.status = NodeStatus::Running;
                    }
                })
                .or_insert_with(|| RunGraphNode {
                    id: payload.node_id,
                    kind,
                    op_type: payload.op_type,
                    agent_code: agent_codes.get(&payload.node_id).cloned(),
                    tool_name: None,
                    status: NodeStatus::Running,
                    started_at_ms: Some(event.meta.timestamp.timestamp_millis()),
                    completed_at_ms: None,
                    duration_ms: None,
                    layer: None,
                    context: payload.context.clone(),
                });
        }
        if let Some(payload) = event.payload.downcast_ref::<OperationEndPayload>()
            && let Some(node) = nodes.get_mut(&payload.node_id)
        {
            node.completed_at_ms = Some(event.meta.timestamp.timestamp_millis());
            node.duration_ms = Some(payload.duration_ms);
            node.status = if payload.success {
                NodeStatus::Succeeded
            } else {
                NodeStatus::Failed
            };
        }
        if let Some(payload) = event.payload.downcast_ref::<GraphEdgePayload>() {
            edges.push(RunGraphEdge {
                from: payload.from_node_id,
                to: payload.to_node_id,
                kind: payload.kind.clone(),
            });
        }
    }

    // Patch in agent_code on nodes if it was learned after the start event.
    for (node_id, node) in &mut nodes {
        if node.agent_code.is_none()
            && let Some(code) = agent_codes.get(node_id)
        {
            node.agent_code = Some(code.clone());
            node.kind = RunNodeKind::Agent;
        }
    }

    // Compute layer index via BFS from edges. Roots get layer 0.
    assign_layers(&mut nodes, &edges);

    let mut node_list: Vec<RunGraphNode> = nodes.into_values().collect();
    node_list.sort_by_key(|n| n.id);
    RunGraph {
        graph_schema_version: 1,
        execution_id: execution_id.to_string(),
        nodes: node_list,
        edges,
    }
}

fn classify_op(op: AISOperationType) -> RunNodeKind {
    match op {
        AISOperationType::SpawnAgent | AISOperationType::SpawnTeam => RunNodeKind::Agent,
        AISOperationType::InvTool => RunNodeKind::Tool,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason => {
            RunNodeKind::Llm
        }
        _ => RunNodeKind::Op,
    }
}

fn assign_layers(nodes: &mut HashMap<u64, RunGraphNode>, edges: &[RunGraphEdge]) {
    let mut incoming: HashMap<u64, Vec<u64>> = HashMap::new();
    for edge in edges {
        incoming.entry(edge.to).or_default().push(edge.from);
    }
    // Iterative relaxation over a snapshot/update map so the borrow
    // checker stays happy. Capped at `nodes.len()` iterations to
    // guarantee termination even on accidental cycles in the edge set.
    let len = nodes.len();
    for _ in 0..len {
        let snapshot: HashMap<u64, Option<u32>> =
            nodes.iter().map(|(id, node)| (*id, node.layer)).collect();
        let mut updates: HashMap<u64, u32> = HashMap::new();
        for id in nodes.keys() {
            let layer = incoming.get(id).map_or(0u32, |parents| {
                parents
                    .iter()
                    .filter_map(|p| snapshot.get(p).copied().flatten())
                    .max()
                    .map_or(0u32, |l| l + 1)
            });
            updates.insert(*id, layer);
        }
        let mut changed = false;
        for (id, layer) in updates {
            if let Some(node) = nodes.get_mut(&id)
                && node.layer != Some(layer)
            {
                node.layer = Some(layer);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// rollout-backed read paths.
//
// When the in-memory `RunEventBus` has aged out (server restart, ring
// rolled), we fall through to the rollout JSONL on disk. The rollout is
// the durable source of truth; the in-memory bus is the fast path.
// ────────────────────────────────────────────────────────────────────

/// Resolve the rollout file path for an execution_id by consulting the
/// SQLite index. Returns None when the index has no entry — e.g. the
/// rollout was never opened or the index is empty.
async fn rollout_file_for(state: &AppState, execution_id: &str) -> Option<std::path::PathBuf> {
    let index = state.rollout_index.lock().await;
    match index.get(execution_id) {
        Ok(Some(entry)) => Some(std::path::PathBuf::from(entry.file_path)),
        _ => None,
    }
}

/// Read events from disk and map them to ApxmEvent envelopes when possible.
/// Unparseable lines are silently skipped.
async fn events_from_disk(state: &AppState, execution_id: &str) -> Vec<ApxmEvent> {
    let Some(path) = rollout_file_for(state, execution_id).await else {
        return Vec::new();
    };
    let Ok((items, _)) = load_rollout(&path).await else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|line| match line.payload {
            apxm_rollout::RolloutPayload::Event(payload) => {
                serde_json::from_value::<ApxmEvent>(payload.event).ok()
            }
            _ => None,
        })
        .collect()
}

/// Get events for an execution_id, preferring the in-memory bus and falling
/// back to the rollout JSONL on disk. Returns an empty vec when neither
/// source has any events.
pub(crate) async fn events_for_run(state: &AppState, execution_id: &str) -> Vec<ApxmEvent> {
    let snapshot = state.run_event_bus.snapshot(execution_id);
    if !snapshot.is_empty() {
        return snapshot;
    }
    events_from_disk(state, execution_id).await
}

pub(crate) async fn events_for_run_since(
    state: &AppState,
    execution_id: &str,
    since: u64,
) -> Vec<ApxmEvent> {
    let snapshot = state.run_event_bus.snapshot(execution_id);
    if !snapshot.is_empty() {
        if snapshot
            .first()
            .is_none_or(|first_event| since >= first_event.meta.seq)
        {
            return snapshot;
        }
    }
    events_from_disk(state, execution_id).await
}

/// blob endpoint. Returns the original spilled blob from disk.
pub(crate) async fn get_run_blob(
    State(state): State<AppState>,
    Path((execution_id, blob_ref)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    // `blob_ref` is a content id used as a filename; reject path separators and
    // traversal so a crafted ref can't escape the run's `blobs/` dir and read
    // arbitrary files.
    if blob_ref.is_empty()
        || blob_ref.contains('/')
        || blob_ref.contains('\\')
        || blob_ref.contains("..")
    {
        return Err(ApiError::bad_request("invalid blob reference"));
    }
    // The index row gives us the rollout file path which encodes
    // started_at; we use the parent directory layout to resolve the
    // sibling `blobs/` directory rather than re-parsing the date out.
    let rollout_path = rollout_file_for(&state, &execution_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("run not found: {execution_id}")))?;
    let parent = rollout_path
        .parent()
        .ok_or_else(|| ApiError::not_found("invalid rollout path"))?;
    let stem = rollout_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let sidecar = parent.join(stem);
    let blob_path = sidecar.join("blobs").join(format!("{blob_ref}.json"));
    if blob_path.exists() {
        let bytes = tokio::fs::read(&blob_path)
            .await
            .map_err(|error| ApiError::internal_message(format!("failed to read blob: {error}")))?;
        return Ok(Response::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(bytes))
            .unwrap());
    }
    Err(ApiError::not_found(format!(
        "blob {blob_ref} not found for run {execution_id}"
    )))
}

// ────────────────────────────────────────────────────────────────────
// Session transcript history.
//
// `GET /v1/sessions/{id}/history` reassembles the visible conversation for
// a chat session from the durable rollout JSONLs. Every chat turn (CLI or
// studio) is one `/v1/execute/stream` execution that records under a shared
// `session_id`; this gathers all those per-turn rollouts and maps their
// event lines to an ordered, role-tagged message list.
// ────────────────────────────────────────────────────────────────────

/// One role-tagged message in a reconstructed conversation.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct HistoryMessage {
    /// `"user"`, `"assistant"`, or `"system"`.
    pub(crate) role: String,
    /// The message text. For the user side this is the redaction summary of
    /// the prompt (the raw prompt is never retained on the raw-execute path —
    /// see the handler docs); for the assistant side it is the model's reply.
    pub(crate) content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionHistoryResponse {
    pub(crate) messages: Vec<HistoryMessage>,
}

/// Map the rollout lines of a single turn to role-tagged messages.
///
/// The server's raw-execute rollout sink records the runtime event stream as
/// `RolloutPayload::Event` lines (it does not synthesize typed
/// `UserMessage`/`AssistantMessage` lines). We therefore reconstruct roles
/// from event kinds:
///
/// - `llm_prompt` → a `user` turn. The prompt text is redacted at the source
///   (`RedactedContent`), so `content` is the redaction summary.
/// - `llm_done` → an `assistant` turn carrying the full reply `content`
///   (authoritative; preferred over the streamed token fragments).
/// - `token` → assistant text fragments, concatenated only when no `llm_done`
///   line is present for the turn (streaming-only fallback).
///
/// Tool and lifecycle events are not part of the visible conversation and are
/// skipped. Typed `UserMessage`/`AssistantMessage` payloads are also honored
/// if a future writer emits them.
fn turn_messages_from_lines(lines: &[apxm_rollout::RolloutLine], out: &mut Vec<HistoryMessage>) {
    use apxm_rollout::RolloutPayload;

    let mut token_buf = String::new();
    let mut have_assistant_done = false;
    // When an explicit typed `UserMessage` line is present for this turn (the
    // streaming path writes one carrying the verbatim prompt), the runtime's own
    // redacted `llm_prompt` event would otherwise add a SECOND, redacted user
    // turn. Detect the typed line up front and skip the redacted prompt so the
    // faithful text appears exactly once.
    let has_typed_user_message = lines
        .iter()
        .any(|line| matches!(line.payload, RolloutPayload::UserMessage(_)));

    let flush_tokens = |buf: &mut String, out: &mut Vec<HistoryMessage>| {
        if !buf.is_empty() {
            out.push(HistoryMessage {
                role: "assistant".to_string(),
                content: std::mem::take(buf),
            });
        }
    };

    for line in lines {
        match &line.payload {
            RolloutPayload::UserMessage(msg) => {
                flush_tokens(&mut token_buf, out);
                out.push(HistoryMessage {
                    role: "user".to_string(),
                    content: content_blocks_to_text(&msg.content),
                });
            }
            RolloutPayload::AssistantMessage(msg) => {
                flush_tokens(&mut token_buf, out);
                out.push(HistoryMessage {
                    role: "assistant".to_string(),
                    content: content_blocks_to_text(&msg.content),
                });
                have_assistant_done = true;
            }
            RolloutPayload::Event(ev) => match ev.event_kind.as_str() {
                k if k == event_kind::LLM_PROMPT.name() => {
                    // A typed UserMessage line already carries the faithful user
                    // turn; don't also emit the redacted summary (no duplicate).
                    if has_typed_user_message {
                        continue;
                    }
                    flush_tokens(&mut token_buf, out);
                    let summary = ev
                        .event
                        .pointer("/payload/prompt/summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("user prompt")
                        .to_string();
                    out.push(HistoryMessage {
                        role: "user".to_string(),
                        content: summary,
                    });
                }
                k if k == event_kind::LLM_DONE.name() => {
                    // Authoritative assistant reply — discard any streamed
                    // token fragments for this turn to avoid duplication.
                    token_buf.clear();
                    let content = ev
                        .event
                        .pointer("/payload/content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    out.push(HistoryMessage {
                        role: "assistant".to_string(),
                        content,
                    });
                    have_assistant_done = true;
                }
                k if k == event_kind::TOKEN.name() => {
                    if let Some(text) = ev.event.pointer("/payload/text").and_then(|v| v.as_str()) {
                        token_buf.push_str(text);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    // Streaming-only turn (no llm_done / AssistantMessage): emit the
    // accumulated token text as the assistant reply. Otherwise the buffer was
    // already superseded by an authoritative assistant message.
    if have_assistant_done {
        token_buf.clear();
    } else {
        flush_tokens(&mut token_buf, out);
    }
}

/// Flatten an Anthropic-style content block list to plain text (Text blocks
/// only — the visible transcript is text, not tool plumbing).
fn content_blocks_to_text(blocks: &[apxm_rollout::ContentBlock]) -> String {
    use apxm_rollout::ContentBlock;
    let mut s = String::new();
    for block in blocks {
        if let ContentBlock::Text { text } = block {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(text);
        }
    }
    s
}

pub(crate) async fn get_session_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionHistoryResponse>, ApiError> {
    // Same charset/traversal validation the execute path enforces. Invalid
    // ids are a 400; an unknown-but-valid id is a 200 with no messages.
    let session_id = crate::execute::validate_session_id(id)?;

    let entries = {
        let index = state.rollout_index.lock().await;
        index.list_by_session(&session_id).unwrap_or_default()
    };

    // `list_by_session` returns newest-first; replay oldest-first so the
    // turns read in conversation order.
    let mut ordered = entries;
    ordered.sort_by(|a, b| a.started_at.cmp(&b.started_at));

    let mut messages: Vec<HistoryMessage> = Vec::new();
    for entry in ordered {
        let path = std::path::PathBuf::from(&entry.file_path);
        let Ok((items, _)) = load_rollout(&path).await else {
            continue;
        };
        // Collapse any compaction marker to its baseline + replay the suffix.
        let tree = apxm_rollout::reconstruct_history(&items);
        if !tree.baseline.is_empty() {
            // Baseline payloads carry no meta; wrap mapping over them.
            baseline_messages(&tree.baseline, &mut messages);
        }
        turn_messages_from_lines(&tree.suffix, &mut messages);
    }

    Ok(Json(SessionHistoryResponse { messages }))
}

/// Map a compaction baseline (bare payloads, no line meta) to messages.
fn baseline_messages(payloads: &[apxm_rollout::RolloutPayload], out: &mut Vec<HistoryMessage>) {
    use apxm_rollout::RolloutPayload;
    for payload in payloads {
        match payload {
            RolloutPayload::UserMessage(msg) => out.push(HistoryMessage {
                role: "user".to_string(),
                content: content_blocks_to_text(&msg.content),
            }),
            RolloutPayload::AssistantMessage(msg) => out.push(HistoryMessage {
                role: "assistant".to_string(),
                content: content_blocks_to_text(&msg.content),
            }),
            _ => {}
        }
    }
}

fn build_node_detail(
    execution_id: &str,
    node_id: u64,
    events: &[ApxmEvent],
    record: Option<&ExecutionRecord>,
) -> Option<RunNodeDetail> {
    let mut op_type: Option<AISOperationType> = None;
    let mut status = NodeStatus::Pending;
    let mut duration_ms: Option<u64> = None;
    let mut agent: Option<RunNodeAgentDetail> = None;
    let mut tool: Option<RunNodeToolDetail> = None;
    let mut filtered: Vec<serde_json::Value> = Vec::new();
    let mut seen = false;

    for event in events {
        let matches_node = if let Some(payload) =
            event.payload.downcast_ref::<OperationStartPayload>()
        {
            if payload.node_id == node_id {
                op_type = Some(payload.op_type);
                status = NodeStatus::Running;
                true
            } else {
                false
            }
        } else if let Some(payload) = event.payload.downcast_ref::<OperationEndPayload>() {
            if payload.node_id == node_id {
                duration_ms = Some(payload.duration_ms);
                status = if payload.success {
                    NodeStatus::Succeeded
                } else {
                    NodeStatus::Failed
                };
                true
            } else {
                false
            }
        } else if let Some(payload) = event.payload.downcast_ref::<AgentSpawnedPayload>() {
            if payload.node_id == node_id {
                agent = Some(RunNodeAgentDetail {
                    agent_code: payload.agent_code.clone(),
                    profile: payload.profile.clone(),
                });
                true
            } else {
                false
            }
        } else if let Some(payload) = event.payload.downcast_ref::<CommunicateDispatchedPayload>() {
            payload.node_id == node_id
        } else {
            false
        };

        if matches_node {
            seen = true;
            filtered.push(serde_json::to_value(event).unwrap_or(serde_json::Value::Null));
        }
    }

    // Tool detail: pull from the first ToolStart whose surrounding
    // span matches `node_id`. The cheap approximation here is to scan
    // for ToolStart/ToolEnd pairs in the event stream and bind them
    // when the node has no other identity (e.g. an INV_TOOL node).
    for event in events {
        if let Some(payload) = event.payload.downcast_ref::<ToolStartPayload>()
            && tool.is_none()
        {
            tool = Some(RunNodeToolDetail {
                tool_name: payload.name.clone(),
                args: serde_json::to_value(&payload.args).ok(),
                result: None,
            });
        }
        if let Some(payload) = event.payload.downcast_ref::<ToolEndPayload>()
            && let Some(ref mut existing) = tool
            && existing.tool_name == payload.name
        {
            existing.result = Some(payload.result.clone());
        }
    }

    let outputs: Vec<NodeOutputRecord> = record
        .map(|record| {
            record
                .node_outputs
                .iter()
                .filter(|output| output.node_id == node_id)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let metrics: Vec<NodeMetricsRecord> = record
        .map(|record| {
            record
                .node_metrics
                .iter()
                .filter(|metrics| metrics.node_id == node_id)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if matches!(status, NodeStatus::Pending)
        && let Some(latest) = metrics.last()
    {
        if latest.metrics.operation.failures > 0 {
            status = NodeStatus::Failed;
        } else if latest.metrics.operation.successes > 0 {
            status = NodeStatus::Succeeded;
        }
        duration_ms = Some(latest.metrics.operation.total_duration_ms);
    }

    if !seen && outputs.is_empty() && metrics.is_empty() {
        return None;
    }

    let artifacts =
        record.and_then(|record| run_node_artifact_refs(record, node_id, &outputs, &metrics));

    Some(RunNodeDetail {
        execution_id: execution_id.to_string(),
        node_id,
        op_type,
        status,
        duration_ms,
        events: filtered,
        outputs,
        metrics,
        artifacts,
        agent,
        tool,
    })
}

fn run_node_artifact_refs(
    record: &ExecutionRecord,
    node_id: u64,
    outputs: &[NodeOutputRecord],
    metrics: &[NodeMetricsRecord],
) -> Option<RunNodeArtifactRefs> {
    let run_root = record.run_root.as_ref()?;
    if let Some(saved) = record
        .node_artifacts
        .iter()
        .find(|artifact| artifact.node_id == node_id)
    {
        return Some(RunNodeArtifactRefs {
            run_root: run_root.clone(),
            node_json: saved.node_json.clone(),
            output_json: saved.output_json.clone(),
            metrics_json: saved.metrics_json.clone(),
            node_dir: saved.node_dir.clone(),
        });
    }
    let node_name = outputs
        .last()
        .and_then(|output| output.node_name.as_deref())
        .or_else(|| {
            metrics
                .last()
                .and_then(|metrics| metrics.node_name.as_deref())
        });
    let dir_name = session_node_dir_name(node_id, node_name.unwrap_or("node"));
    let node_dir = format!("{}/{}", constants::session::files::NODES_DIR, dir_name);
    Some(RunNodeArtifactRefs {
        run_root: run_root.clone(),
        node_json: format!("{}/{}", node_dir, constants::session::node::NODE_JSON),
        output_json: Some(format!(
            "{}/{}",
            node_dir,
            constants::session::node::OUTPUT_JSON
        )),
        metrics_json: Some(format!(
            "{}/{}",
            node_dir,
            constants::session::node::METRICS_JSON
        )),
        node_dir,
    })
}
