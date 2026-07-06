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
    /// Pin each turn to a registered backend (package chat only).
    pub backend: Option<String>,
    /// Pin each turn to a specific model id (package chat only).
    pub model: Option<String>,
    /// Tenant/owner scope for per-tool credential resolution.
    pub owner: Option<String>,
    /// Agent package id for thin server-backed chat
    /// (`POST /v1/agents/packages/{{id}}/sessions`).
    pub package: Option<String>,
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

/// True when the artifact carries its OWN in-graph conversation loop: a
/// re-arming `recv` anchor (AUTONOMOUS `mode = "recv"`, `recv_once = "false"`),
/// the signature a `ConversationalAgent(loop="in_graph")` emits. Such artifacts
/// own the loop, so the host is a dumb pipe (constitution #2).
fn air_has_in_program_loop(air: &str) -> bool {
    air.contains("mode = \"recv\"") && air.contains("recv_once = \"false\"")
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
            let kind = v.pointer("/payload/kind").and_then(|k| k.as_str());
            if kind == Some("token")
                && let Some(tok) = v.pointer("/payload/text").and_then(|t| t.as_str())
            {
                print!("{tok}");
                let _ = std::io::stdout().flush();
            } else if kind == Some("error")
                && let Some(m) = v.pointer("/payload/message").and_then(|m| m.as_str())
            {
                eprintln!("\n[error] {m}");
            }
        }
    }
}

/// Thin package chat: start a server-owned session, subscribe to session SSE,
/// deliver stdin turns via the conversations endpoint — no host transcript or
/// compaction (constitution #2).
async fn run_package_chat(opts: &ChatOptions, package_id: &str) -> Result<()> {
    let base = opts
        .server
        .clone()
        .or_else(|| std::env::var("APXM_SERVER_BASE").ok())
        .unwrap_or_else(|| DEFAULT_SERVER_BASE.to_string());
    let client = client_for_sse(&base);

    let started = client
        .create_package_session(
            package_id,
            &crate::client::execute::CreatePackageSessionRequest {
                system_prompt: None,
                backend: opts.backend.clone(),
                model: opts.model.clone(),
                session_id: opts.session_id.clone(),
            },
        )
        .await
        .context("package session start failed")?;
    let session_id = started.session_id;
    eprintln!(
        "apxm chat — package {package_id} session {session_id} @ {}",
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
            let kind = v.pointer("/payload/kind").and_then(|k| k.as_str());
            if kind == Some("token")
                && let Some(tok) = v.pointer("/payload/text").and_then(|t| t.as_str())
            {
                print!("{tok}");
                let _ = std::io::stdout().flush();
            } else if kind == Some("error")
                && let Some(m) = v.pointer("/payload/message").and_then(|m| m.as_str())
            {
                eprintln!("\n[error] {m}");
            }
        }
    }
}

/// Entry point dispatched from `main.rs`.
pub async fn chat_command(opts: ChatOptions) -> Result<()> {
    if let Some(package_id) = opts
        .package
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return run_package_chat(&opts, package_id).await;
    }

    let air_path = opts.air.as_ref().ok_or_else(|| {
        anyhow!(
            "apxm chat requires --package <id> for server-backed chat or \
             --air <path> for a custom in-graph artifact"
        )
    })?;

    let air = std::fs::read_to_string(air_path)
        .with_context(|| format!("failed to read AIR graph {}", air_path.display()))?;

    if !air_has_in_program_loop(&air) {
        return Err(anyhow!(
            "custom --air artifact must declare an in-program recv loop \
             (mode = \"recv\", recv_once = \"false\")"
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
    fn air_has_in_program_loop_detects_recv_anchor() {
        let air = r#"
            %loop = ais.autonomous "" {mode = "recv", recv_once = "false"} (%arg0 : !ais.token) : !ais.token
        "#;
        assert!(air_has_in_program_loop(air));
    }

    #[test]
    fn air_has_in_program_loop_rejects_host_turn_flow() {
        let air = r#"
            %run_turn = ais.flow_call "conversation" "turn" {} (%arg0 : !ais.token) : !ais.token
        "#;
        assert!(!air_has_in_program_loop(air));
    }
}
