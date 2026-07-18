//! `apxm chat` — interactive conversational REPL over a running apxm-server.
//!
//! Thin protocol pipe: deliver user input, render SSE events, and answer server
//! permission prompts. Session ledger turn caps and tool budgets are enforced
//! server-side — the host does not count turns or track budgets locally
//! (constitution #2).

use std::io::Write as _;
use std::path::PathBuf;

use crate::client::{
    Client, ClientInfo, DEFAULT_SERVER_BASE, client_for_sse, execute::ExecuteRequest,
    types::SessionStatus,
};
use anyhow::{Context, Result, anyhow};
use apxm_ais::{AISOperationType, attrs::MLIR_ATTR_PREFIX};
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
    /// Pin each turn to a registered backend (agent chat only).
    pub backend: Option<String>,
    /// Pin each turn to a specific model id (agent chat only).
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

/// True when the package owns a typed host-input suspension point. The CLI
/// treats this only as artifact validation; it does not infer any Agent Skill
/// or conversational semantics from the op.
fn air_has_await_input(air: &str) -> bool {
    let operation = format!(
        "{MLIR_ATTR_PREFIX}{}",
        AISOperationType::AwaitInput.mlir_mnemonic()
    );
    air.contains(&operation)
}

/// Dumb-pipe host (constitution #2): POST the artifact ONCE, pipe stdin lines to
/// the turn-input endpoint, render streamed tokens. No transcript, no
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
            eprintln!("turn-input failed: {status}: {text}");
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
/// deliver stdin turns via the conversations endpoint — no host transcript or
/// compaction (constitution #2).
async fn run_agent_chat(opts: &ChatOptions, agent_id: &str) -> Result<()> {
    let base = opts
        .server
        .clone()
        .or_else(|| std::env::var("APXM_SERVER_BASE").ok())
        .unwrap_or_else(|| DEFAULT_SERVER_BASE.to_string());
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
                    if let Ok(resp) = client.get_session_status(&session_id).await {
                        print_server_budget(&resp.into_inner());
                    }
                    continue;
                }
                "budget" => match client.get_session_status(&session_id).await {
                    Ok(resp) => print_server_budget(&resp.into_inner()),
                    Err(err) => eprintln!("(session status failed: {err})"),
                },
                "help" => {
                    eprintln!("meta-commands: /session /budget /exit");
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
            eprintln!("turn-input failed: {status}: {text}");
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

    if !air_has_await_input(&air) {
        return Err(anyhow!(
            "custom --air artifact must declare an explicit AWAIT_INPUT node"
        ));
    }

    let base = opts
        .server
        .clone()
        .or_else(|| std::env::var("APXM_SERVER_BASE").ok())
        .unwrap_or_else(|| DEFAULT_SERVER_BASE.to_string());
    let session_id = opts.session_id.clone().unwrap_or_else(mint_session_id);
    let client = client_for_sse(&base);

    eprintln!(
        "apxm chat — in-program loop detected; host is a dumb pipe (session {session_id} @ {})",
        client.baseurl()
    );
    eprintln!("type a message; /exit to quit");

    run_dumb_pipe(&client, &session_id, &air, &opts).await
}

fn print_server_budget(status: &SessionStatus) {
    let ledger = &status.ledger;
    if ledger.turn_cap.is_none() && ledger.tool_budgets.is_empty() {
        eprintln!("(no session budgets set — server ledger empty)");
        return;
    }
    if let Some(cap) = ledger.turn_cap {
        eprintln!("  turns: {}/{} used", status.turn_count, cap);
    }
    if ledger.tool_budgets.is_empty() {
        return;
    }
    let mut names: Vec<_> = ledger.tool_budgets.keys().collect();
    names.sort();
    for name in names {
        let budget = ledger.tool_budgets[name];
        eprintln!("  {name}: session budget {budget} (server-tracked)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_has_await_input_detects_explicit_input_node() {
        let air = format!(
            "%input = {MLIR_ATTR_PREFIX}{} \"\" {{wait_key = \"session\"}} : !ais.token",
            AISOperationType::AwaitInput.mlir_mnemonic()
        );
        assert!(air_has_await_input(&air));
    }

    #[test]
    fn air_has_await_input_rejects_host_turn_flow() {
        let air = format!(
            "%run_turn = {MLIR_ATTR_PREFIX}{} \"conversation\" \"turn\" {{}} (%arg0 : !ais.token) : !ais.token",
            AISOperationType::FlowCall.mlir_mnemonic()
        );
        assert!(!air_has_await_input(&air));
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

    /// `chat_renderer_surfaces_unknown_kind_instead_of_dropping` — a
    /// `turn_started` frame (today: nothing) now produces visible output
    /// instead of being parsed and silently discarded.
    #[test]
    fn chat_renderer_surfaces_unknown_kind_instead_of_dropping() {
        let frame = serde_json::json!({
            "payload": {
                "kind": "turn_started",
                "execution_id": "exec-1",
            }
        });
        let render = render_chat_frame(&frame);
        assert!(
            render.is_some(),
            "a turn_started frame must render something, not nothing"
        );
        assert_eq!(
            render,
            Some(ChatFrameRender::Notice("[turn_started]".to_string()))
        );
    }
}
