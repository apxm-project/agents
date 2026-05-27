//! Phase 14.8.B — Observer endpoints (`/v1/runs/...`).
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
//! Backwards-compatible: the producer-side `/v1/skills/.../execute`
//! routes are untouched and observers that don't know about
//! `/v1/runs` simply ignore it.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use apxm_core::events::payload::{
    AgentSpawnedPayload, CommunicateDispatchedPayload, GraphEdgePayload, OperationEndPayload,
    OperationStartPayload, ToolEndPayload, ToolStartPayload,
};
use apxm_core::events::{ApxmEvent, EventEmitter};
use apxm_core::types::operations::AISOperationType;
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

use crate::error::ApiError;
use crate::executions::{ExecutionRecord, ExecutionStatus};
use crate::state::AppState;

/// Default cap on `/v1/runs` listings to keep responses bounded.
const DEFAULT_LIST_LIMIT: usize = 200;
/// Default page size for the bulk events endpoint.
const DEFAULT_EVENTS_LIMIT: usize = 500;
/// Cap on the broadcast channel buffer per run — events older than this
/// are still in the on-disk ring but no longer streamed live.
const STREAM_BUFFER: usize = 1024;

// ────────────────────────────────────────────────────────────────────
// RunEventBus — owns retained events + live broadcast per execution.
// ────────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub(crate) struct RunEventBus {
    inner: Arc<DashMap<String, RunEventState>>,
}

struct RunEventState {
    events: Vec<ApxmEvent>,
    tx: broadcast::Sender<ApxmEvent>,
}

impl RunEventState {
    fn new() -> Self {
        let (tx, _rx) = broadcast::channel(STREAM_BUFFER);
        Self {
            events: Vec::new(),
            tx,
        }
    }
}

impl RunEventBus {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record an event into the per-execution ring and fan out to live
    /// subscribers. The trace_id on the event meta is treated as the
    /// execution id — that's how the runtime currently stamps events.
    pub(crate) fn record(&self, execution_id: &str, event: ApxmEvent) {
        let mut entry = self
            .inner
            .entry(execution_id.to_string())
            .or_insert_with(RunEventState::new);
        entry.events.push(event.clone());
        // No-subscriber send errors are non-fatal — only live observers care.
        let _ = entry.tx.send(event);
    }

    /// All events recorded for `execution_id` so far, in arrival order.
    pub(crate) fn snapshot(&self, execution_id: &str) -> Vec<ApxmEvent> {
        self.inner
            .get(execution_id)
            .map(|entry| entry.events.clone())
            .unwrap_or_default()
    }

    /// Subscribe to live events for `execution_id`. Subscribers get
    /// only events emitted after the subscription is taken; replay is
    /// done via the snapshot above.
    pub(crate) fn subscribe(&self, execution_id: &str) -> broadcast::Receiver<ApxmEvent> {
        let entry = self
            .inner
            .entry(execution_id.to_string())
            .or_insert_with(RunEventState::new);
        entry.tx.subscribe()
    }

    /// Test helper: returns true if any events are recorded for the run.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn has_run(&self, execution_id: &str) -> bool {
        self.inner.contains_key(execution_id)
    }
}

/// EventEmitter that funnels every event into the run bus, keyed by
/// the event's trace_id (which is the execution_id for runtime events).
pub(crate) struct RunBusEmitter {
    bus: RunEventBus,
    execution_id: String,
}

impl RunBusEmitter {
    pub(crate) fn new(bus: RunEventBus, execution_id: impl Into<String>) -> Self {
        Self {
            bus,
            execution_id: execution_id.into(),
        }
    }
}

impl EventEmitter for RunBusEmitter {
    fn emit(&self, event: ApxmEvent) {
        self.bus.record(&self.execution_id, event);
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
    pub(crate) status: ExecutionStatus,
    pub(crate) started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at_ms: Option<u64>,
    pub(crate) session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) root_agent: Option<String>,
    pub(crate) totals: RunTotals,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RunTotals {
    pub(crate) events: usize,
    pub(crate) nodes: usize,
    pub(crate) edges: usize,
    pub(crate) tool_calls: usize,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<RunNodeAgentDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool: Option<RunNodeToolDetail>,
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

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RunsListQuery {
    #[serde(default)]
    pub(crate) session_id: Option<String>,
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
    let limit = query.limit.unwrap_or(DEFAULT_LIST_LIMIT);
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
        .filter(|record| match &status_filter {
            Some(filter) => record.status == *filter,
            None => true,
        })
        .map(|record| record_to_summary(&state, record))
        .take(limit)
        .collect();
    // Phase 14.8.E — fall back to the SQLite index when the in-memory
    // execution store has nothing for this filter. Useful after a
    // restart: the rollout JSONLs survive and the index points at them.
    if runs.is_empty() {
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
        status,
        started_at_ms,
        completed_at_ms: None,
        session_id: entry.session_id,
        root_agent: entry.agent_code,
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

pub(crate) async fn get_run_graph(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Result<Json<RunGraph>, ApiError> {
    let events = events_for_run(&state, &execution_id).await;
    // Surface 404 cleanly when neither the execution store, the in-memory
    // bus, nor the on-disk rollout knows about this run.
    if state.execution_store.get(&execution_id).is_none() && events.is_empty() {
        return Err(ApiError::not_found(format!("run not found: {execution_id}")));
    }
    Ok(Json(build_graph(&execution_id, &events)))
}

pub(crate) async fn get_run_node(
    State(state): State<AppState>,
    Path((execution_id, node_id)): Path<(String, u64)>,
) -> Result<Json<RunNodeDetail>, ApiError> {
    let events = events_for_run(&state, &execution_id).await;
    if events.is_empty() && state.execution_store.get(&execution_id).is_none() {
        return Err(ApiError::not_found(format!("run not found: {execution_id}")));
    }
    build_node_detail(&execution_id, node_id, &events)
        .map(Json)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "node {node_id} not found for run {execution_id}"
            ))
        })
}

pub(crate) async fn get_run_events_bulk(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventsBulkResponse>, ApiError> {
    let events = events_for_run(&state, &execution_id).await;
    if events.is_empty() && state.execution_store.get(&execution_id).is_none() {
        return Err(ApiError::not_found(format!("run not found: {execution_id}")));
    }
    let since = query.since.unwrap_or(0);
    let limit = query.limit.unwrap_or(DEFAULT_EVENTS_LIMIT);

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
    // Fall back to the on-disk rollout when the in-memory bus has aged
    // out. The 404 only triggers when nothing at all knows about the run.
    let disk_events = if state.run_event_bus.snapshot(&execution_id).is_empty() {
        events_from_disk(&state, &execution_id).await
    } else {
        Vec::new()
    };
    if state.execution_store.get(&execution_id).is_none()
        && state.run_event_bus.snapshot(&execution_id).is_empty()
        && disk_events.is_empty()
    {
        return Err(ApiError::not_found(format!("run not found: {execution_id}")));
    }

    // Resolve the resume cursor: Last-Event-ID header wins (W3C SSE
    // contract), falling back to the ?since= query, then 0.
    let last_event_id = headers
        .get("Last-Event-ID")
        .or_else(|| headers.get("last-event-id"))
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let since = last_event_id.or(query.since).unwrap_or(0);

    let snapshot = if state.run_event_bus.snapshot(&execution_id).is_empty() {
        disk_events
    } else {
        state.run_event_bus.snapshot(&execution_id)
    };
    let rx = state.run_event_bus.subscribe(&execution_id);

    let replay = snapshot
        .into_iter()
        .filter(move |event| event.meta.seq >= since);

    let live = BroadcastStream::new(rx).filter_map(|item| async move {
        // RecvError::Lagged → keep streaming, just skip the missed slots.
        item.ok()
    });
    use futures::StreamExt as _;
    let combined = futures::stream::iter(replay).chain(live);

    let stream = combined.map(|event| {
        let id = event.meta.seq.to_string();
        let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        Ok(Event::default()
            .event(event.kind().name())
            .id(id)
            .data(data))
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
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

fn record_to_summary(state: &AppState, record: ExecutionRecord) -> RunSummary {
    let events = state.run_event_bus.snapshot(&record.execution_id);
    let totals = run_totals(&events);
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
        status: record.status,
        started_at_ms: record.started_at_ms,
        completed_at_ms: record.completed_at_ms,
        session_id: record.session_id,
        root_agent,
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
// Phase 14.8.E — rollout-backed read paths.
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

/// Phase 14.8.E — blob endpoint. Returns the original spilled blob from disk.
pub(crate) async fn get_run_blob(
    State(state): State<AppState>,
    Path((execution_id, blob_ref)): Path<(String, String)>,
) -> Result<Response, ApiError> {
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
    // Try both common extensions — blobs are serialized as JSON today but
    // the layout allows for `.txt` legacy blobs.
    let candidates = ["json", "txt"];
    for ext in candidates {
        let blob_path = sidecar.join("blobs").join(format!("{blob_ref}.{ext}"));
        if blob_path.exists() {
            let bytes = tokio::fs::read(&blob_path).await.map_err(|error| {
                ApiError::internal_message(format!("failed to read blob: {error}"))
            })?;
            let mime = if ext == "json" {
                "application/json"
            } else {
                "text/plain"
            };
            return Ok(Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(bytes))
                .unwrap());
        }
    }
    Err(ApiError::not_found(format!(
        "blob {blob_ref} not found for run {execution_id}"
    )))
}

fn build_node_detail(
    execution_id: &str,
    node_id: u64,
    events: &[ApxmEvent],
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
        } else if let Some(payload) =
            event.payload.downcast_ref::<CommunicateDispatchedPayload>()
        {
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

    if !seen {
        return None;
    }

    Some(RunNodeDetail {
        execution_id: execution_id.to_string(),
        node_id,
        op_type,
        status,
        duration_ms,
        events: filtered,
        agent,
        tool,
    })
}
