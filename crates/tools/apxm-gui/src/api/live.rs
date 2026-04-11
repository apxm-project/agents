//! Live SSE streaming for APXM session traces.
//!
//! Provides Server-Sent Events endpoints that tail session files in real time,
//! enabling the GUI to stream execution progress as it happens.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use axum::extract::{Path as AxumPath, Query};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Json;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Interval between mtime polls for file watchers.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Interval between SSE heartbeat pings.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Terminal session statuses — stream closes when manifest reaches one of these.
const TERMINAL_STATUSES: &[&str] = &["completed", "failed"];

// ---------------------------------------------------------------------------
// Query parameter types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SessionQuery {
    /// Path to the session directory.
    pub path: String,
}

#[derive(Deserialize)]
pub struct NodeQuery {
    /// Path to the session directory.
    pub session: String,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub graph_name: Option<String>,
    pub status: String,
    pub started_at: String,
    pub duration_ms: Option<u64>,
    pub node_count: Option<u64>,
    /// Epoch seconds of manifest.json last modification — enables staleness detection.
    pub mtime_epoch: Option<u64>,
}

// ---------------------------------------------------------------------------
// File watcher helper
// ---------------------------------------------------------------------------

/// Watches a file for new content by polling its mtime every [`POLL_INTERVAL`].
///
/// Returns a [`mpsc::Receiver`] that yields new content appended since the last
/// read. The sender half is driven by a background task that runs until the
/// receiver is dropped.
pub fn watch_file(path: PathBuf) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel::<String>(64);

    tokio::spawn(async move {
        let mut last_pos: u64 = 0;
        let mut last_mtime: Option<SystemTime> = None;

        loop {
            // Check if receiver has been dropped.
            if tx.is_closed() {
                break;
            }

            // Try to get file metadata for mtime comparison.
            let meta = match tokio::fs::metadata(&path).await {
                Ok(m) => m,
                Err(_) => {
                    // File doesn't exist yet — wait and retry.
                    tokio::time::sleep(POLL_INTERVAL).await;
                    continue;
                }
            };

            let current_mtime = meta.modified().ok();

            // Only read if the file was modified since our last check.
            let changed = match (last_mtime, current_mtime) {
                (Some(prev), Some(cur)) => cur > prev,
                (None, Some(_)) => true,
                _ => false,
            };

            if changed || last_pos == 0 {
                match tokio::fs::File::open(&path).await {
                    Ok(mut file) => {
                        let file_len = meta.len();

                        if file_len > last_pos {
                            // Seek to where we left off and read new data.
                            if let Err(e) =
                                file.seek(std::io::SeekFrom::Start(last_pos)).await
                            {
                                warn!(path = %path.display(), error = %e, "seek failed");
                                tokio::time::sleep(POLL_INTERVAL).await;
                                continue;
                            }

                            let mut buf = Vec::with_capacity((file_len - last_pos) as usize);
                            match file.read_to_end(&mut buf).await {
                                Ok(n) => {
                                    last_pos += n as u64;
                                    let content = String::from_utf8_lossy(&buf).into_owned();
                                    if !content.is_empty() {
                                        if tx.send(content).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(
                                        path = %path.display(),
                                        error = %e,
                                        "read failed"
                                    );
                                }
                            }
                        } else if file_len < last_pos {
                            // File was truncated — reset.
                            last_pos = 0;
                        }
                    }
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "open failed");
                    }
                }

                last_mtime = current_mtime;
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }

        debug!(path = %path.display(), "file watcher stopped");
    });

    rx
}

// ---------------------------------------------------------------------------
// SSE session stream handler
// ---------------------------------------------------------------------------

/// `GET /api/live/session?path=<session_dir>`
///
/// Opens the session's `trace.ndjson` and tails it, also watching `live.json`
/// for progress and `manifest.json` for status changes. Events are streamed as
/// SSE until the session reaches a terminal state.
pub async fn sse_session_stream(
    Query(params): Query<SessionQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<serde_json::Value>)>
{
    let session_dir = PathBuf::from(&params.path);
    if !session_dir.is_dir() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("session directory not found: {}", params.path)
            })),
        ));
    }

    let trace_path = session_dir.join("trace.ndjson");
    let live_path = session_dir.join("live.json");
    let manifest_path = session_dir.join("manifest.json");

    // Start file watchers.
    let mut trace_rx = watch_file(trace_path);
    let mut live_rx = watch_file(live_path);
    let mut manifest_rx = watch_file(manifest_path.clone());

    let stream = async_stream::stream! {
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        // Skip the first immediate tick.
        heartbeat.tick().await;

        loop {
            tokio::select! {
                // --- Trace events (NDJSON lines) ---
                Some(chunk) = trace_rx.recv() => {
                    for line in chunk.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        yield Ok(Event::default()
                            .event("trace")
                            .data(trimmed.to_owned()));
                    }
                }

                // --- Progress updates (full JSON file content) ---
                Some(content) = live_rx.recv() => {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        // live.json is a single JSON object rewritten each time.
                        // We send the latest complete content.
                        // Find the last complete JSON object in case of partial writes.
                        if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
                            yield Ok(Event::default()
                                .event("progress")
                                .data(trimmed.to_owned()));
                        }
                    }
                }

                // --- Manifest status changes ---
                Some(content) = manifest_rx.recv() => {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            yield Ok(Event::default()
                                .event("status")
                                .data(manifest.to_string()));

                            // Check for terminal status.
                            if let Some(status) = manifest.get("status").and_then(|s| s.as_str()) {
                                if TERMINAL_STATUSES.contains(&status) {
                                    debug!("session reached terminal status: {status}");
                                    break;
                                }
                            }
                        }
                    }
                }

                // --- Heartbeat ---
                _ = heartbeat.tick() => {
                    yield Ok(Event::default().comment("heartbeat"));
                }
            }
        }

        // Send final status before closing.
        if let Ok(content) = tokio::fs::read_to_string(&manifest_path).await {
            if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(content.trim()) {
                yield Ok(Event::default()
                    .event("status")
                    .data(manifest.to_string()));
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new().interval(HEARTBEAT_INTERVAL),
    ))
}

// ---------------------------------------------------------------------------
// SSE node output handler
// ---------------------------------------------------------------------------

/// `GET /api/live/node/:id?session=<session_dir>`
///
/// Watches a specific node's `output.json` and per-node `trace.ndjson` for
/// live updates as the node executes.
pub async fn sse_node_output(
    AxumPath(node_id): AxumPath<String>,
    Query(params): Query<NodeQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<serde_json::Value>)>
{
    let session_dir = PathBuf::from(&params.session);
    if !session_dir.is_dir() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("session directory not found: {}", params.session)
            })),
        ));
    }

    let nodes_dir = session_dir.join("nodes");

    // Find the node directory matching the given ID prefix (e.g. "01_spawn_architect").
    let node_dir = match find_node_dir(&nodes_dir, &node_id).await {
        Some(d) => d,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!("node directory not found for id: {node_id}")
                })),
            ));
        }
    };

    let output_path = node_dir.join("output.json");
    let trace_path = node_dir.join("trace.ndjson");
    let status_path = node_dir.join("status.json");

    let mut output_rx = watch_file(output_path);
    let mut trace_rx = watch_file(trace_path);
    let mut status_rx = watch_file(status_path);

    let stream = async_stream::stream! {
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat.tick().await;

        loop {
            tokio::select! {
                // --- Node trace events (NDJSON lines) ---
                Some(chunk) = trace_rx.recv() => {
                    for line in chunk.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        yield Ok(Event::default()
                            .event("trace")
                            .data(trimmed.to_owned()));
                    }
                }

                // --- Node output ---
                Some(content) = output_rx.recv() => {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
                            yield Ok(Event::default()
                                .event("output")
                                .data(trimmed.to_owned()));
                        }
                    }
                }

                // --- Node status changes ---
                Some(content) = status_rx.recv() => {
                    let trimmed = content.trim();
                    if !trimmed.is_empty() {
                        if let Ok(status) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            yield Ok(Event::default()
                                .event("status")
                                .data(status.to_string()));

                            // Close stream if node completed or failed.
                            if let Some(s) = status.get("status").and_then(|s| s.as_str()) {
                                if TERMINAL_STATUSES.contains(&s) {
                                    break;
                                }
                            }
                        }
                    }
                }

                // --- Heartbeat ---
                _ = heartbeat.tick() => {
                    yield Ok(Event::default().comment("heartbeat"));
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new().interval(HEARTBEAT_INTERVAL),
    ))
}

// ---------------------------------------------------------------------------
// List sessions handler
// ---------------------------------------------------------------------------

/// `GET /api/sessions`
///
/// Lists all session directories under `~/.apxm/sessions/`, returning an
/// array of session summaries parsed from each `manifest.json`.
pub async fn list_sessions(
) -> Result<Json<Vec<SessionInfo>>, (StatusCode, Json<serde_json::Value>)> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let sessions_dir = PathBuf::from(home).join(".apxm").join("sessions");

    if !sessions_dir.is_dir() {
        // No sessions directory yet — return empty list.
        return Ok(Json(Vec::new()));
    }

    let mut sessions: Vec<SessionInfo> = Vec::new();

    let mut entries = tokio::fs::read_dir(&sessions_dir).await.map_err(|e| {
        error!(error = %e, "failed to read sessions directory");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("failed to read sessions directory: {e}")
            })),
        )
    })?;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let manifest_path = entry_path.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }

        let content = match tokio::fs::read_to_string(&manifest_path).await {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "failed to read manifest, skipping"
                );
                continue;
            }
        };

        let manifest: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "invalid manifest JSON, skipping"
                );
                continue;
            }
        };

        let id = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let graph_name = manifest
            .get("graph_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let status = manifest
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let started_at = manifest
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let duration_ms = manifest
            .get("duration_ms")
            .and_then(|v| v.as_u64());

        let node_count = manifest
            .get("node_count")
            .and_then(|v| v.as_u64());

        let mtime_epoch = tokio::fs::metadata(&manifest_path)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        sessions.push(SessionInfo {
            id,
            graph_name,
            status,
            started_at,
            duration_ms,
            node_count,
            mtime_epoch,
        });
    }

    // Sort by started_at descending (most recent first).
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    Ok(Json(sessions))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find a node directory under `nodes_dir` matching the given node ID.
///
/// Node directories are named like `01_spawn_architect`. We match by the
/// leading numeric prefix: both `"1"` and `"01"` match `01_*`.
async fn find_node_dir(nodes_dir: &Path, node_id: &str) -> Option<PathBuf> {
    let mut entries = tokio::fs::read_dir(nodes_dir).await.ok()?;

    let parsed_id: Option<u64> = node_id.parse().ok();

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Try exact prefix match: "01_" for node_id "01"
        if name_str.starts_with(&format!("{}_", node_id)) {
            return Some(entry.path());
        }

        // Try zero-padded match: node_id "1" should match "01_"
        if let Some(id) = parsed_id {
            if name_str.starts_with(&format!("{:02}_", id)) {
                return Some(entry.path());
            }
        }
    }

    None
}
