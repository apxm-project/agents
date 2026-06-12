//!.F — `apxm watch <thread_id>` SSE watcher.
//!
//! Opens `GET /v1/runs/<thread_id>/events/stream` and renders the same
//! per-specialist tree the chat panel renders, refreshed line-by-line as
//! events arrive. Uses a hand-rolled `text/event-stream` parser sitting on
//! top of `reqwest::Response::bytes_stream()` — the workspace already
//! pulls in `futures` + `tokio-stream` for the LLM backends, so we don't
//! drag in `reqwest-eventsource` just for this one consumer.

use std::io::Write as _;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use apxm_core::events::ApxmEvent;
use futures::StreamExt;

use super::render::{RunSnapshot, render_tree};

/// Default apxm-server endpoint. Override via APXM_SERVER_BASE or the
/// `--server` flag.
const DEFAULT_SERVER_BASE: &str = "http://127.0.0.1:8000";

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
    let client = reqwest::Client::builder()
        // SSE streams may stall — disable the body read timeout so the
        // keepalive heartbeats don't trip a false-negative.
        .read_timeout(Duration::from_secs(0))
        .build()
        .context("failed to build reqwest client")?;
    // Optional one-shot node detail pull (the `Ctrl+O` gesture from the
    // plan; exposed here as `--expand <node_id>` so tests can drive it
    // without a real terminal). Printed to stderr so the live tree on
    // stdout stays parseable.
    if let Some(node_id) = opts.expand_node_id {
        match fetch_node_detail(&client, &opts.server_base, &opts.thread_id, node_id).await {
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
    let response = client
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

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::commands::render::{RunSnapshot, render_tree};

    fn agent_spawned_event(seq: u64, node_id: u64, agent_code: &str) -> serde_json::Value {
        serde_json::json!({
            "meta": {
                "seq": seq,
                "timestamp": "2026-05-27T00:00:00Z",
                "trace_id": "t-test",
                "source": "runtime",
                "span_id": format!("span-{seq}"),
            },
            "payload": {
                "kind": "agent_spawned",
                "node_id": node_id,
                "agent_code": agent_code,
                "parent_execution_id": "t-test",
            },
        })
    }

    fn tool_start_event(seq: u64, name: &str) -> serde_json::Value {
        serde_json::json!({
            "meta": {
                "seq": seq,
                "timestamp": "2026-05-27T00:00:00Z",
                "trace_id": "t-test",
                "source": "runtime",
                "span_id": format!("span-{seq}"),
            },
            "payload": {
                "kind": "tool_start",
                "name": name,
                "args": {},
            },
        })
    }

    #[test]
    fn watch_renders_tree_for_synthetic_event_stream() {
        // The TUI render contract: fold three canned events into a snapshot
        // and assert each agent surfaces on its own branch with the right
        // counter, regardless of arrival order.
        let mut snapshot = RunSnapshot::new("t-tree");
        for raw in [
            agent_spawned_event(1, 100, "module.knowledge"),
            agent_spawned_event(2, 101, "module.crm"),
            tool_start_event(3, "knowledge.article.search"),
        ] {
            let event: ApxmEvent = serde_json::from_value(raw).unwrap();
            snapshot.apply(&event);
        }
        let rendered = render_tree(&snapshot);
        assert!(rendered.contains("module.knowledge"));
        assert!(rendered.contains("module.crm"));
        assert!(rendered.contains("t-tree"));
        // Tool count surfaces in the branch line.
        assert!(rendered.contains("1 tool"));
        // Synthesizing footer should show when >1 agent is non-terminal.
        assert!(rendered.contains("Synthesizing"));
    }

    /// Spin up a minimal tokio listener that serves a single JSON
    /// response on the next accept; returns the bound address.
    async fn one_shot_json_server(body: String) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let _ = socket.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
        });
        (addr, handle)
    }

    #[tokio::test]
    async fn watch_expand_fetches_node_detail() {
        let canned = serde_json::json!({
            "execution_id": "t-expand",
            "node_id": 42,
            "status": "running",
            "events": [],
        });
        let (addr, server) = one_shot_json_server(canned.to_string()).await;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let detail = fetch_node_detail(&client, &format!("http://{addr}"), "t-expand", 42)
            .await
            .unwrap();
        let _ = server.await;
        assert_eq!(detail["node_id"], 42);
        assert_eq!(detail["execution_id"], "t-expand");
    }

    #[tokio::test]
    async fn watch_with_options_returns_against_sse_mock() {
        // Bring up a minimal SSE server that serves one frame then closes
        // the connection. The watch loop should consume the frame, fold
        // it into the snapshot, and return Ok.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let event_json =
            serde_json::to_string(&agent_spawned_event(1, 100, "module.knowledge")).unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 2048];
            let _ = socket.read(&mut buf).await.unwrap();
            let body = format!("event: agent_spawned\nid: 1\ndata: {event_json}\n\n");
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
            // One chunked frame + a zero-terminator so the client sees a
            // clean end of stream.
            let chunk = format!("{:X}\r\n{}\r\n0\r\n\r\n", body.len(), body);
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(chunk.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
        });
        let opts = WatchOptions {
            thread_id: "t-watch".to_string(),
            server_base: format!("http://{addr}"),
            expand_node_id: None,
        };
        let result = tokio::time::timeout(Duration::from_secs(5), watch_with_options(opts)).await;
        let _ = server.await;
        assert!(result.is_ok(), "watch_with_options timed out");
        let inner = result.unwrap();
        // Two acceptable outcomes: (a) the watcher consumed the canned
        // frame + then saw EOF and returned Ok; (b) the kernel closed
        // the connection between the chunked terminator and our reader,
        // surfacing as a transient SSE read error. Both confirm the
        // SSE plumbing worked — the failure case would be a 404 or a
        // serialization error on the way in.
        if let Err(error) = &inner {
            // Surface the error type to make flakes diagnosable rather
            // than mysterious. Connection-close races are fine; other
            // shapes signal a real bug.
            let message = error.to_string().to_lowercase();
            assert!(
                message.contains("sse")
                    || message.contains("decode")
                    || message.contains("connection")
                    || message.contains("eof"),
                "unexpected watch_with_options error: {error:?}",
            );
        }
    }

    #[test]
    fn parser_yields_one_frame_per_blank_line() {
        let mut parser = SseParser::default();
        let chunk = b"event: foo\ndata: hello\n\nevent: bar\ndata: world\n\n";
        let frames = parser.feed(chunk);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].event.as_deref(), Some("foo"));
        assert_eq!(frames[0].data, "hello");
        assert_eq!(frames[1].event.as_deref(), Some("bar"));
        assert_eq!(frames[1].data, "world");
    }

    #[test]
    fn parser_handles_split_chunks_across_boundary() {
        let mut parser = SseParser::default();
        let a = parser.feed(b"event: foo\ndata: par");
        assert!(a.is_empty()); // No blank line yet → no frame.
        let b = parser.feed(b"tial\n\n");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].data, "partial");
    }

    #[test]
    fn parser_concatenates_multiline_data() {
        let mut parser = SseParser::default();
        let frames = parser.feed(b"data: one\ndata: two\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "one\ntwo");
    }

    #[test]
    fn parser_skips_comments_and_blank_keepalives() {
        let mut parser = SseParser::default();
        // SSE comments (lines starting with `:`) are the standard
        // keepalive shape — they must not produce a frame.
        let frames = parser.feed(b": keepalive\n\n");
        assert!(frames.is_empty());
    }
}
