//! Session control API: status, cancel, grants, compact, events.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::time::Duration;

use apxm_core::events::{ApxmEvent, kind as event_kind};
use apxm_runtime::executor::session_ledger::{self, SessionLedger};
use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::routes::ServerRoute;
use crate::runs::{EventReplayCursor, EventsQuery, run_events_limit};
use crate::state::AppState;
use crate::types::responses::OkAck;

const SESSION_COMPACT_KEEP_RECENT: i64 = 4;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct SessionStatus {
    pub(crate) session_id: String,
    pub(crate) turn_count: usize,
    pub(crate) ledger: SessionLedgerView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) active_execution_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub(crate) struct SessionLedgerView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) turn_cap: Option<usize>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub(crate) tool_budgets: HashMap<String, usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) grants: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default, ToSchema)]
pub(crate) struct GrantUpdate {
    #[serde(default)]
    pub(crate) add: Vec<String>,
    #[serde(default)]
    pub(crate) remove: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct SessionEventsResponse {
    pub(crate) session_id: String,
    pub(crate) events: Vec<serde_json::Value>,
    pub(crate) next_seq: u64,
    pub(crate) done: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct CompactSessionResponse {
    pub(crate) ok: bool,
    pub(crate) session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) folded_turns: Option<usize>,
}

#[derive(Debug, Clone)]
struct SessionEventRecord {
    session_seq: u64,
    event: ApxmEvent,
}

enum SessionSseItem {
    Event(SessionEventRecord),
    Lagged(u64),
}

/// Mount session control routes on an existing router (contract-test helper).
pub(crate) fn mount_routes(router: Router<AppState>) -> Router<AppState> {
    router
        .route(ServerRoute::SessionStatus.path(), get(get_session_status))
        .route(ServerRoute::SessionCancel.path(), post(cancel_session))
        .route(
            ServerRoute::SessionGrants.path(),
            post(update_session_grants),
        )
        .route(ServerRoute::SessionCompact.path(), post(compact_session))
        .route(ServerRoute::SessionEvents.path(), get(list_session_events))
        .route(
            ServerRoute::SessionEventsStream.path(),
            get(stream_session_events),
        )
}

pub(crate) async fn get_session_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionStatus>, ApiError> {
    let session_id = crate::execute::validate_session_id(session_id)?;
    session_status_for_state(&state, &session_id).map(Json)
}

pub(crate) async fn cancel_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<OkAck>, ApiError> {
    let session_id = crate::execute::validate_session_id(session_id)?;
    cancel_session_for_state(&state, &session_id)?;
    Ok(Json(OkAck::new()))
}

pub(crate) async fn update_session_grants(
    State(_state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<GrantUpdate>,
) -> Result<Json<OkAck>, ApiError> {
    let session_id = crate::execute::validate_session_id(session_id)?;
    update_grants_for_state(&session_id, req)?;
    Ok(Json(OkAck::new()))
}

pub(crate) async fn compact_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<CompactSessionResponse>, ApiError> {
    let session_id = crate::execute::validate_session_id(session_id)?;
    compact_session_for_state(&state, &session_id)
        .await
        .map(Json)
}

pub(crate) async fn list_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<SessionEventsResponse>, ApiError> {
    let session_id = crate::execute::validate_session_id(session_id)?;
    let since = query.since.unwrap_or(0);
    let limit = run_events_limit(&state.server_config.run_events, query.limit);
    session_events_for_state(&state, &session_id, since, limit)
        .await
        .map(Json)
}

pub(crate) async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let session_id = crate::execute::validate_session_id(session_id)?;
    ensure_session_known(&state, &session_id)?;

    let cursor = EventReplayCursor::from_request(&headers, &query);
    let replay: Vec<SessionEventRecord> = collect_session_event_records(&state, &session_id)
        .await
        .into_iter()
        .filter(|record| cursor.accepts(record.session_seq))
        .collect();
    let replay_high_water = replay.iter().map(|record| record.session_seq).max();

    let execution_id = state
        .session_registry
        .get(&session_id)
        .map(|record| record.execution_id)
        .unwrap_or_default();
    let rx = state.run_event_bus.subscribe(&execution_id);

    use futures::StreamExt as _;
    let live = BroadcastStream::new(rx).filter_map(move |item| async move {
        match item {
            Ok(event) => {
                let session_seq = event.meta.seq;
                if cursor.accepts(session_seq)
                    && replay_high_water.is_none_or(|high| session_seq > high)
                {
                    Some(SessionSseItem::Event(SessionEventRecord {
                        session_seq,
                        event,
                    }))
                } else {
                    None
                }
            }
            Err(BroadcastStreamRecvError::Lagged(missed)) => Some(SessionSseItem::Lagged(missed)),
        }
    });

    let combined = futures::stream::iter(replay.into_iter().map(SessionSseItem::Event)).chain(live);

    let stream = combined.map(|item| {
        let event = match item {
            SessionSseItem::Event(record) => {
                let id = record.session_seq.to_string();
                let data = serde_json::to_string(&record.event).unwrap_or_else(|_| "{}".to_string());
                Event::default()
                    .event(record.event.kind().name())
                    .id(id)
                    .data(data)
            }
            SessionSseItem::Lagged(missed) => Event::default()
                .event(event_kind::ERROR.sse_event_type())
                .data(
                    serde_json::json!({
                        "message": "session event stream lagged; reconnect with Last-Event-ID to replay retained events and call session status",
                        "missed": missed,
                        "code": "stream_lag",
                    })
                    .to_string(),
                ),
        };
        Ok(event)
    });

    Ok(
        Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(
            state.server_config.run_events.keep_alive_secs.max(1),
        ))),
    )
}

pub(crate) fn session_status_for_state(
    state: &AppState,
    session_id: &str,
) -> Result<SessionStatus, ApiError> {
    ensure_session_known(state, session_id)?;
    let ledger = session_ledger::get(session_id);
    let turn_count = ledger
        .as_ref()
        .map(|ledger| ledger.turns_used())
        .unwrap_or(0);
    let ledger_view = ledger
        .as_ref()
        .map(|ledger| ledger_view_from_runtime(ledger.as_ref()))
        .unwrap_or_default();
    let active_execution_id = state
        .session_registry
        .get(session_id)
        .map(|record| record.execution_id);
    Ok(SessionStatus {
        session_id: session_id.to_string(),
        turn_count,
        ledger: ledger_view,
        active_execution_id,
    })
}

pub(crate) fn cancel_session_for_state(state: &AppState, session_id: &str) -> Result<(), ApiError> {
    let record = state
        .session_registry
        .get(session_id)
        .ok_or_else(|| ApiError::conflict("no in-flight session work to cancel"))?;
    match state.cancel_registry.get(&record.execution_id) {
        Some(notify) => {
            notify.notify_one();
            Ok(())
        }
        None => Err(ApiError::conflict("no in-flight session work to cancel")),
    }
}

pub(crate) fn update_grants_for_state(
    session_id: &str,
    update: GrantUpdate,
) -> Result<(), ApiError> {
    let ledger = session_ledger::get(session_id)
        .ok_or_else(|| ApiError::not_found(format!("unknown session: {session_id}")))?;
    if !update.add.is_empty() {
        ledger.add_grants(update.add);
    }
    if !update.remove.is_empty() {
        ledger.remove_grants(update.remove);
    }
    Ok(())
}

pub(crate) async fn compact_session_for_state(
    state: &AppState,
    session_id: &str,
) -> Result<CompactSessionResponse, ApiError> {
    ensure_session_known(state, session_id)?;
    let mem = state.runtime.memory();
    let count_key = "conversation:user_count";
    let count = mem
        .read_scoped(
            apxm_runtime::memory::MemorySpace::Stm,
            session_id,
            count_key,
        )
        .await
        .ok()
        .flatten()
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    if count <= SESSION_COMPACT_KEEP_RECENT {
        return Ok(CompactSessionResponse {
            ok: true,
            session_id: session_id.to_string(),
            folded_turns: Some(0),
        });
    }

    let fold_through = count - SESSION_COMPACT_KEEP_RECENT;
    let mut folded = Vec::new();
    for turn in 1..=fold_through {
        let key = format!("conversation:user:{turn}");
        if let Ok(Some(value)) = mem
            .read_scoped(apxm_runtime::memory::MemorySpace::Stm, session_id, &key)
            .await
            && let Some(text) = value.as_str()
        {
            folded.push(text.to_string());
        }
    }
    if folded.is_empty() {
        return Ok(CompactSessionResponse {
            ok: true,
            session_id: session_id.to_string(),
            folded_turns: Some(0),
        });
    }

    let summary = folded.join("\n");
    let _ = mem
        .write_scoped(
            apxm_runtime::memory::MemorySpace::Stm,
            session_id,
            "conversation:summary".to_string(),
            apxm_core::types::Value::String(summary),
        )
        .await;
    for turn in 1..=fold_through {
        let key = format!("conversation:user:{turn}");
        let _ = mem
            .delete_scoped(apxm_runtime::memory::MemorySpace::Stm, session_id, &key)
            .await;
    }
    let _ = mem
        .write_scoped(
            apxm_runtime::memory::MemorySpace::Stm,
            session_id,
            count_key.to_string(),
            apxm_core::types::Value::Number(apxm_core::types::values::Number::Integer(
                SESSION_COMPACT_KEEP_RECENT,
            )),
        )
        .await;

    Ok(CompactSessionResponse {
        ok: true,
        session_id: session_id.to_string(),
        folded_turns: Some(folded.len()),
    })
}

pub(crate) async fn session_events_for_state(
    state: &AppState,
    session_id: &str,
    since: u64,
    limit: usize,
) -> Result<SessionEventsResponse, ApiError> {
    ensure_session_known(state, session_id)?;
    let records = collect_session_event_records(state, session_id).await;
    let filtered: Vec<&SessionEventRecord> = records
        .iter()
        .filter(|record| record.session_seq >= since)
        .collect();
    let page: Vec<&SessionEventRecord> = filtered.iter().take(limit).copied().collect();
    let next_seq = page
        .last()
        .map_or(since, |record| record.session_seq.saturating_add(1));
    Ok(SessionEventsResponse {
        session_id: session_id.to_string(),
        events: page
            .into_iter()
            .map(|record| serde_json::to_value(&record.event).unwrap_or(serde_json::Value::Null))
            .collect(),
        next_seq,
        done: filtered.len() <= limit,
    })
}

fn ensure_session_known(state: &AppState, session_id: &str) -> Result<(), ApiError> {
    if session_known(state, session_id) {
        Ok(())
    } else {
        Err(ApiError::not_found(format!(
            "unknown session: {session_id}"
        )))
    }
}

fn session_known(state: &AppState, session_id: &str) -> bool {
    session_ledger::get(session_id).is_some()
        || state.session_registry.get(session_id).is_some()
        || state
            .execution_store
            .list()
            .into_iter()
            .any(|record| record.session_id == session_id)
}

async fn linked_execution_ids(state: &AppState, session_id: &str) -> Vec<String> {
    let mut ids = HashSet::new();
    if let Some(record) = state.session_registry.get(session_id) {
        ids.insert(record.execution_id);
    }
    for record in state.execution_store.list() {
        if record.session_id == session_id {
            ids.insert(record.execution_id);
        }
    }
    let index = state.rollout_index.lock().await;
    let index_entries = index.list_by_session(session_id).unwrap_or_default();
    for entry in index_entries {
        ids.insert(entry.thread_id);
    }
    if ids.is_empty() && session_ledger::get(session_id).is_some() {
        for execution_id in state.run_event_bus.list_execution_ids() {
            if !state.run_event_bus.snapshot(&execution_id).is_empty() {
                ids.insert(execution_id);
            }
        }
    }
    ids.into_iter().collect()
}

fn ledger_view_from_runtime(ledger: &SessionLedger) -> SessionLedgerView {
    let mut grants: Vec<String> = ledger.grants().into_iter().collect();
    grants.sort();
    SessionLedgerView {
        turn_cap: ledger.turn_cap(),
        tool_budgets: ledger.tool_budgets_remaining(),
        grants,
    }
}

async fn collect_session_event_records(
    state: &AppState,
    session_id: &str,
) -> Vec<SessionEventRecord> {
    let mut records = Vec::new();
    for execution_id in linked_execution_ids(state, session_id).await {
        for event in state.run_event_bus.snapshot(&execution_id) {
            records.push((execution_id.clone(), event));
        }
    }
    records.sort_by(|(left_id, left), (right_id, right)| {
        left.meta
            .timestamp
            .cmp(&right.meta.timestamp)
            .then_with(|| left_id.cmp(right_id))
            .then_with(|| left.meta.seq.cmp(&right.meta.seq))
    });
    records
        .into_iter()
        .enumerate()
        .map(|(session_seq, (_execution_id, event))| SessionEventRecord {
            session_seq: session_seq as u64,
            event,
        })
        .collect()
}

impl Default for SessionLedgerView {
    fn default() -> Self {
        Self {
            turn_cap: None,
            tool_budgets: HashMap::new(),
            grants: Vec::new(),
        }
    }
}
