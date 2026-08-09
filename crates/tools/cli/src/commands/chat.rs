//! `apxm chat` — interactive conversational REPL over a running apxm-server.
//!
//! Thin protocol pipe: deliver user input, render SSE events, and answer server
//! permission prompts. Per-tool call budgets are enforced server-side — the
//! host tracks no budgets locally (constitution #2).

use std::io::Write as _;
use std::path::PathBuf;

use crate::client::{
    Client, ClientInfo, client_for_sse, execute::ExecuteRequest, resolve_server_base,
    types::SessionStatus,
};
use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use serde_json::Value as JsonValue;

use super::sse_permissions::maybe_answer_permission;
use super::watch::SseParser;

/// CLI options for the chat REPL.
#[derive(Debug, Clone)]
pub struct ChatOptions {
    pub air: Option<PathBuf>,
    pub server: Option<String>,
    pub session_id: Option<String>,
    pub capability_grant_ids: Vec<String>,
    /// Skill libraries / ids this agent imports (scoped visible set).
    pub import: Vec<String>,
    /// Pin each model call to a registered backend (agent chat only).
    pub backend: Option<String>,
    /// Pin each model call to a specific model id (agent chat only).
    pub model: Option<String>,
    /// Tenant/owner scope for per-tool credential resolution.
    pub owner: Option<String>,
    /// Agent id for thin server-backed chat
    /// (`POST /v1/agents/{{id}}/sessions`).
    pub agent: Option<String>,
}

fn mint_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("chat-{:x}", nanos)
}

fn required_capability_bindings(opts: &ChatOptions) -> Vec<String> {
    opts.capability_grant_ids
        .iter()
        .filter(|grant| !grant.starts_with("grant_"))
        .cloned()
        .collect()
}

async fn resolve_execute_capability_grant_ids(
    client: &Client,
    opts: &ChatOptions,
) -> Result<Vec<String>> {
    let mut grant_ids = opts
        .capability_grant_ids
        .iter()
        .filter(|grant| grant.starts_with("grant_"))
        .cloned()
        .collect::<Vec<_>>();
    for binding in required_capability_bindings(opts) {
        grant_ids.push(client.mint_capability_grant(&binding).await?);
    }
    Ok(grant_ids)
}

/// True when the artifact owns a typed host-input suspension point.
fn artifact_has_await_event(air: &str) -> bool {
    air.contains("\"await.event\"")
}

/// Dumb-pipe host (constitution #2): POST the artifact ONCE, pipe stdin lines to
/// the message-input endpoint, render streamed tokens. No transcript, no
/// compaction, no budgets host-side — all of that lives in the program/runtime.
async fn run_dumb_pipe(
    client: &Client,
    session_id: &str,
    air: &str,
    opts: &ChatOptions,
) -> Result<()> {
    let capability_grant_ids = resolve_execute_capability_grant_ids(client, opts).await?;
    let body = ExecuteRequest {
        air: air.to_string(),
        session_id: Some(session_id.to_string()),
        capability_grant_ids,
        imports: opts.import.clone(),
        owner: opts.owner.clone(),
        ..Default::default()
    };
    let resp = client
        .execute_stream(&body)
        .await
        .context("execute stream failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("server returned {status}: {text}"));
    }

    let client_c = client.clone();
    let render = tokio::spawn(render_session_stream(resp, client_c));

    use tokio::io::AsyncBufReadExt as _;
    let mut stdin_lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        eprint!("\nuser> ");
        let _ = std::io::stderr().flush();
        let line = match stdin_lines.next_line().await.context("stdin read failed")? {
            Some(l) => l.trim().to_string(),
            None => break,
        };
        if line.is_empty() {
            continue;
        }
        if matches!(line.as_str(), "/exit" | "/quit") {
            break;
        }
        let r = client.post_conversation_message(session_id, &line).await?;
        if !r.status().is_success() {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            eprintln!("message delivery failed: {status}: {text}");
        }
    }

    render.abort();
    Ok(())
}

/// What to do with one decoded SSE frame in the chat REPL.
///
/// `token`/`error` render their own way; every other kind gets a minimal
/// visible notice instead of being silently dropped — the CLI previously
/// parsed and discarded everything besides `token`/`error`
/// (`chat_renderer_surfaces_unknown_kind_instead_of_dropping`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum ChatFrameRender {
    /// Append raw token text to the answer stream (no newline).
    Token(String),
    /// A server-reported error.
    Error(String),
    /// A minimal notice for a Layer-2 (or any other) kind the chat REPL
    /// doesn't have bespoke rendering for yet.
    Notice(String),
}

/// Decide how to render one already-JSON-decoded SSE `/payload` frame.
/// Returns `None` only when the frame has no `payload.kind` at all (not a
/// real event envelope) — every recognized kind, known or not, renders
/// *something* visible, per the "fail loud, not silent" posture.
fn render_chat_frame(value: &JsonValue) -> Option<ChatFrameRender> {
    let kind = value.pointer("/payload/kind").and_then(|k| k.as_str())?;
    match kind {
        "token" => value
            .pointer("/payload/text")
            .and_then(|t| t.as_str())
            .map(|t| ChatFrameRender::Token(t.to_string())),
        "error" => value
            .pointer("/payload/message")
            .and_then(|m| m.as_str())
            .map(|m| ChatFrameRender::Error(m.to_string())),
        other => Some(ChatFrameRender::Notice(format!("[{other}]"))),
    }
}

fn apply_chat_frame_render(render: ChatFrameRender) {
    match render {
        ChatFrameRender::Token(tok) => {
            print!("{tok}");
            let _ = std::io::stdout().flush();
        }
        ChatFrameRender::Error(message) => eprintln!("\n[error] {message}"),
        ChatFrameRender::Notice(notice) => eprintln!("\n{notice}"),
    }
}

async fn render_session_stream(resp: crate::client::reqwest::Response, client: Client) {
    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::default();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else { break };
        for frame in parser.feed(&chunk) {
            let Ok(v) = serde_json::from_str::<JsonValue>(&frame.data) else {
                continue;
            };
            let _ = maybe_answer_permission(&client, &v).await;
            if let Some(render) = render_chat_frame(&v) {
                apply_chat_frame_render(render);
            }
        }
    }
}

/// Thin agent chat: start a server-owned session, subscribe to session SSE,
/// deliver stdin messages via the conversations endpoint — no host transcript
/// or compaction (constitution #2).
async fn run_agent_chat(opts: &ChatOptions, agent_id: &str) -> Result<()> {
    let base = resolve_server_base(opts.server.as_deref())?;
    let client = client_for_sse(&base);

    let started = client
        .create_agent_session(
            agent_id,
            &crate::client::execute::CreateAgentSessionRequest {
                system_prompt: None,
                backend: opts.backend.clone(),
                model: opts.model.clone(),
                session_id: opts.session_id.clone(),
            },
        )
        .await
        .context("agent session start failed")?;
    let session_id = started.session_id;
    eprintln!(
        "apxm chat — agent {agent_id} session {session_id} @ {}",
        client.baseurl()
    );
    eprintln!("type a message, or /session /budget /exit; events via session SSE");

    let stream_url = if started.stream_url.starts_with("http") {
        started.stream_url
    } else {
        format!("{}{}", client.baseurl(), started.stream_url)
    };
    let client_c = client.clone();
    let render = tokio::spawn(render_session_events_from_url(client_c, stream_url));

    use tokio::io::AsyncBufReadExt as _;
    let mut stdin_lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        eprint!("\nuser> ");
        let _ = std::io::stderr().flush();
        let line = match stdin_lines.next_line().await.context("stdin read failed")? {
            Some(l) => l.trim().to_string(),
            None => break,
        };
        if line.is_empty() {
            continue;
        }
        if matches!(line.as_str(), "/exit" | "/quit") {
            break;
        }
        if let Some(meta) = line.strip_prefix('/') {
            match meta.split(' ').next().unwrap_or(meta) {
                "session" => {
                    eprintln!("session_id: {session_id}");
                    match client.get_session_status(&session_id).await {
                        Ok(resp) => print_session_status(&resp.into_inner()),
                        Err(err) => eprintln!("(session status failed: {err})"),
                    }
                    continue;
                }
                "help" => {
                    eprintln!("meta-commands: /session /exit");
                    continue;
                }
                other => {
                    eprintln!("unknown meta-command /{other} (try /help /exit)");
                    continue;
                }
            }
        }
        let r = client.post_conversation_message(&session_id, &line).await?;
        if !r.status().is_success() {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            eprintln!("message delivery failed: {status}: {text}");
        }
    }

    render.abort();
    Ok(())
}

async fn render_session_events_from_url(client: Client, url: String) {
    let resp = match client
        .client()
        .get(&url)
        .header("Accept", "text/event-stream")
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp,
        Ok(resp) => {
            eprintln!(
                "session events stream failed: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
            return;
        }
        Err(err) => {
            eprintln!("session events stream failed: {err}");
            return;
        }
    };
    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::default();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else { break };
        for frame in parser.feed(&chunk) {
            let Ok(v) = serde_json::from_str::<JsonValue>(&frame.data) else {
                continue;
            };
            let _ = maybe_answer_permission(&client, &v).await;
            if let Some(render) = render_chat_frame(&v) {
                apply_chat_frame_render(render);
            }
        }
    }
}

/// Entry point dispatched from `main.rs`.
pub async fn chat_command(opts: ChatOptions) -> Result<()> {
    if let Some(agent_id) = opts
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return run_agent_chat(&opts, agent_id).await;
    }

    let air_path = opts.air.as_ref().ok_or_else(|| {
        anyhow!(
            "apxm chat requires --agent <id> for server-backed chat or \
             --air <path> for a custom in-graph artifact"
        )
    })?;

    let air = std::fs::read_to_string(air_path)
        .with_context(|| format!("failed to read AIR graph {}", air_path.display()))?;

    if !artifact_has_await_event(&air) {
        return Err(anyhow!(
            "custom --air artifact must declare an explicit await.event operation"
        ));
    }

    let base = resolve_server_base(opts.server.as_deref())?;
    let session_id = opts.session_id.clone().unwrap_or_else(mint_session_id);
    let client = client_for_sse(&base);

    eprintln!(
        "apxm chat — in-program loop detected; host is a dumb pipe (session {session_id} @ {})",
        client.baseurl()
    );
    eprintln!("type a message; /exit to quit");

    run_dumb_pipe(&client, &session_id, &air, &opts).await
}

fn print_session_status(status: &SessionStatus) {
    match &status.active_execution_id {
        Some(execution_id) => eprintln!("  active execution: {execution_id}"),
        None => eprintln!("  no active execution"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_has_await_event_detects_canonical_json() {
        let air = r#"{"schema_version":"apxm.air.v2","semantic_operations":[{"node_id":"n1","op":"await.event"}],"structural_ir":[],"source_map":{"schema_version":"apxm.source-map.v1","source_language":"json","node_spans":[],"region_annotations":[]}}"#;
        assert!(artifact_has_await_event(air));
    }

    #[test]
    fn artifact_has_await_event_rejects_noncanonical_source() {
        assert!(!artifact_has_await_event("ais.await_input"));
    }

    /// Positive: `token` still renders as raw text, matching the pre-fix
    /// behavior exactly (regression pin).
    #[test]
    fn chat_renderer_still_prints_token_text() {
        let frame = serde_json::json!({"payload": {"kind": "token", "text": "hi"}});
        assert_eq!(
            render_chat_frame(&frame),
            Some(ChatFrameRender::Token("hi".to_string()))
        );
    }

    /// Positive: `error` still renders its message (regression pin).
    #[test]
    fn chat_renderer_still_prints_error_message() {
        let frame = serde_json::json!({"payload": {"kind": "error", "message": "boom"}});
        assert_eq!(
            render_chat_frame(&frame),
            Some(ChatFrameRender::Error("boom".to_string()))
        );
    }

    /// `chat_renderer_surfaces_unknown_kind_instead_of_dropping` — a frame
    /// the REPL has no dedicated projection for produces visible output
    /// instead of being parsed and silently discarded.
    #[test]
    fn chat_renderer_surfaces_unknown_kind_instead_of_dropping() {
        let frame = serde_json::json!({
            "payload": {
                "kind": "execution_started",
                "execution_id": "exec-1",
            }
        });
        let render = render_chat_frame(&frame);
        assert!(
            render.is_some(),
            "an execution_started frame must render something, not nothing"
        );
        assert_eq!(
            render,
            Some(ChatFrameRender::Notice("[execution_started]".to_string()))
        );
    }
}
