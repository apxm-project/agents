//! `apxm chat` — interactive conversational REPL over a running apxm-server.
//!
//! Each user message runs one execution of the agent graph against
//! `POST /v1/execute/stream`, threading a stable `session_id` (so server-side
//! memory accrues across turns — see `ExecutionContext::memory_scope`) and a
//! client-side transcript that is passed back as the graph's `conversation`
//! parameter every turn. This is the host-resident conversational loop: the
//! runtime stays single-shot ("one DAG = one turn") and the REPL drives it.
//!
//! The SSE stream is parsed with the same [`super::watch::SseParser`] +
//! [`super::render`] machinery the `watch` command uses, so the per-agent
//! dispatch tree (`--tree`) renders identically.

use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use apxm_ais::chat::{self, COMPACT_AT_TOKENS, KEEP_RECENT_TURNS, Role};
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
    pub tree: bool,
    /// Enable the agent's `web` tool group each turn (ignored when `--air` is set).
    pub tools: bool,
    /// Pin each turn to a registered backend (ignored when `--air` is set).
    pub backend: Option<String>,
    /// Pin each turn to a specific model id (ignored when `--air` is set).
    pub model: Option<String>,
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

/// Entry point dispatched from `main.rs`.
pub async fn chat_command(opts: ChatOptions) -> Result<()> {
    let base = opts
        .server
        .clone()
        .or_else(|| std::env::var("APXM_SERVER_BASE").ok())
        .unwrap_or_else(|| DEFAULT_SERVER_BASE.to_string());
    let session_id = opts.session_id.clone().unwrap_or_else(mint_session_id);
    // `--air` drives a custom graph; otherwise build the shared built-in chat
    // graph, threading the `--tools` / `--backend` controls through it (the same
    // builder the studio uses, so the two stay identical).
    let air = match &opts.air {
        Some(p) => std::fs::read_to_string(p)
            .with_context(|| format!("failed to read AIR graph {}", p.display()))?,
        None => chat::chat_air(&chat::ChatAirOptions {
            backend: opts.backend.as_deref(),
            model: opts.model.as_deref(),
            tools: opts.tools,
        }),
    };

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
    let mut session_grants: Vec<String> = opts.admit.clone();
    let stdin = std::io::stdin();
    loop {
        eprint!("\nuser> ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let n = stdin.read_line(&mut line).context("stdin read failed")?;
        if n == 0 {
            // EOF (Ctrl-D): clean exit.
            eprintln!();
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
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
            match handle_meta(meta, &client, &base).await {
                Ok(true) => break,
                Ok(false) => {}
                Err(err) => eprintln!("(meta-command error: {err})"),
            }
            continue;
        }

        let prompt = convo.render(line);
        // Interactive HITL grant loop (parity with the studio frontend): on a
        // refused write capability, prompt the operator; on approval grant it for
        // the session and retry the SAME turn — repeating if a further write is
        // refused. Rides the static admission path (admit_capabilities).
        loop {
            match run_turn(&client, &base, &air, &session_id, &prompt, &session_grants, &opts).await
            {
                Ok(TurnOutcome::Answered(answer)) => {
                    record_and_compact(&mut convo, line, answer, &client, &base, &session_id).await;
                    break;
                }
                Ok(TurnOutcome::NeedsGrant(cap)) => {
                    if prompt_grant(&cap)? {
                        session_grants.push(cap);
                        continue; // re-run with the new grant
                    }
                    eprintln!("(denied; turn skipped)");
                    break;
                }
                Err(err) => {
                    eprintln!("(turn failed: {err})");
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Outcome of one conversational turn.
enum TurnOutcome {
    /// The assistant produced a reply.
    Answered(String),
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
                && let Some(c) = v.pointer("/payload/result/content").and_then(|c| c.as_str())
            {
                summary = c.to_string();
            }
        }
    }
    Ok(summary)
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
) -> Result<TurnOutcome> {
    let url = format!("{}/v1/execute/stream", base.trim_end_matches('/'));
    // Args bind POSITIONALLY to graph parameters — the bare transcript is the
    // single `conversation` parameter (no `name=value` parsing on this path).
    let body = serde_json::json!({
        "air": air,
        "args": [prompt],
        "session_id": session_id,
        "admit_capabilities": admit,
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
                if let Some(c) = v.pointer("/payload/result/content").and_then(|c| c.as_str()) {
                    assistant = c.to_string();
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
    Ok(TurnOutcome::Answered(assistant))
}

/// Handle an in-REPL `/meta` command. Returns `Ok(true)` to exit the loop.
async fn handle_meta(cmd: &str, client: &reqwest::Client, base: &str) -> Result<bool> {
    let (verb, _rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
    match verb {
        "exit" | "quit" => return Ok(true),
        "help" => {
            eprintln!("meta-commands: /tools  /skills  /agents  /compact  /help  /exit");
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
        assert_eq!(c.render("next"), "User: hi\nAssistant: hello\nUser: next\nAssistant:");
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
