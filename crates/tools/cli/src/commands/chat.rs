//! `apxm chat` — interactive conversational REPL over a running apxm-server.
//!
//! Thin protocol pipe: deliver user input, render SSE events, and answer server
//! permission prompts. Session ledger turn caps and tool budgets are enforced
//! server-side — the host does not count turns or track budgets locally
//! (constitution #2).

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use apxm_ais::chat::{self, COMPACT_AT_TOKENS, KEEP_RECENT_TURNS, Role};
use crate::client::reqwest;
use crate::client::{
    Client, ClientInfo, DEFAULT_SERVER_BASE, client_for_sse, execute::ExecuteRequest,
    types::SessionStatus,
};
use apxm_core::constants::orchestration::admission as orchestration_admission;
use futures::StreamExt;
use serde_json::Value as JsonValue;

use super::render::{RunSnapshot, render_tree};
use super::sse_permissions::maybe_answer_permission;
use super::watch::{SseParser, decode_event_frame};

/// CLI options for the chat REPL.
#[derive(Debug, Clone)]
pub struct ChatOptions {
    pub air: Option<PathBuf>,
    pub server: Option<String>,
    pub session_id: Option<String>,
    pub capability_grant_ids: Vec<String>,
    /// Skill libraries / ids this agent imports (scoped visible set).
    pub import: Vec<String>,
    pub tree: bool,
    /// Enable the agent's `web` tool group each turn (ignored when `--air` is set).
    pub tools: bool,
    /// Pin each turn to a registered backend (ignored when `--air` is set).
    pub backend: Option<String>,
    /// Pin each turn to a specific model id (ignored when `--air` is set).
    pub model: Option<String>,
    /// Spawn this ACP profile for each turn instead of using a direct ASK node.
    pub agent: Option<String>,
    /// Optional ACP mode for `agent`.
    pub agent_mode: Option<String>,
    /// Optional ACP model request for `agent`.
    pub agent_model: Option<String>,
    /// Subscribe to an apxm-os control-plane event stream
    /// (`GET {monitor_url}/events`); each cue event becomes a synthetic turn, so
    /// the agent reacts to external events (file changes, process output, cron,
    /// webhooks) without a human typing. The REPL stays interactive alongside it.
    pub monitor_url: Option<String>,
    /// Maximum substantive turns — forwarded to the server session ledger on the
    /// first execute; enforcement is server-side (no host counting).
    pub max_turns: Option<usize>,
    /// Maximum event-driven turns — reserved for server-side policy; the host
    /// does not drop events based on a local counter.
    pub max_events: Option<usize>,
    /// Per-tool call budget for one turn, as `CAP=N`. Sent as `tool_call_budgets`;
    /// the server/runtime enforces across the execution tree.
    pub tool_budget: Vec<String>,
    /// Per-tool session call cap as `CAP=N`. Merged into the first execute's
    /// `tool_call_budgets` so the server session ledger owns consumption.
    pub tool_cap: Vec<String>,
    /// Per-tool auth binding as raw `CAP=CONNECTION_ID` strings. The
    /// server resolves each to a bearer token (scoped to `owner`) and the runtime
    /// injects it at the tool's invoke seam; only the connection id is sent.
    pub tool_auth: Vec<String>,
    /// Tenant/owner scope for per-tool credential resolution.
    pub owner: Option<String>,
    /// Expose the workflow-authoring tools (`compose_workflow`, `run_workflow`).
    pub author: bool,
}

/// Client-side conversation transcript. The runtime carries no role-tagged
/// message history across turns, so the REPL owns it and threads it back as the
/// `conversation` arg each turn.
///
/// To stay within the model context window, the transcript is compacted as a
/// **post-hook of the host loop**: after each turn, if the rendered transcript
/// exceeds [`COMPACT_AT_TOKENS`], the oldest turns are folded into a running
/// `summary` (cumulative — the prior summary is fed back in) while the most
/// recent [`KEEP_RECENT_TURNS`] stay verbatim.
#[derive(Default)]
pub(crate) struct Conversation {
    /// Running summary of folded-away (compacted) turns. Empty until first compaction.
    summary: String,
    turns: Vec<(String, String)>,
}

impl Conversation {
    /// Render the running summary (if any) + the verbatim turns + the pending
    /// user line via the shared [`chat::render_transcript`], leaving a final
    /// `Assistant:` for the model to complete. The summary rides a `System` turn
    /// ("Summary of earlier conversation: …") — the same framing the studio uses.
    pub(crate) fn render(&self, next_user: &str) -> String {
        let summary_line;
        let mut msgs: Vec<(Role, &str)> = Vec::new();
        if !self.summary.is_empty() {
            summary_line = format!("Summary of earlier conversation: {}", self.summary);
            msgs.push((Role::System, summary_line.as_str()));
        }
        for (user, assistant) in &self.turns {
            msgs.push((Role::User, user));
            msgs.push((Role::Assistant, assistant));
        }
        msgs.push((Role::User, next_user));
        chat::render_transcript(msgs)
    }

    pub(crate) fn record(&mut self, user: String, assistant: String) {
        self.turns.push((user, assistant));
    }

    /// The most recent assistant reply, if any.
    pub(crate) fn last_assistant(&self) -> Option<&str> {
        self.turns.last().map(|(_, assistant)| assistant.as_str())
    }

    /// Rough token estimate of the rendered transcript (shared chars/4 heuristic).
    pub(crate) fn token_estimate(&self) -> usize {
        chat::estimate_tokens(&self.render(""))
    }

    /// The text of the oldest turns that would be folded by a compaction.
    fn foldable_text(&self) -> Option<String> {
        if self.turns.len() <= KEEP_RECENT_TURNS {
            return None;
        }
        let fold = self.turns.len() - KEEP_RECENT_TURNS;
        let mut text = String::new();
        if !self.summary.is_empty() {
            text.push_str("Prior summary: ");
            text.push_str(&self.summary);
            text.push_str("\n\n");
        }
        for (user, assistant) in &self.turns[..fold] {
            text.push_str("User: ");
            text.push_str(user);
            text.push_str("\nAssistant: ");
            text.push_str(assistant);
            text.push('\n');
        }
        Some(text)
    }

    /// Replace the folded turns with the new running summary (keeps recent K).
    fn apply_compaction(&mut self, new_summary: String) {
        let fold = self.turns.len().saturating_sub(KEEP_RECENT_TURNS);
        self.turns.drain(..fold);
        self.summary = new_summary;
    }
}

/// Mint a unique session id without pulling in a uuid dependency.
fn mint_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("chat-{:x}", nanos)
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn chat_air_for_options(opts: &ChatOptions, context: Option<&str>) -> Result<String> {
    match &opts.air {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("failed to read AIR graph {}", path.display())),
        None => Ok(builtin_chat_air(opts, context)),
    }
}

fn builtin_chat_air(opts: &ChatOptions, context: Option<&str>) -> String {
    match opts.agent.as_deref().and_then(non_empty) {
        Some(profile) => chat::acp_chat_air(&chat::ChatAcpAirOptions {
            profile,
            system_prompt: context,
            mode: opts.agent_mode.as_deref().and_then(non_empty),
            model: opts.agent_model.as_deref().and_then(non_empty),
        }),
        None => chat::chat_air(&chat::ChatAirOptions {
            system_prompt: context,
            backend: opts.backend.as_deref(),
            model: opts.model.as_deref(),
            effort: None,
            tools: opts.tools,
            capability_discovery: true,
            skills: true,
            authoring: opts.author,
        }),
    }
}

fn required_capability_bindings(opts: &ChatOptions) -> Vec<String> {
    let mut bindings = opts
        .capability_grant_ids
        .iter()
        .filter(|grant| !grant.starts_with("grant_"))
        .cloned()
        .collect::<Vec<_>>();
    if opts.agent.as_deref().and_then(non_empty).is_some()
        && !bindings
            .iter()
            .any(|capability| capability == orchestration_admission::SPAWN_AGENT)
    {
        bindings.push(orchestration_admission::SPAWN_AGENT.to_string());
    }
    if opts.author {
        for cap in ["compose_workflow", "run_workflow"] {
            if !bindings.iter().any(|g| g == cap) {
                bindings.push(cap.to_string());
            }
        }
    }
    bindings
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
    let capability_grant_ids = resolve_execute_capability_grant_ids(&client, opts).await?;
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

async fn render_session_stream(resp: reqwest::Response, client: Client) {
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
    let base = opts
        .server
        .clone()
        .or_else(|| std::env::var("APXM_SERVER_BASE").ok())
        .unwrap_or_else(|| DEFAULT_SERVER_BASE.to_string());
    let session_id = opts.session_id.clone().unwrap_or_else(mint_session_id);
    // `--air` drives a custom graph; otherwise build the built-in chat graph.
    // Assemble the AGENTS.md / CLAUDE.md hierarchy and feed it as the system prompt.
    let context = {
        let base = crate::context_assembly::assemble_context();
        let mut hints = vec![capability_discovery_prompt().to_string()];
        if !opts.import.is_empty() {
            hints.push(format!(
                "Imported skill libraries: {}. Call search_skills with these in `imports` to discover them; shared-tier skills are always visible.",
                opts.import.join(", ")
            ));
        }
        append_context_hints(base, &hints)
    };
    let air = chat_air_for_options(&opts, context.as_deref())?;
    if let Some(ctx) = &context {
        eprintln!(
            "context: loaded AGENTS.md/CLAUDE.md hierarchy (~{} tokens)",
            ctx.len() / 4
        );
    }

    let client = client_for_sse(&base);

    if air_has_in_program_loop(&air) {
        eprintln!(
            "apxm chat — in-program loop detected; host is a dumb pipe (session {session_id} @ {})",
            client.baseurl()
        );
        eprintln!("type a message; /exit to quit");
        return run_dumb_pipe(&client, &session_id, &air, &opts).await;
    }

    eprintln!("apxm chat — session {session_id} @ {}", client.baseurl());
    eprintln!("type a message, or /help for meta-commands; /exit to quit");

    let mut convo = Conversation::default();
    let mut approved_capability_bindings: Vec<String> = opts
        .capability_grant_ids
        .iter()
        .filter(|grant| !grant.starts_with("grant_"))
        .cloned()
        .collect();
    for binding in required_capability_bindings(&opts) {
        if !approved_capability_bindings
            .iter()
            .any(|existing| existing == &binding)
        {
            approved_capability_bindings.push(binding);
        }
    }
    let tool_call_budgets = tool_call_budgets_for_opts(&opts);

    let mut event_rx = match &opts.monitor_url {
        Some(url) => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            spawn_monitor_subscriber(client.client().clone(), url.clone(), tx);
            eprintln!("monitor: subscribed to {url}/events — external events become turns");
            Some(rx)
        }
        None => None,
    };

    use tokio::io::AsyncBufReadExt as _;
    let mut stdin_lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();

    if opts.max_turns.is_some() || opts.max_events.is_some() {
        eprintln!(
            "(turn/event limits are enforced by the server session ledger — not counted host-side)"
        );
    }
    if !tool_call_budgets.is_empty() {
        eprintln!("tool budgets (server-enforced): {tool_call_budgets:?}");
    }

    loop {
        eprint!("\nuser> ");
        let _ = std::io::stderr().flush();

        // Whichever arrives first: a typed line or an external event. An event
        // turn skips meta-command handling (its text is JSON, never a `/cmd`).
        enum Source {
            Stdin(Option<String>),
            Event(String),
        }
        let source = tokio::select! {
            line = stdin_lines.next_line() => Source::Stdin(line.context("stdin read failed")?),
            Some(cue) = recv_opt(&mut event_rx) => Source::Event(cue),
        };

        let (line, from_event) = match source {
            Source::Stdin(None) => {
                // EOF (Ctrl-D): clean exit.
                eprintln!();
                break;
            }
            Source::Stdin(Some(l)) => (l.trim().to_string(), false),
            Source::Event(cue) => {
                eprintln!("\n[event] {cue}");
                (cue, true)
            }
        };
        if line.is_empty() {
            continue;
        }
        if !from_event {
            if let Some(meta) = line.strip_prefix('/') {
                if meta.split(' ').next() == Some("compact") {
                    match compact_if_needed(&mut convo, &client, &session_id, true).await {
                        Ok(true) => {}
                        Ok(false) => eprintln!("(nothing to compact yet)"),
                        Err(err) => eprintln!("(compaction failed: {err})"),
                    }
                    continue;
                }
                let (verb, rest) = meta
                    .split_once(' ')
                    .map(|(v, r)| (v, r.trim()))
                    .unwrap_or((meta, ""));
                match verb {
                    "budget" => {
                        match client.get_session_status(&session_id).await {
                            Ok(resp) => print_server_budget(&resp.into_inner()),
                            Err(err) => eprintln!("(session status failed: {err})"),
                        }
                        continue;
                    }
                    // Persist the agent's last reply to disk only when the
                    // operator asks; the agent stays read-only by default.
                    "save" => {
                        if rest.is_empty() {
                            eprintln!("(usage: /save <path>)");
                        } else {
                            match convo.last_assistant() {
                                Some(text) => match std::fs::write(rest, text) {
                                    Ok(()) => eprintln!("(saved last reply to {rest})"),
                                    Err(e) => eprintln!("(save failed: {e})"),
                                },
                                None => eprintln!("(nothing to save yet)"),
                            }
                        }
                        continue;
                    }
                    // Run or list authored workflows only after an operator command.
                    "workflow" => {
                        handle_workflow_meta(rest, &client, &session_id).await;
                        continue;
                    }
                    _ => {}
                }
                match handle_meta(meta, &client).await {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(err) => eprintln!("(meta-command error: {err})"),
                }
                continue;
            }
        }

        handle_user_turn(
            &line,
            &mut convo,
            &client,
            &air,
            &session_id,
            &opts,
            &mut approved_capability_bindings,
            &tool_call_budgets,
        )
        .await;
    }
    Ok(())
}

fn capability_discovery_prompt() -> &'static str {
    "Runtime capability discovery: call `capability_discovery` when you need to inspect available capability templates. Treat results as CapabilityTemplateV1 authoring metadata only; they are not authority. Do not invent `capability_id` values. Present writes only when capability_grant_ids are supplied by APXM."
}

fn append_context_hints(base: Option<String>, hints: &[String]) -> Option<String> {
    let hint = hints
        .iter()
        .map(String::as_str)
        .filter(|item| !item.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if hint.is_empty() {
        return base;
    }
    Some(match base {
        Some(base) if !base.trim().is_empty() => format!("{base}\n\n{hint}"),
        _ => hint,
    })
}

/// Merge per-turn `--tool-budget` and session `--tool-cap` into the wire map
/// sent to the server. Consumption is tracked server-side (SessionLedger).
fn tool_call_budgets_for_opts(opts: &ChatOptions) -> HashMap<String, usize> {
    let turn = parse_kv_usize(&opts.tool_budget);
    let cap = parse_kv_usize(&opts.tool_cap);
    let mut out = HashMap::new();
    for name in turn.keys().chain(cap.keys()) {
        if out.contains_key(name) {
            continue;
        }
        let per_turn = turn.get(name).copied();
        let session = cap.get(name).copied();
        match (per_turn, session) {
            (Some(a), Some(b)) => {
                out.insert(name.clone(), a.min(b));
            }
            (Some(a), None) => {
                out.insert(name.clone(), a);
            }
            (None, Some(b)) => {
                out.insert(name.clone(), b);
            }
            (None, None) => {}
        }
    }
    out
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

/// Parse repeatable `CAP=N` strings into a per-tool map, warning on malformed
/// entries rather than failing the session.
fn parse_kv_usize(items: &[String]) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for item in items {
        match item.split_once('=') {
            Some((k, v)) => match v.trim().parse::<usize>() {
                Ok(n) => {
                    map.insert(k.trim().to_string(), n);
                }
                Err(_) => eprintln!("(ignoring '{item}': budget value must be a number)"),
            },
            None => eprintln!("(ignoring '{item}': expected CAP=N)"),
        }
    }
    map
}

/// Parse repeatable `KEY=VALUE` strings into a string map, warning on malformed
/// entries rather than failing the session.
fn parse_kv_string(items: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for item in items {
        match item.split_once('=') {
            Some((k, v)) if !k.trim().is_empty() && !v.trim().is_empty() => {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
            _ => eprintln!("(ignoring '{item}': expected KEY=VALUE)"),
        }
    }
    map
}

/// Await the next event, or pend forever when no monitor is attached — so an
/// absent event source simply never wins the `select!`.
async fn recv_opt(rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<String>>) -> Option<String> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

/// Run one turn for `user_text` (typed or event-driven), including the
/// interactive HITL retry loop. Errors are reported, never fatal — the
/// REPL keeps serving.
#[allow(clippy::too_many_arguments)]
async fn handle_user_turn(
    user_text: &str,
    convo: &mut Conversation,
    client: &Client,
    air: &str,
    session_id: &str,
    opts: &ChatOptions,
    approved_capability_bindings: &mut Vec<String>,
    tool_call_budgets: &HashMap<String, usize>,
) {
    let prompt = convo.render(user_text);
    loop {
        let capability_grant_ids = match client
            .resolve_capability_grant_ids(&approved_capability_bindings)
            .await
        {
            Ok(ids) => ids,
            Err(err) => {
                eprintln!("(could not mint capability grants: {err})");
                break;
            }
        };
        match run_turn(
            client,
            air,
            session_id,
            &prompt,
            user_text,
            &capability_grant_ids,
            opts,
            tool_call_budgets,
        )
        .await
        {
            Ok(TurnOutcome::Answered { answer }) => {
                record_and_compact(convo, user_text, answer, client, session_id).await;
                break;
            }
            Ok(TurnOutcome::NeedsGrant(cap)) => {
                eprint!(
                    "(capability '{cap}' requires a capability grant — grant for this session? [y/N]) "
                );
                let _ = std::io::stderr().flush();
                let mut line = String::new();
                if std::io::stdin().read_line(&mut line).is_err() {
                    break;
                }
                if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    eprintln!("(turn skipped — write capability '{cap}' was not granted)");
                    break;
                }
                if !approved_capability_bindings.iter().any(|binding| binding == &cap) {
                    approved_capability_bindings.push(cap);
                }
            }
            Err(err) => {
                eprintln!("(turn failed: {err})");
                break;
            }
        }
    }
}

/// Subscribe to an apxm-os control-plane `GET {os}/events` SSE stream and forward
/// each cue event as a turn string. Reconnects with a fixed backoff so a brief
/// os outage doesn't end the subscription.
fn spawn_monitor_subscriber(
    client: reqwest::Client,
    os_base: String,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) {
    let url = format!("{}/events", os_base.trim_end_matches('/'));
    tokio::spawn(async move {
        loop {
            match client
                .get(&url)
                .header("accept", "text/event-stream")
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    let mut stream = resp.bytes_stream();
                    let mut parser = SseParser::default();
                    while let Some(chunk) = stream.next().await {
                        let Ok(bytes) = chunk else { break };
                        for frame in parser.feed(&bytes) {
                            if frame.data.is_empty() {
                                continue;
                            }
                            if tx.send(cue_event_to_turn(&frame.data)).is_err() {
                                return; // REPL gone
                            }
                        }
                    }
                }
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
}

/// Render a CueEvent JSON payload as a compact, model-friendly turn. Falls back
/// to the raw JSON if it doesn't parse.
fn cue_event_to_turn(data: &str) -> String {
    match serde_json::from_str::<JsonValue>(data) {
        Ok(ev) => {
            let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("event");
            let cue = ev
                .get("cue_id")
                .or_else(|| ev.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let payload = ev.get("payload").cloned().unwrap_or(JsonValue::Null);
            format!(
                "An apxm-os event fired (kind={kind}{}). Payload: {}. React appropriately for this agent.",
                if cue.is_empty() {
                    String::new()
                } else {
                    format!(", cue={cue}")
                },
                payload
            )
        }
        Err(_) => format!("An apxm-os event fired: {data}"),
    }
}

/// Outcome of one conversational turn.
enum TurnOutcome {
    Answered { answer: String },
    NeedsGrant(String),
}

/// Record a completed turn, then run compaction (the host loop's post-hook):
/// if the transcript is over budget, fold the oldest turns into the running
/// summary so future turns stay within the model context window.
async fn record_and_compact(
    convo: &mut Conversation,
    user: &str,
    answer: String,
    client: &Client,
    session_id: &str,
) {
    convo.record(user.to_string(), answer);
    if convo.token_estimate() > COMPACT_AT_TOKENS {
        if let Err(err) = compact_if_needed(convo, client, session_id, false).await {
            eprintln!("(compaction skipped: {err})");
        }
    }
}

async fn compact_if_needed(
    convo: &mut Conversation,
    client: &Client,
    session_id: &str,
    force: bool,
) -> Result<bool> {
    let Some(foldable) = convo.foldable_text() else {
        return Ok(false); // nothing old enough to fold
    };
    if !force && convo.token_estimate() <= COMPACT_AT_TOKENS {
        return Ok(false);
    }
    let before = convo.token_estimate();
    let new_summary = summarize_quiet(client, session_id, &foldable).await?;
    if new_summary.trim().is_empty() {
        return Ok(false); // summarizer returned nothing — keep transcript intact
    }
    convo.apply_compaction(new_summary);
    eprintln!(
        "(compacted: ~{before} -> ~{} tokens, {} recent turns kept)",
        convo.token_estimate(),
        convo.turns.len()
    );
    Ok(true)
}

/// Run the built-in summarize graph for `text` and return the summary, without
/// printing anything (a quiet, non-streaming turn). Reuses the same session id
/// so the summarization is attributed to this conversation.
async fn summarize_quiet(client: &Client, session_id: &str, text: &str) -> Result<String> {
    let body = ExecuteRequest {
        air: chat::SUMMARIZE_AIR.to_string(),
        args: vec![text.to_string()],
        session_id: Some(session_id.to_string()),
        ..Default::default()
    };
    let resp = client.execute_stream(&body).await?;
    anyhow::ensure!(
        resp.status().is_success(),
        "summarize returned {} for {}",
        resp.status(),
        client.baseurl()
    );
    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::default();
    let mut summary = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("SSE chunk read failed")?;
        for frame in parser.feed(&chunk) {
            if let Ok(v) = serde_json::from_str::<JsonValue>(&frame.data) {
                let _ = maybe_answer_permission(client, &v).await;
                if let Some(c) = v
                    .pointer("/payload/result/content")
                    .and_then(|c| c.as_str())
                {
                    summary = c.to_string();
                }
            }
        }
    }
    Ok(summary)
}

/// Handle `/workflow <sub>` meta-commands: `list` enumerates `.air` / `.apxmw`
/// files in the cwd; `run <path>` reads the file client-side and runs it through
/// `/v1/compile/stream`.
async fn handle_workflow_meta(rest: &str, client: &Client, session_id: &str) {
    let (sub, arg) = rest
        .split_once(' ')
        .map(|(s, a)| (s, a.trim()))
        .unwrap_or((rest, ""));
    match sub {
        "list" | "" => match std::fs::read_dir(".") {
            Ok(entries) => {
                let mut found = false;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if matches!(
                        path.extension().and_then(|e| e.to_str()),
                        Some("air") | Some("apxmw")
                    ) {
                        eprintln!("  {}", path.display());
                        found = true;
                    }
                }
                if !found {
                    eprintln!("(no .air/.apxmw files in cwd)");
                }
            }
            Err(e) => eprintln!("(cannot list cwd: {e})"),
        },
        "run" => {
            if arg.is_empty() {
                eprintln!("(usage: /workflow run <path>)");
                return;
            }
            match std::fs::read_to_string(arg) {
                Ok(air) => {
                    if let Err(e) = run_workflow_air(client, session_id, &air).await {
                        eprintln!("(workflow run failed: {e})");
                    }
                }
                Err(e) => eprintln!("(cannot read {arg}: {e})"),
            }
        }
        other => eprintln!("(unknown /workflow subcommand '{other}'; try list|run)"),
    }
}

/// Run an authored workflow's AIR through `/v1/compile/stream`, printing the
/// final content.
async fn run_workflow_air(client: &Client, session_id: &str, air: &str) -> Result<()> {
    let url = format!("{}/v1/compile/stream", client.baseurl());
    let body = serde_json::json!({
        "air": air,
        "session_id": session_id,
    });
    let resp = client
        .client()
        .post(&url)
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .with_context(|| format!("failed to POST {url}"))?;
    anyhow::ensure!(
        resp.status().is_success(),
        "server returned {} for {url}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("SSE chunk read failed")?;
        for frame in parser.feed(&chunk) {
            if let Ok(v) = serde_json::from_str::<JsonValue>(&frame.data) {
                let _ = maybe_answer_permission(client, &v).await;
                if let Some(c) = v
                    .pointer("/payload/result/content")
                    .and_then(|c| c.as_str())
                {
                    println!("{c}");
                } else if let Some(m) = v.pointer("/payload/message").and_then(|m| m.as_str()) {
                    eprintln!("(workflow: {m})");
                }
            }
        }
    }
    Ok(())
}

// Write-denial detection is shared with the studio via `chat::parse_denied_capability`.

/// Run one conversational turn: POST the graph + transcript, stream the SSE,
/// and return the assistant's text.
#[allow(clippy::too_many_arguments)]
async fn run_turn(
    client: &Client,
    air: &str,
    session_id: &str,
    prompt: &str,
    user_text: &str,
    capability_grant_ids: &[String],
    opts: &ChatOptions,
    tool_call_budgets: &HashMap<String, usize>,
) -> Result<TurnOutcome> {
    let tool_credentials = parse_kv_string(&opts.tool_auth);
    let body = ExecuteRequest {
        air: air.to_string(),
        args: vec![prompt.to_string()],
        session_id: Some(session_id.to_string()),
        user_text: Some(user_text.to_string()),
        capability_grant_ids: capability_grant_ids.to_vec(),
        imports: opts.import.clone(),
        tool_call_budgets: tool_call_budgets.clone(),
        tool_credentials,
        owner: opts.owner.clone(),
    };
    let resp = client.execute_stream(&body).await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if let Some(cap) = chat::parse_denied_capability(&text) {
            return Ok(TurnOutcome::NeedsGrant(cap));
        }
        return Err(anyhow!(
            "server returned {status} for {}: {text}",
            client.baseurl()
        ));
    }

    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::default();
    let mut snap = RunSnapshot::new(session_id.to_string());
    let mut assistant = String::new();
    let mut error: Option<String> = None;
    let mut streamed_any = false;
    let stdout = std::io::stdout();

    if !opts.tree {
        print!("\nassistant> ");
        let _ = std::io::stdout().flush();
    }

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("SSE chunk read failed")?;
        for frame in parser.feed(&chunk) {
            if let Ok(v) = serde_json::from_str::<JsonValue>(&frame.data) {
                let _ = maybe_answer_permission(client, &v).await;
                let kind = v.pointer("/payload/kind").and_then(|k| k.as_str());
                if kind == Some("token")
                    && let Some(tok) = v.pointer("/payload/text").and_then(|t| t.as_str())
                {
                    if !opts.tree {
                        print!("{tok}");
                        let _ = std::io::stdout().flush();
                    }
                    assistant.push_str(tok);
                    streamed_any = true;
                }
                if kind == Some("thought")
                    && !opts.tree
                    && let Some(th) = v.pointer("/payload/text").and_then(|t| t.as_str())
                {
                    print!("\x1b[2m{th}\x1b[0m");
                    let _ = std::io::stdout().flush();
                }
                if let Some(c) = v
                    .pointer("/payload/result/content")
                    .and_then(|c| c.as_str())
                {
                    assistant = c.to_string();
                }
                if kind == Some("error")
                    && let Some(m) = v.pointer("/payload/message").and_then(|m| m.as_str())
                {
                    error = Some(m.to_string());
                }
            }
            if let Some(ev) = decode_event_frame(&frame) {
                snap.apply(&ev);
            }
        }
        if opts.tree {
            let mut handle = stdout.lock();
            let _ = write!(handle, "\x1B[2J\x1B[H");
            let _ = writeln!(handle, "{}", render_tree(&snap));
            let _ = handle.flush();
        }
    }

    if let Some(msg) = error {
        if !opts.tree && streamed_any {
            println!();
        }
        // A write capability may also be refused mid-stream; route it to HITL.
        if let Some(cap) = chat::parse_denied_capability(&msg) {
            return Ok(TurnOutcome::NeedsGrant(cap));
        }
        return Err(anyhow!(msg));
    }
    if opts.tree {
        println!("{}", render_tree(&snap));
        println!("\nassistant> {assistant}");
    } else if streamed_any {
        // Tokens already streamed inline; just close the line.
        println!();
    } else {
        // Backend didn't stream tokens — print the final content in one shot.
        println!("{assistant}");
    }
    Ok(TurnOutcome::Answered { answer: assistant })
}

async fn handle_meta(cmd: &str, client: &Client) -> Result<bool> {
    let (verb, _rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
    match verb {
        "exit" | "quit" => return Ok(true),
        "help" => {
            eprintln!(
                "meta-commands: /tools /skills /agents /compact /budget /save <path> \
                 /workflow <list|run <path>> /help /exit"
            );
        }
        "tools" => {
            print_list(client, "/v1/capability-templates", "tools").await?;
        }
        "skills" => {
            print_list(client, "/v1/skills", "skills").await?;
        }
        "agents" => {
            print_list(client, "/v1/agents", "agents").await?;
        }
        other => eprintln!("unknown meta-command /{other} (try /help)"),
    }
    Ok(false)
}

/// Fetch a JSON list endpoint and print `name`/`description` (or `skill_id`)
/// rows. Tolerates either a bare array or an object with a `data` array.
async fn print_list(client: &Client, path: &str, label: &str) -> Result<()> {
    let v = client.get_json(path).await?;
    let items = v
        .as_array()
        .cloned()
        .or_else(|| v.get("data").and_then(|d| d.as_array()).cloned())
        .unwrap_or_default();
    if items.is_empty() {
        eprintln!("(no {label})");
        return Ok(());
    }
    for item in items {
        let name = item
            .get("name")
            .or_else(|| item.get("skill_id"))
            .or_else(|| item.get("id"))
            .and_then(|n| n.as_str())
            .unwrap_or("<unknown>");
        let desc = item
            .get("description")
            .or_else(|| item.get("version"))
            .and_then(|d| d.as_str())
            .unwrap_or("");
        if desc.is_empty() {
            eprintln!("  {name}");
        } else {
            eprintln!("  {name} — {desc}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_options() -> ChatOptions {
        ChatOptions {
            air: None,
            server: None,
            session_id: None,
            capability_grant_ids: Vec::new(),
            import: Vec::new(),
            tree: false,
            tools: false,
            backend: None,
            model: None,
            agent: None,
            agent_mode: None,
            agent_model: None,
            monitor_url: None,
            max_turns: None,
            max_events: None,
            tool_budget: Vec::new(),
            tool_cap: Vec::new(),
            tool_auth: Vec::new(),
            owner: None,
            author: false,
        }
    }

    #[test]
    fn conversational_runtime_prompt_names_capability_discovery_only() {
        let prompt = capability_discovery_prompt();
        assert_eq!(
            prompt,
            "Runtime capability discovery: call `capability_discovery` when you need to inspect available capability templates. Treat results as CapabilityTemplateV1 authoring metadata only; they are not authority. Do not invent `capability_id` values. Present writes only when capability_grant_ids are supplied by APXM."
        );
    }

    #[test]
    fn built_in_conversational_agent_exposes_capability_discovery() {
        let opts = test_options();
        let air = builtin_chat_air(&opts, Some(capability_discovery_prompt()));
        assert!(air.contains("capability_groups"));
        assert!(air.contains("\"discovery\""));
        assert!(air.contains("\"skills\""));
        assert!(air.contains("capability_discovery"));
        assert!(air.contains("capability_grant_ids"));
    }
}
