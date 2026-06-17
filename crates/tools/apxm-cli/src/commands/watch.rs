//!.F — `apxm watch <thread_id>` SSE watcher.
//!
//! Opens `GET /v1/runs/<thread_id>/events/stream` and renders the same
//! per-specialist tree the chat panel renders, refreshed line-by-line as
//! events arrive. Uses a hand-rolled `text/event-stream` parser sitting on
//! top of `reqwest::Response::bytes_stream()` — the workspace already
//! pulls in `futures` + `tokio-stream` for the LLM backends, so we don't
//! drag in `reqwest-eventsource` just for this one consumer.

use std::io::Write as _;

use anyhow::{Context, Result, anyhow};
use apxm_client::{
    client_for_sse,
    events::{event_kind, parse_approval_prompt},
    ClientInfo, DEFAULT_SERVER_BASE,
};
use apxm_core::events::ApxmEvent;
use apxm_client::reqwest;
use futures::StreamExt;
use serde_json::Value as JsonValue;

use super::render::{RunSnapshot, render_tree};
use super::sse_permissions::maybe_answer_permission;

/// Parameters for the watch command. Kept as a struct so the CLI surface
/// can grow flags without bloating the function signature.
#[derive(Debug, Clone)]
pub struct WatchOptions {
    pub thread_id: String,
    pub server_base: String,
    /// Optional explicit node_id to drill into via /v1/runs/<id>/nodes/<n>
    /// before the live stream starts. Mirrors the chat panel's drawer.
    pub expand_node_id: Option<u64>,
}

impl WatchOptions {
    pub fn new(thread_id: impl Into<String>) -> Self {
        let server_base =
            std::env::var("APXM_SERVER_BASE").unwrap_or_else(|_| DEFAULT_SERVER_BASE.to_string());
        Self {
            thread_id: thread_id.into(),
            server_base,
            expand_node_id: None,
        }
    }
}

pub async fn watch_command(thread_id: String, expand: Option<u64>) -> Result<()> {
    let mut opts = WatchOptions::new(thread_id);
    opts.expand_node_id = expand;
    watch_with_options(opts).await
}

/// Run the watcher with explicit options. The CLI entry point is a thin
/// wrapper around this; tests drive it directly so they don't have to
/// poke env vars.
pub async fn watch_with_options(opts: WatchOptions) -> Result<()> {
    let client = client_for_sse(&opts.server_base);
    let http = client.client().clone();
    // Optional one-shot node detail pull (the `Ctrl+O` gesture from the
    // plan; exposed here as `--expand <node_id>` so tests can drive it
    // without a real terminal). Printed to stderr so the live tree on
    // stdout stays parseable.
    if let Some(node_id) = opts.expand_node_id {
        match fetch_node_detail(&http, &opts.server_base, &opts.thread_id, node_id).await {
            Ok(detail) => eprintln!(
                "── node {node_id} detail ──\n{}\n──────────────────────────",
                serde_json::to_string_pretty(&detail).unwrap_or_default()
            ),
            Err(error) => eprintln!("(node detail fetch failed: {error})"),
        }
    }
    let mut snapshot = RunSnapshot::new(opts.thread_id.clone());
    let url = format!(
        "{base}/v1/runs/{tid}/events/stream",
        base = opts.server_base.trim_end_matches('/'),
        tid = opts.thread_id,
    );
    let response = http
        .get(&url)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "apxm watch: server returned {} for {url}",
            response.status()
        ));
    }
    let mut stream = response.bytes_stream();
    let mut parser = SseParser::default();
    let stdout = std::io::stdout();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("SSE chunk read failed")?;
        for frame in parser.feed(&chunk) {
            if !frame.data.is_empty() {
                if let Ok(json) = serde_json::from_str::<JsonValue>(&frame.data) {
                    if event_kind(&json) == Some("approval_request") {
                        if let Some(prompt) = parse_approval_prompt(&json) {
                            eprintln!(
                                "\n[permission] awaiting reply for tool '{}' ({})",
                                prompt.tool_name, prompt.approval_id
                            );
                        }
                        let _ = maybe_answer_permission(&client, &json).await;
                    }
                }
            }
            if let Some(event) = decode_event_frame(&frame) {
                snapshot.apply(&event);
                // Re-render in place: clear screen + redraw the tree.
                // Use ANSI rather than crossterm here so the dependency
                // stays optional for the watch path.
                let mut handle = stdout.lock();
                let _ = write!(handle, "\x1B[2J\x1B[H");
                let _ = writeln!(handle, "{}", render_tree(&snapshot));
                let _ = handle.flush();
            }
        }
    }
    // Print a final summary so callers piping into a file see a
    // terminated tree.
    println!();
    println!("(stream ended)");
    println!("{}", render_tree(&snapshot));
    Ok(())
}

/// Pull `/v1/runs/<thread>/nodes/<node>` and return the raw JSON detail
/// the chat-panel drawer renders. Public so the test harness can drive
/// the expand path without going through the watch loop.
pub async fn fetch_node_detail(
    client: &reqwest::Client,
    server_base: &str,
    thread_id: &str,
    node_id: u64,
) -> Result<serde_json::Value> {
    let url = format!(
        "{base}/v1/runs/{thread_id}/nodes/{node_id}",
        base = server_base.trim_end_matches('/'),
    );
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "apxm watch: node detail fetch returned {} for {url}",
            response.status()
        ));
    }
    Ok(response.json::<serde_json::Value>().await?)
}

/// Decoded SSE frame — only carries `event:` + `data:` fields, which is
/// all the apxm-server emits today.
#[derive(Debug, Clone, Default)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
}

/// Streaming parser for `text/event-stream`. Holds a small carry buffer
/// across chunk boundaries so a frame split mid-line still parses.
#[derive(Debug, Default)]
pub struct SseParser {
    buf: String,
    current: SseFrame,
    has_current: bool,
}

impl SseParser {
    /// Feed one byte chunk; return the frames that completed in this call.
    /// Robustly handles arbitrary chunk boundaries (the splitter only
    /// emits when it sees a blank line; partial lines stay in `buf`).
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseFrame> {
        // SSE is required to be UTF-8; lossy decode keeps the parser from
        // panicking on a transient bad chunk.
        let s = String::from_utf8_lossy(bytes);
        self.buf.push_str(&s);
        let mut out = Vec::new();
        while let Some(line_end) = self.buf.find('\n') {
            let raw_line = self.buf[..line_end].trim_end_matches('\r').to_string();
            self.buf.drain(..=line_end);
            if raw_line.is_empty() {
                if self.has_current {
                    out.push(std::mem::take(&mut self.current));
                    self.has_current = false;
                }
                continue;
            }
            if let Some((field, value)) = parse_sse_line(&raw_line) {
                self.has_current = true;
                match field {
                    "event" => self.current.event = Some(value.to_string()),
                    "data" => {
                        if !self.current.data.is_empty() {
                            self.current.data.push('\n');
                        }
                        self.current.data.push_str(value);
                    }
                    "id" => self.current.id = Some(value.to_string()),
                    _ => {}
                }
            }
        }
        out
    }
}

fn parse_sse_line(line: &str) -> Option<(&str, &str)> {
    // Comments (lines starting with ':') and malformed lines are ignored.
    if line.starts_with(':') {
        return None;
    }
    let (field, rest) = line.split_once(':')?;
    // Per spec, a single leading space after the colon is stripped.
    Some((field, rest.strip_prefix(' ').unwrap_or(rest)))
}

pub(crate) fn decode_event_frame(frame: &SseFrame) -> Option<ApxmEvent> {
    if frame.data.is_empty() {
        return None;
    }
    // The apxm-server emits the event in two ways: the `event:` field
    // carries the kind name, and the `data:` field carries the full
    // ApxmEvent JSON. We rely on the data field — the kind line is
    // already inside the envelope.
    serde_json::from_str::<ApxmEvent>(&frame.data).ok()
}

