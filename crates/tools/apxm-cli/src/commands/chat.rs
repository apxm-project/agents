//! `apxm chat` — interactive conversational REPL over a running apxm-server.
//!
//! Each user message runs one execution of the agent graph against
//! `POST /v1/execute/stream`, threading a stable `session_id` (so server-side
//! memory accrues across turns — see `ExecutionContext::memory_scope`) and a
//! client-side transcript that is passed back as the graph's `conversation`
//! parameter every turn. This is the host-resident conversational loop: the
//! runtime stays single-shot ("one DAG = one turn") and the REPL drives it.
//! The built-in graph is a direct ASK by default, or a SPAWN_AGENT +
//! COMMUNICATE turn when `--agent` selects an ACP profile such as `claude`.
//!
//! The SSE stream is parsed with the same [`super::watch::SseParser`] +
//! [`super::render`] machinery the `watch` command uses, so the per-agent
//! dispatch tree (`--tree`) renders identically.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use apxm_ais::chat::{self, COMPACT_AT_TOKENS, KEEP_RECENT_TURNS, Role};
use apxm_core::constants::orchestration::admission as orchestration_admission;
use futures::StreamExt;
use serde_json::Value as JsonValue;

use super::render::{RunSnapshot, render_tree};
use super::watch::{SseParser, decode_event_frame};

/// Default apxm-server endpoint. Matches the server's own default bind address
/// (`apxm-server` `DEFAULT_PORT` = 18800). Override via `APXM_SERVER_BASE` or
/// `--server`.
const DEFAULT_SERVER_BASE: &str = "http://127.0.0.1:18800";

/// CLI options for the chat REPL.
#[derive(Debug, Clone)]
pub struct ChatOptions {
    pub air: Option<PathBuf>,
    pub server: Option<String>,
    pub session_id: Option<String>,
    pub admit: Vec<String>,
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
    /// Maximum number of substantive turns (stdin + event) before the loop stops
    /// serving. `None` = unbounded. A stdin turn at the cap is soft-blocked
    /// (warns; `/continue` extends); an event-driven turn at the cap is dropped
    /// (no human to confirm). Clamped to the operator ceiling
    /// `APXM_CHAT_MAX_TURNS_CEILING` if that env var is set.
    pub max_turns: Option<usize>,
    /// Maximum number of event-driven (monitor cue) turns. `None` = unbounded.
    /// Events past the cap are dropped while stdin stays interactive. Clamped to
    /// `APXM_CHAT_MAX_EVENTS_CEILING` if set.
    pub max_events: Option<usize>,
    /// Per-tool, per-turn call budget as raw `CAP=N` strings (Control 2). Sent as
    /// `tool_call_budgets` each turn; the runtime enforces it across the turn's
    /// whole execution tree.
    pub tool_budget: Vec<String>,
    /// Per-tool, per-conversation call cap as raw `CAP=N` strings (Control 2).
    /// Tracked host-side across turns; the effective per-turn budget sent to the
    /// runtime is `min(tool_budget, session_cap − consumed)`.
    pub tool_cap: Vec<String>,
    /// Per-tool auth binding as raw `CAP=CONNECTION_ID` strings (Control 5). The
    /// server resolves each to a bearer token (scoped to `owner`) and the runtime
    /// injects it at the tool's invoke seam; only the connection id is sent.
    pub tool_auth: Vec<String>,
    /// Tenant/owner scope for per-tool credential resolution (Control 5).
    pub owner: Option<String>,
    /// Expose + auto-admit the workflow-authoring tools (`compose_workflow`,
    /// `run_workflow`) so the agent can create and run workflows (Goal 1).
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

    /// The most recent assistant reply, if any (used by `/save`).
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
            skills: true,
            authoring: opts.author,
        }),
    }
}

fn initial_session_grants(opts: &ChatOptions) -> Vec<String> {
    let mut grants = opts.admit.clone();
    if opts.agent.as_deref().and_then(non_empty).is_some()
        && !grants
            .iter()
            .any(|capability| capability == orchestration_admission::SPAWN_AGENT)
    {
        grants.push(orchestration_admission::SPAWN_AGENT.to_string());
    }
    // `--author` admits the write-class authoring tools so the agent can actually
    // create + run workflows (exposure alone doesn't grant execution).
    if opts.author {
        for cap in ["compose_workflow", "run_workflow"] {
            if !grants.iter().any(|g| g == cap) {
                grants.push(cap.to_string());
            }
        }
    }
    grants
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
        if opts.import.is_empty() {
            base
        } else {
            // Tell the agent its visible libraries so it scopes `search_skills`.
            let hint = format!(
                "Imported skill libraries: {}. Call search_skills with these in `imports` to discover them; shared-tier skills are always visible.",
                opts.import.join(", ")
            );
            Some(match base {
                Some(b) => format!("{b}\n\n{hint}"),
                None => hint,
            })
        }
    };
    let air = chat_air_for_options(&opts, context.as_deref())?;
    if let Some(ctx) = &context {
        eprintln!(
            "context: loaded AGENTS.md/CLAUDE.md hierarchy (~{} tokens)",
            ctx.len() / 4
        );
    }

    // No read timeout: SSE streams stall between events, and reqwest's default
    // client already has no read timeout (setting it to zero would, per
    // reqwest's per-read semantics, time out immediately and fail every call).
    let client = reqwest::Client::builder()
        .build()
        .context("failed to build HTTP client")?;

    eprintln!("apxm chat — session {session_id} @ {base}");
    eprintln!("type a message, or /help for meta-commands; /exit to quit");

    let mut convo = Conversation::default();
    // Capabilities granted for this session: starts with --admit and grows as
    // the operator approves write-tool turns interactively.
    let mut session_grants = initial_session_grants(&opts);

    // Optional event source: subscribe to an apxm-os control-plane SSE stream so
    // external cue events (process output, file changes, cron, webhooks) drive
    // turns alongside stdin. The agent is no longer blocked waiting on a human.
    let mut event_rx = match &opts.monitor_url {
        Some(url) => {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            spawn_monitor_subscriber(client.clone(), url.clone(), tx);
            eprintln!("monitor: subscribed to {url}/events — external events become turns");
            Some(rx)
        }
        None => None,
    };

    // Async stdin so it can be `select!`-ed against the event stream.
    use tokio::io::AsyncBufReadExt as _;
    let mut stdin_lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();

    // Turn budgets (Control 1 — host-enforced). The runtime is single-shot
    // ("one DAG = one turn"), so the turn count lives in this outer loop, never
    // in the AIR program. Request values are clamped to the operator ceiling env
    // vars: a request may only *lower* the bound, never raise it.
    let max_turns = clamp_to_ceiling(opts.max_turns, "APXM_CHAT_MAX_TURNS_CEILING");
    let max_events = clamp_to_ceiling(opts.max_events, "APXM_CHAT_MAX_EVENTS_CEILING");
    if let Some(n) = max_turns {
        eprintln!("turn limit: {n} turns (/continue extends by {n})");
    }
    if let Some(n) = max_events {
        eprintln!("event limit: {n} event-driven turns");
    }
    let mut turns_used: usize = 0;
    let mut events_used: usize = 0;
    // Running ceiling on `turns_used`; `/continue` raises it by `max_turns`.
    let mut turn_budget = max_turns;

    // Per-tool call budgets (Control 2). The per-turn budget rides the execute
    // request and is enforced by the runtime across the turn's execution tree;
    // the per-conversation cap is tracked here and folded into each turn's budget.
    let tool_turn_budget = parse_kv_usize(&opts.tool_budget);
    let tool_session_cap = parse_kv_usize(&opts.tool_cap);
    let mut tool_session_consumed: HashMap<String, usize> = HashMap::new();
    if !tool_turn_budget.is_empty() || !tool_session_cap.is_empty() {
        eprintln!(
            "tool budgets: per-turn {tool_turn_budget:?}, per-conversation {tool_session_cap:?}"
        );
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
                    // Force a compaction now (post-hook on demand).
                    match compact_if_needed(&mut convo, &client, &base, &session_id, true).await {
                        Ok(true) => {}
                        Ok(false) => eprintln!("(nothing to compact yet)"),
                        Err(err) => eprintln!("(compaction failed: {err})"),
                    }
                    continue;
                }
                if meta.split(' ').next() == Some("continue") {
                    // Extend the soft turn budget (Control 1) by the configured
                    // increment so the operator can keep going past the cap.
                    match max_turns {
                        Some(n) => {
                            turn_budget = Some(turns_used + n);
                            eprintln!("(extended: budget now {} turns)", turns_used + n);
                        }
                        None => eprintln!("(no turn limit set)"),
                    }
                    continue;
                }
                let (verb, rest) = meta
                    .split_once(' ')
                    .map(|(v, r)| (v, r.trim()))
                    .unwrap_or((meta, ""));
                match verb {
                    // Control 4: inspect / revoke the live capability grant set.
                    "grants" => {
                        if session_grants.is_empty() {
                            eprintln!("(no capabilities granted this session)");
                        } else {
                            eprintln!("granted: {}", session_grants.join(", "));
                        }
                        continue;
                    }
                    "revoke" => {
                        if rest.is_empty() {
                            eprintln!("(usage: /revoke <capability>)");
                        } else if let Some(pos) = session_grants.iter().position(|c| c == rest) {
                            session_grants.remove(pos);
                            eprintln!("(revoked '{rest}' for this session)");
                        } else {
                            eprintln!("('{rest}' was not granted)");
                        }
                        continue;
                    }
                    // Control 2: show per-tool budgets and remaining session headroom.
                    "budget" => {
                        print_budget(&tool_turn_budget, &tool_session_cap, &tool_session_consumed);
                        continue;
                    }
                    // Goal 1: persist the agent's last reply (e.g. an authored
                    // workflow) to disk — operator-initiated, so the agent stays
                    // read-only while still creating workflows.
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
                    // Goal 1: run / list authored workflows (operator-gated).
                    "workflow" => {
                        handle_workflow_meta(rest, &client, &base, &session_id, &session_grants)
                            .await;
                        continue;
                    }
                    _ => {}
                }
                match handle_meta(meta, &client, &base).await {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(err) => eprintln!("(meta-command error: {err})"),
                }
                continue;
            }
        }

        // Enforce the turn budget (Control 1). An event-driven turn at the cap is
        // dropped (no human to confirm); a stdin turn is soft-blocked so the
        // operator can `/continue`. Both stdin and event turns share `turns_used`.
        if from_event {
            if let Some(me) = max_events {
                if events_used >= me {
                    eprintln!("[event] dropped: event limit ({me}) reached");
                    continue;
                }
            }
        }
        if let Some(tb) = turn_budget {
            if turns_used >= tb {
                if from_event {
                    eprintln!("[event] dropped: turn limit ({tb}) reached");
                } else {
                    eprintln!("(turn limit {tb} reached — /continue to extend, /exit to quit)");
                }
                continue;
            }
        }

        handle_user_turn(
            &line,
            &mut convo,
            &mut session_grants,
            &client,
            &base,
            &air,
            &session_id,
            &opts,
            &tool_turn_budget,
            &tool_session_cap,
            &mut tool_session_consumed,
        )
        .await;

        turns_used += 1;
        if from_event {
            events_used += 1;
        }
    }
    Ok(())
}

/// Clamp an optional request bound to an operator ceiling env var. A request may
/// only *lower* the operator ceiling, never raise it: if both are present the
/// effective bound is `min(request, ceiling)`; a ceiling with no request applies
/// on its own. Unparseable ceilings are ignored.
fn clamp_to_ceiling(requested: Option<usize>, ceiling_env: &str) -> Option<usize> {
    let ceiling = std::env::var(ceiling_env)
        .ok()
        .and_then(|v| v.parse::<usize>().ok());
    clamp_bound(requested, ceiling)
}

/// Pure clamp: a request may only *lower* an operator ceiling, never raise it.
fn clamp_bound(requested: Option<usize>, ceiling: Option<usize>) -> Option<usize> {
    match (requested, ceiling) {
        (Some(r), Some(c)) => Some(r.min(c)),
        (Some(r), None) => Some(r),
        (None, Some(c)) => Some(c),
        (None, None) => None,
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

/// Compute the per-tool budget to send for the next turn (Control 2). For each
/// tool with a per-turn and/or per-conversation cap, the effective budget is the
/// lower of the per-turn budget and the session cap's remaining headroom. An
/// exhausted session cap yields 0 — a hard deny at the runtime's invoke seam.
fn effective_turn_budgets(
    turn: &HashMap<String, usize>,
    session_cap: &HashMap<String, usize>,
    consumed: &HashMap<String, usize>,
) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for name in turn.keys().chain(session_cap.keys()) {
        if out.contains_key(name) {
            continue;
        }
        let per_turn = turn.get(name).copied();
        let session_remaining = session_cap
            .get(name)
            .map(|cap| cap.saturating_sub(consumed.get(name).copied().unwrap_or(0)));
        let effective = clamp_bound(per_turn, session_remaining);
        if let Some(n) = effective {
            out.insert(name.clone(), n);
        }
    }
    out
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
/// interactive HITL grant retry loop. Errors are reported, never fatal — the
/// REPL keeps serving.
#[allow(clippy::too_many_arguments)]
async fn handle_user_turn(
    user_text: &str,
    convo: &mut Conversation,
    session_grants: &mut Vec<String>,
    client: &reqwest::Client,
    base: &str,
    air: &str,
    session_id: &str,
    opts: &ChatOptions,
    tool_turn_budget: &HashMap<String, usize>,
    tool_session_cap: &HashMap<String, usize>,
    tool_session_consumed: &mut HashMap<String, usize>,
) {
    let prompt = convo.render(user_text);
    // The per-tool budget for THIS turn folds the per-turn budget with the
    // remaining per-conversation cap (Control 2).
    let turn_budgets =
        effective_turn_budgets(tool_turn_budget, tool_session_cap, tool_session_consumed);
    // On a refused write capability, prompt the operator; on approval grant it
    // for the session and retry the SAME turn. Rides the static admission path.
    loop {
        match run_turn(
            client,
            base,
            air,
            session_id,
            &prompt,
            session_grants,
            opts,
            &turn_budgets,
        )
        .await
        {
            Ok(TurnOutcome::Answered {
                answer,
                tool_call_counts,
            }) => {
                // Fold this turn's consumption into the running session totals so
                // the next turn's budget tightens (per-conversation cap).
                for (name, count) in tool_call_counts {
                    *tool_session_consumed.entry(name).or_insert(0) += count;
                }
                record_and_compact(convo, user_text, answer, client, base, session_id).await;
                break;
            }
            Ok(TurnOutcome::NeedsGrant(cap)) => match prompt_grant(&cap) {
                Ok(true) => {
                    session_grants.push(cap);
                    continue;
                }
                Ok(false) => {
                    eprintln!("(denied; turn skipped)");
                    break;
                }
                Err(err) => {
                    eprintln!("(grant prompt failed: {err})");
                    break;
                }
            },
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
    /// The assistant produced a reply, with the per-tool call counts consumed
    /// this turn (Control 2 — for host-side session accounting).
    Answered {
        answer: String,
        tool_call_counts: HashMap<String, usize>,
    },
    /// A write capability was refused; the operator must grant it to proceed.
    NeedsGrant(String),
}

/// Record a completed turn, then run compaction (the host loop's post-hook):
/// if the transcript is over budget, fold the oldest turns into the running
/// summary so future turns stay within the model context window.
async fn record_and_compact(
    convo: &mut Conversation,
    user: &str,
    answer: String,
    client: &reqwest::Client,
    base: &str,
    session_id: &str,
) {
    convo.record(user.to_string(), answer);
    if convo.token_estimate() > COMPACT_AT_TOKENS {
        if let Err(err) = compact_if_needed(convo, client, base, session_id, false).await {
            eprintln!("(compaction skipped: {err})");
        }
    }
}

/// Fold the oldest turns into the running summary via a quiet summarize turn.
/// `force` compacts regardless of the token threshold (used by `/compact`).
/// On any failure the transcript is left untouched (no turn is lost).
async fn compact_if_needed(
    convo: &mut Conversation,
    client: &reqwest::Client,
    base: &str,
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
    let new_summary = summarize_quiet(client, base, session_id, &foldable).await?;
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
async fn summarize_quiet(
    client: &reqwest::Client,
    base: &str,
    session_id: &str,
    text: &str,
) -> Result<String> {
    let url = format!("{}/v1/execute/stream", base.trim_end_matches('/'));
    let body = serde_json::json!({
        "air": chat::SUMMARIZE_AIR,
        "args": [text],
        "session_id": session_id,
    });
    let resp = client
        .post(&url)
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .with_context(|| format!("failed to POST {url}"))?;
    anyhow::ensure!(
        resp.status().is_success(),
        "summarize returned {} for {url}",
        resp.status()
    );
    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::default();
    let mut summary = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("SSE chunk read failed")?;
        for frame in parser.feed(&chunk) {
            if let Ok(v) = serde_json::from_str::<JsonValue>(&frame.data)
                && let Some(c) = v
                    .pointer("/payload/result/content")
                    .and_then(|c| c.as_str())
            {
                summary = c.to_string();
            }
        }
    }
    Ok(summary)
}

/// Print the per-tool call budgets and remaining per-conversation headroom
/// (Control 2), for the `/budget` meta-command.
fn print_budget(
    turn: &HashMap<String, usize>,
    session_cap: &HashMap<String, usize>,
    consumed: &HashMap<String, usize>,
) {
    if turn.is_empty() && session_cap.is_empty() {
        eprintln!("(no tool budgets set)");
        return;
    }
    let mut names: Vec<&String> = turn.keys().chain(session_cap.keys()).collect();
    names.sort();
    names.dedup();
    for name in names {
        let per_turn = turn
            .get(name)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "∞".to_string());
        match session_cap.get(name) {
            Some(cap) => {
                let used = consumed.get(name).copied().unwrap_or(0);
                eprintln!(
                    "  {name}: per-turn {per_turn}, session {used}/{cap} ({} left)",
                    cap.saturating_sub(used)
                );
            }
            None => eprintln!("  {name}: per-turn {per_turn}, session ∞"),
        }
    }
}

/// Handle `/workflow <sub>` meta-commands (Goal 1): `list` enumerates `.air` /
/// `.apxmw` files in the cwd; `run <path>` reads the file client-side and runs it
/// through `/v1/compile/stream`, admitting the session's granted capabilities.
async fn handle_workflow_meta(
    rest: &str,
    client: &reqwest::Client,
    base: &str,
    session_id: &str,
    session_grants: &[String],
) {
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
                    if let Err(e) =
                        run_workflow_air(client, base, session_id, &air, session_grants).await
                    {
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
/// final content. Admits the session's granted capabilities so writes the
/// operator already approved carry through.
async fn run_workflow_air(
    client: &reqwest::Client,
    base: &str,
    session_id: &str,
    air: &str,
    session_grants: &[String],
) -> Result<()> {
    let url = format!("{}/v1/compile/stream", base.trim_end_matches('/'));
    let body = serde_json::json!({
        "air": air,
        "session_id": session_id,
        "admit_capabilities": session_grants,
    });
    let resp = client
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

/// Prompt the operator to grant a write capability for the session.
fn prompt_grant(capability: &str) -> Result<bool> {
    eprint!("grant write capability '{capability}' for this session? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("stdin read failed")?;
    let a = answer.trim().to_ascii_lowercase();
    Ok(a == "y" || a == "yes")
}

// Write-denial detection is shared with the studio via `chat::parse_denied_capability`.

/// Run one conversational turn: POST the graph + transcript, stream the SSE,
/// and return the assistant's text.
async fn run_turn(
    client: &reqwest::Client,
    base: &str,
    air: &str,
    session_id: &str,
    prompt: &str,
    admit: &[String],
    opts: &ChatOptions,
    tool_call_budgets: &HashMap<String, usize>,
) -> Result<TurnOutcome> {
    let url = format!("{}/v1/execute/stream", base.trim_end_matches('/'));
    // Per-tool auth bindings: capability -> apxm-auth connection id (Control 5).
    // Only the connection id travels; the server resolves the token.
    let tool_credentials = parse_kv_string(&opts.tool_auth);
    // Args bind POSITIONALLY to graph parameters — the bare transcript is the
    // single `conversation` parameter (no `name=value` parsing on this path).
    let body = serde_json::json!({
        "air": air,
        "args": [prompt],
        "session_id": session_id,
        "admit_capabilities": admit,
        "imports": opts.import,
        "tool_call_budgets": tool_call_budgets,
        "tool_credentials": tool_credentials,
        "owner": opts.owner,
    });
    let resp = client
        .post(&url)
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .with_context(|| format!("failed to POST {url}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        // A pre-stream 400 from the static write-admission check means a write
        // capability needs explicit consent; surface it for the HITL prompt.
        if let Some(cap) = chat::parse_denied_capability(&text) {
            return Ok(TurnOutcome::NeedsGrant(cap));
        }
        return Err(anyhow!("server returned {status} for {url}: {text}"));
    }

    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::default();
    let mut snap = RunSnapshot::new(session_id.to_string());
    let mut assistant = String::new();
    let mut tool_call_counts: HashMap<String, usize> = HashMap::new();
    let mut error: Option<String> = None;
    let mut streamed_any = false;
    let stdout = std::io::stdout();

    // Live streaming: in plain (non-tree) mode, print each `token` event's text
    // as it arrives so the reply appears incrementally. The tree view renders
    // the dispatch tree instead, so token streaming is suppressed there.
    if !opts.tree {
        print!("\nassistant> ");
        let _ = std::io::stdout().flush();
    }

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("SSE chunk read failed")?;
        for frame in parser.feed(&chunk) {
            if let Ok(v) = serde_json::from_str::<JsonValue>(&frame.data) {
                let kind = v.pointer("/payload/kind").and_then(|k| k.as_str());
                // Stream tokens live as they are produced.
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
                // Stream extended-thinking blocks dimmed, so the reasoning is
                // visible without being mistaken for the final answer.
                if kind == Some("thought")
                    && !opts.tree
                    && let Some(th) = v.pointer("/payload/text").and_then(|t| t.as_str())
                {
                    print!("\x1b[2m{th}\x1b[0m");
                    let _ = std::io::stdout().flush();
                }
                // ExecuteComplete carries the authoritative final content.
                if let Some(c) = v
                    .pointer("/payload/result/content")
                    .and_then(|c| c.as_str())
                {
                    assistant = c.to_string();
                }
                // …and the per-tool call counts consumed (Control 2 session cap).
                if let Some(counts) = v
                    .pointer("/payload/result/tool_call_counts")
                    .and_then(|c| c.as_object())
                {
                    for (name, value) in counts {
                        if let Some(n) = value.as_u64() {
                            tool_call_counts.insert(name.clone(), n as usize);
                        }
                    }
                }
                // ErrorPayload carries a message.
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
    Ok(TurnOutcome::Answered {
        answer: assistant,
        tool_call_counts,
    })
}

/// Handle an in-REPL `/meta` command. Returns `Ok(true)` to exit the loop.
async fn handle_meta(cmd: &str, client: &reqwest::Client, base: &str) -> Result<bool> {
    let (verb, _rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
    match verb {
        "exit" | "quit" => return Ok(true),
        "help" => {
            eprintln!(
                "meta-commands: /tools /skills /agents /compact /continue /grants /revoke <cap> \
                 /budget /save <path> /workflow <list|run <path>> /help /exit"
            );
        }
        "tools" => {
            print_list(client, &format!("{base}/v1/capabilities"), "tools").await?;
        }
        "skills" => {
            print_list(client, &format!("{base}/v1/skills"), "skills").await?;
        }
        "agents" => {
            print_list(client, &format!("{base}/v1/agents"), "agents").await?;
        }
        other => eprintln!("unknown meta-command /{other} (try /help)"),
    }
    Ok(false)
}

/// Fetch a JSON list endpoint and print `name`/`description` (or `skill_id`)
/// rows. Tolerates either a bare array or an object with a `data` array.
async fn print_list(client: &reqwest::Client, url: &str, label: &str) -> Result<()> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;
    anyhow::ensure!(
        resp.status().is_success(),
        "server returned {} for {url}",
        resp.status()
    );
    let v: JsonValue = resp.json().await.context("invalid JSON from server")?;
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

    fn chat_opts() -> ChatOptions {
        ChatOptions {
            air: None,
            server: None,
            session_id: None,
            admit: Vec::new(),
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
    fn clamp_bound_lets_request_only_lower_operator_ceiling() {
        // No ceiling: the request passes through unchanged.
        assert_eq!(clamp_bound(Some(10), None), Some(10));
        assert_eq!(clamp_bound(None, None), None);
        // Ceiling clamps a higher request, leaves a lower one, and applies on its
        // own when no request is given. A request can only lower, never raise.
        assert_eq!(clamp_bound(Some(10), Some(5)), Some(5));
        assert_eq!(clamp_bound(Some(3), Some(5)), Some(3));
        assert_eq!(clamp_bound(None, Some(5)), Some(5));
    }

    #[test]
    fn effective_turn_budgets_folds_session_cap_into_per_turn() {
        let turn = HashMap::from([("web.fetch".to_string(), 5usize)]);
        let session_cap = HashMap::from([
            ("web.fetch".to_string(), 8usize),
            ("bash".to_string(), 3usize),
        ]);

        // Fresh conversation: web.fetch = min(per-turn 5, remaining 8) = 5;
        // bash has only a session cap, so it passes through as 3.
        let consumed = HashMap::new();
        let b = effective_turn_budgets(&turn, &session_cap, &consumed);
        assert_eq!(b.get("web.fetch").copied(), Some(5));
        assert_eq!(b.get("bash").copied(), Some(3));

        // After consuming 6 of web.fetch's 8, the remaining 2 caps the per-turn 5.
        let consumed = HashMap::from([("web.fetch".to_string(), 6usize)]);
        let b = effective_turn_budgets(&turn, &session_cap, &consumed);
        assert_eq!(b.get("web.fetch").copied(), Some(2));

        // Exhausted session cap yields 0 — a hard deny next turn.
        let consumed = HashMap::from([("web.fetch".to_string(), 8usize)]);
        let b = effective_turn_budgets(&turn, &session_cap, &consumed);
        assert_eq!(b.get("web.fetch").copied(), Some(0));

        // A tool with neither cap is absent (unbounded).
        assert!(b.get("read").is_none());
    }

    #[test]
    fn parse_kv_usize_parses_and_skips_malformed() {
        let parsed = parse_kv_usize(&[
            "web.fetch=5".to_string(),
            "bash = 2".to_string(),
            "bad".to_string(),
            "x=notnum".to_string(),
        ]);
        assert_eq!(parsed.get("web.fetch").copied(), Some(5));
        assert_eq!(parsed.get("bash").copied(), Some(2));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn author_flag_admits_the_workflow_authoring_tools() {
        let mut opts = chat_opts();
        opts.author = true;
        let grants = initial_session_grants(&opts);
        assert!(grants.iter().any(|g| g == "compose_workflow"));
        assert!(grants.iter().any(|g| g == "run_workflow"));
        // Without --author, they are not admitted.
        let grants = initial_session_grants(&chat_opts());
        assert!(!grants.iter().any(|g| g == "compose_workflow"));
    }

    #[test]
    fn parse_kv_string_parses_connection_bindings() {
        let parsed = parse_kv_string(&[
            "slack.post=conn_42".to_string(),
            "github.create_issue = conn_7".to_string(),
            "bad".to_string(),
            "empty=".to_string(),
        ]);
        assert_eq!(
            parsed.get("slack.post").map(String::as_str),
            Some("conn_42")
        );
        assert_eq!(
            parsed.get("github.create_issue").map(String::as_str),
            Some("conn_7")
        );
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn cue_event_to_turn_summarizes_structured_event() {
        let turn = cue_event_to_turn(
            r#"{"kind":"process","cue_id":"process:tail","payload":{"event":"line","line":"ERROR boom"}}"#,
        );
        assert!(turn.contains("kind=process"), "got: {turn}");
        assert!(turn.contains("cue=process:tail"), "got: {turn}");
        assert!(turn.contains("ERROR boom"), "payload included: {turn}");
    }

    #[test]
    fn cue_event_to_turn_falls_back_to_raw_on_bad_json() {
        let turn = cue_event_to_turn("not json");
        assert!(turn.contains("not json"));
    }

    #[test]
    fn conversation_render_threads_history() {
        let mut c = Conversation::default();
        c.record("hi".into(), "hello".into());
        let rendered = c.render("how are you");
        assert!(rendered.contains("User: hi"));
        assert!(rendered.contains("Assistant: hello"));
        assert!(rendered.ends_with("User: how are you\nAssistant:"));
    }

    #[test]
    fn empty_conversation_renders_single_turn() {
        let c = Conversation::default();
        let rendered = c.render("first");
        assert_eq!(rendered, "User: first\nAssistant:");
    }

    #[test]
    fn builtin_air_is_a_single_ask_over_conversation() {
        let air = chat::chat_air(&chat::ChatAirOptions::default());
        assert!(air.contains("ais.ask"));
        assert!(air.contains("conversation"));
        assert!(air.contains("ais.entry"));
    }

    #[test]
    fn chat_default_mode_builds_direct_ask_without_spawn_grant() {
        let mut opts = chat_opts();
        opts.backend = Some("amd".to_string());
        opts.model = Some("cheap-model".to_string());
        let air = builtin_chat_air(&opts, Some("project instructions"));

        assert!(air.contains("ais.ask"), "{air}");
        assert!(air.contains("backend = \"amd\""), "{air}");
        assert!(air.contains("model = \"cheap-model\""), "{air}");
        assert!(!air.contains("ais.spawn_agent"), "{air}");
        assert!(
            !initial_session_grants(&opts)
                .contains(&orchestration_admission::SPAWN_AGENT.to_string())
        );
    }

    #[test]
    fn chat_agent_mode_builds_acp_graph_and_spawn_grant() {
        let mut opts = chat_opts();
        opts.agent = Some("claude".to_string());
        opts.agent_mode = Some("architect".to_string());
        opts.agent_model = Some("claude-3-5-haiku-latest".to_string());
        let air = builtin_chat_air(&opts, Some("orchestrate through APXM"));

        assert!(air.contains("ais.spawn_agent"), "{air}");
        assert!(air.contains("profile = \"claude\""), "{air}");
        assert!(air.contains("mode = \"architect\""), "{air}");
        assert!(air.contains("model = \"claude-3-5-haiku-latest\""), "{air}");
        assert!(air.contains("ais.communicate"), "{air}");
        assert!(
            initial_session_grants(&opts)
                .contains(&orchestration_admission::SPAWN_AGENT.to_string())
        );
    }

    #[test]
    fn chat_agent_mode_does_not_duplicate_spawn_grant() {
        let mut opts = chat_opts();
        opts.agent = Some("claude".to_string());
        opts.admit = vec![orchestration_admission::SPAWN_AGENT.to_string()];
        let grants = initial_session_grants(&opts);

        assert_eq!(
            grants
                .iter()
                .filter(|grant| *grant == orchestration_admission::SPAWN_AGENT)
                .count(),
            1
        );
    }

    #[test]
    fn summarize_air_validates_shape() {
        assert!(chat::SUMMARIZE_AIR.contains("ais.ask"));
        assert!(chat::SUMMARIZE_AIR.contains("to_summarize"));
        assert!(chat::SUMMARIZE_AIR.contains("ais.entry"));
    }

    #[test]
    fn render_with_summary_prepends_block_else_identical() {
        // No summary: byte-identical to a flat transcript (back-compat).
        let mut c = Conversation::default();
        c.record("hi".into(), "hello".into());
        assert_eq!(
            c.render("next"),
            "User: hi\nAssistant: hello\nUser: next\nAssistant:"
        );
        // With a summary: a System turn carries it (same framing as the studio).
        c.summary = "earlier we discussed X".into();
        let r = c.render("next");
        assert!(r.starts_with("System: Summary of earlier conversation: earlier we discussed X\n"));
        assert!(r.contains("User: hi\nAssistant: hello"));
    }

    #[test]
    fn compaction_keeps_recent_and_folds_old() {
        let mut c = Conversation::default();
        for i in 0..(KEEP_RECENT_TURNS + 3) {
            c.record(format!("u{i}"), format!("a{i}"));
        }
        // The oldest turns are foldable; recent KEEP_RECENT stay.
        let foldable = c.foldable_text().expect("should have foldable turns");
        assert!(foldable.contains("u0"));
        c.apply_compaction("folded summary".into());
        assert_eq!(c.turns.len(), KEEP_RECENT_TURNS);
        assert_eq!(c.summary, "folded summary");
        // Newest turn is preserved verbatim.
        let last = KEEP_RECENT_TURNS + 2;
        assert!(c.render("x").contains(&format!("u{last}")));
    }

    #[test]
    fn no_fold_when_under_keep_recent() {
        let mut c = Conversation::default();
        c.record("only".into(), "one".into());
        assert!(c.foldable_text().is_none());
    }

    #[test]
    fn token_estimate_grows_with_content() {
        let mut c = Conversation::default();
        let base = c.token_estimate();
        c.record("a".repeat(400), "b".repeat(400));
        assert!(c.token_estimate() > base + 100);
    }

    #[test]
    fn mint_session_id_is_prefixed_and_nonempty() {
        let s = mint_session_id();
        assert!(s.starts_with("chat-"));
        assert!(s.len() > "chat-".len());
    }

    // Write-denial parsing is tested in `apxm_ais::chat` (the shared impl).
}
