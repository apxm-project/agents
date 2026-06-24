//! `apxm goal` - user-facing goal execution over APXM server MCP.
//!
//! The CLI stays thin: it calls the server-owned `goal_start` tool once, then
//! follows aggregate goal status/events by `goal_id`.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use apxm_core::constants::jsonrpc;
use apxm_core::constants::mcp::{self as mcp_constants, fields as mcp_fields, tools as mcp_tools};
use apxm_core::constants::orchestration::admission as goal_admission;
use apxm_core::constants::orchestration::execution_status as goal_execution_status;
use apxm_core::events::kind as event_kind_constants;
use apxm_core::types::OrchestrationWorkspaceMode;
use apxm_core::types::{AISOperationType, OrchestrationTransport, OrchestrationWorkspaceCleanup};
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use super::cli::GoalArgs;

const DEFAULT_SERVER_BASE: &str = "http://127.0.0.1:18800";
const TEMPLATE_GOAL_WORKER_ROLE: &str = "goal_worker_role";

#[derive(Debug, Clone, PartialEq, Eq)]
enum GoalMode {
    Start(String),
    Status(String),
    Events(String),
    Cancel(String),
}

#[derive(Debug, Clone)]
struct WorkerRequest {
    id: String,
    role: String,
    profile: Option<String>,
    depends_on: Vec<String>,
}

#[derive(Debug, Clone)]
struct FollowResult {
    events_seen: usize,
    terminal_event_kind: Option<String>,
    status: JsonValue,
}

pub async fn goal_command(args: GoalArgs, json_output: bool) -> Result<()> {
    let mode = resolve_mode(&args)?;
    let base = resolve_server_base(args.server.as_deref());
    let client = reqwest::Client::builder()
        .build()
        .context("failed to build HTTP client")?;

    match mode {
        GoalMode::Start(task) => {
            let max_iterations = args.max_iterations.unwrap_or(1).max(1);
            let request = build_start_arguments(&args, &task, max_iterations)?;
            let started =
                call_mcp_tool(&client, &base, mcp_tools::APXM_GOAL_START, request).await?;
            if !json_output {
                print_start_summary(&base, &started);
            }

            let should_follow = !args.no_follow
                && !args.dry_run
                && started.get("goal_id").and_then(JsonValue::as_str).is_some();
            let follow = if should_follow {
                let goal_id = started
                    .get("goal_id")
                    .and_then(JsonValue::as_str)
                    .expect("checked goal_id");
                let follow = follow_goal(
                    &client,
                    &base,
                    goal_id,
                    args.limit,
                    args.timeout_secs.map(Duration::from_secs),
                    !json_output,
                )
                .await?;
                if !json_output {
                    print_final_status(&follow.status);
                }
                Some(follow)
            } else {
                None
            };
            if json_output {
                let mut output = json!({ "start": started });
                if let Some(follow) = follow {
                    output["events_seen"] = json!(follow.events_seen);
                    output["terminal_event_kind"] = json!(follow.terminal_event_kind);
                    output["status"] = follow.status;
                }
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
        }
        GoalMode::Status(goal_id) => {
            let status = goal_status(&client, &base, &goal_id).await?;
            print_json_or_status(json_output, status)?;
        }
        GoalMode::Events(goal_id) => {
            let events = goal_events(&client, &base, &goal_id, 0, args.limit).await?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&events)?);
            } else {
                for event in events
                    .get("events")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(line) = summarize_event(event) {
                        println!("{line}");
                    }
                }
            }
        }
        GoalMode::Cancel(goal_id) => {
            let cancelled = call_mcp_tool(
                &client,
                &base,
                mcp_tools::APXM_GOAL_CANCEL,
                json!({ "goal_id": goal_id }),
            )
            .await?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&cancelled)?);
            } else {
                println!(
                    "cancelled: {}",
                    cancelled
                        .get("goal_id")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("<unknown>")
                );
            }
        }
    }

    Ok(())
}

fn resolve_mode(args: &GoalArgs) -> Result<GoalMode> {
    let mut modes = Vec::new();
    if let Some(task) = args.task.as_ref().filter(|value| !value.trim().is_empty()) {
        modes.push(GoalMode::Start(task.clone()));
    }
    if let Some(id) = args
        .status
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        modes.push(GoalMode::Status(id.clone()));
    }
    if let Some(id) = args
        .events
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        modes.push(GoalMode::Events(id.clone()));
    }
    if let Some(id) = args
        .cancel
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        modes.push(GoalMode::Cancel(id.clone()));
    }

    match modes.len() {
        1 => Ok(modes.remove(0)),
        0 => bail!(
            "provide a goal task, or use exactly one of --status, --events, or --cancel with a goal id"
        ),
        _ => bail!("use exactly one goal mode: TASK, --status, --events, or --cancel"),
    }
}

fn resolve_server_base(server: Option<&str>) -> String {
    server
        .map(str::to_string)
        .or_else(|| std::env::var("APXM_SERVER_BASE").ok())
        .unwrap_or_else(|| DEFAULT_SERVER_BASE.to_string())
}

fn build_start_arguments(args: &GoalArgs, task: &str, max_iterations: usize) -> Result<JsonValue> {
    let auto_plan = should_auto_plan(args);
    let workers = if auto_plan {
        Vec::new()
    } else {
        build_workers(args)?
    };
    let uses_profiles =
        workers.iter().any(|worker| worker.profile.is_some()) || args.supervisor_profile.is_some();

    let mut root = JsonMap::new();
    root.insert("task".to_string(), JsonValue::String(task.to_string()));
    insert_optional_string(&mut root, "context", args.context.as_deref());
    root.insert(
        "max_iterations".to_string(),
        JsonValue::from(max_iterations as u64),
    );
    insert_optional_string(&mut root, "event", args.event.as_deref());
    insert_optional_string(&mut root, "trigger", args.trigger.as_deref());
    if let Some(session_id) = args.session_id.as_deref().and_then(non_empty) {
        root.insert(
            "session_id".to_string(),
            JsonValue::String(session_id.to_string()),
        );
    }
    if auto_plan {
        root.insert("planning".to_string(), json!({ "mode": "auto" }));
    } else {
        root.insert(
            "workers".to_string(),
            JsonValue::Array(workers.iter().map(worker_to_json).collect()),
        );
    }
    if auto_plan || args.use_agents {
        root.insert(
            "selection".to_string(),
            json!({
                "agents": "auto",
                "require_agents": true,
            }),
        );
    }
    root.insert("workspace".to_string(), workspace_json(args)?);

    if let Some(profile) = args
        .supervisor_profile.as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        root.insert(
            "supervisor".to_string(),
            json!({
                "id": "gate",
                "profile": profile,
                "transport": OrchestrationTransport::Acp.as_str(),
            }),
        );
    }

    let mut admit = BTreeSet::new();
    for cap in &args.admit {
        if !cap.trim().is_empty() {
            admit.insert(cap.trim().to_string());
        }
    }
    if args.admit_spawn || uses_profiles || args.use_agents || auto_plan {
        admit.insert(goal_admission::SPAWN_AGENT.to_string());
    }
    root.insert(
        "admit_capabilities".to_string(),
        JsonValue::Array(admit.into_iter().map(JsonValue::String).collect()),
    );
    if !args.import.is_empty() {
        root.insert(
            "imports".to_string(),
            JsonValue::Array(args.import.iter().cloned().map(JsonValue::String).collect()),
        );
    }
    root.insert("dry_run".to_string(), JsonValue::Bool(args.dry_run));
    Ok(JsonValue::Object(root))
}

fn should_auto_plan(args: &GoalArgs) -> bool {
    args.workers.is_empty()
        && args.depends.is_empty()
        && args.planner_profile.is_none()
        && args.executor_profile.is_none()
        && args.verifier_profile.is_none()
        && args.critics.is_empty()
        && args.reviewers.is_empty()
}

fn build_workers(args: &GoalArgs) -> Result<Vec<WorkerRequest>> {
    let deps = parse_dependencies(&args.depends)?;
    let mut workers = if args.workers.is_empty() {
        let mut workers = default_workers(args)?;
        let reviewers = build_reviewers(args, vec!["executor".to_string()])?;
        let reviewer_ids = reviewers
            .iter()
            .map(|worker| worker.id.clone())
            .collect::<Vec<_>>();
        let verifier_index = workers
            .iter()
            .position(|worker| worker.id == "verifier")
            .expect("default workers include verifier");
        workers.splice(verifier_index..verifier_index, reviewers);
        if let Some(verifier) = workers.iter_mut().find(|worker| worker.id == "verifier") {
            verifier.depends_on.extend(reviewer_ids);
        }
        workers
    } else {
        if args.planner_profile.is_some()
            || args.executor_profile.is_some()
            || args.verifier_profile.is_some()
        {
            bail!(
                "--planner/--executor/--verifier apply only to the default worker set; use --worker ID:ROLE:PROFILE for custom workers"
            );
        }
        let mut workers = args
            .workers
            .iter()
            .map(|raw| parse_worker(raw))
            .collect::<Result<Vec<_>>>()?;
        apply_known_dependencies(&mut workers, &deps);
        let reviewer_depends = terminal_worker_ids(&workers);
        workers.extend(build_reviewers(args, reviewer_depends)?);
        workers
    };
    ensure_unique_worker_ids(&workers)?;

    apply_dependencies(&mut workers, deps)?;
    Ok(workers)
}

fn default_workers(args: &GoalArgs) -> Result<Vec<WorkerRequest>> {
    Ok(vec![
        WorkerRequest {
            id: "planner".to_string(),
            role: default_goal_role("planner", "planner")?,
            profile: args
                .planner_profile
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            depends_on: Vec::new(),
        },
        WorkerRequest {
            id: "executor".to_string(),
            role: default_goal_role("executor", "executor")?,
            profile: args
                .executor_profile
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            depends_on: vec!["planner".to_string()],
        },
        WorkerRequest {
            id: "verifier".to_string(),
            role: default_goal_role("verifier", "verifier")?,
            profile: args
                .verifier_profile
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            depends_on: vec!["executor".to_string()],
        },
    ])
}

fn parse_worker(raw: &str) -> Result<WorkerRequest> {
    parse_worker_with_default(raw, |id| default_goal_role("custom", id))
}

fn parse_worker_with_default(
    raw: &str,
    default_role: impl FnOnce(&str) -> Result<String>,
) -> Result<WorkerRequest> {
    let mut parts = raw.splitn(3, ':');
    let id = parts
        .next()
        .and_then(non_empty)
        .ok_or_else(|| anyhow!("worker spec must start with an id"))?;
    let role = match parts.next().and_then(non_empty) {
        Some(role) => role.to_string(),
        None => default_role(id)?,
    };
    let profile = parts.next().and_then(non_empty).map(str::to_string);
    Ok(WorkerRequest {
        id: id.to_string(),
        role,
        profile,
        depends_on: Vec::new(),
    })
}

fn build_reviewers(args: &GoalArgs, depends_on: Vec<String>) -> Result<Vec<WorkerRequest>> {
    let mut reviewers = Vec::new();
    for (index, raw) in args.critics.iter().enumerate() {
        let id = generated_role_id("critic", index);
        reviewers.push(parse_review_worker(raw, &id, &depends_on)?);
    }
    for (index, raw) in args.reviewers.iter().enumerate() {
        let id = generated_role_id("reviewer", index);
        reviewers.push(parse_review_worker(raw, &id, &depends_on)?);
    }
    Ok(reviewers)
}

fn parse_review_worker(
    raw: &str,
    generated_id: &str,
    depends_on: &[String],
) -> Result<WorkerRequest> {
    let default_role = |id: &str| default_goal_role("reviewer", id);
    let mut worker = if raw.contains(':') {
        parse_worker_with_default(raw, default_role)?
    } else {
        WorkerRequest {
            id: generated_id.to_string(),
            role: default_role(generated_id)?,
            profile: non_empty(raw).map(str::to_string),
            depends_on: Vec::new(),
        }
    };
    worker.depends_on = depends_on.to_vec();
    Ok(worker)
}

fn default_goal_role(kind: &str, worker_id: &str) -> Result<String> {
    apxm_backends::render_prompt(
        TEMPLATE_GOAL_WORKER_ROLE,
        &json!({
            "kind": kind,
            "worker_id": worker_id
        }),
    )
    .map_err(|error| anyhow!("failed to render goal worker role template: {error}"))
}

fn generated_role_id(prefix: &str, index: usize) -> String {
    if index == 0 {
        prefix.to_string()
    } else {
        format!("{prefix}{}", index + 1)
    }
}

fn terminal_worker_ids(workers: &[WorkerRequest]) -> Vec<String> {
    let depended_on = workers
        .iter()
        .flat_map(|worker| worker.depends_on.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    let terminals = workers
        .iter()
        .filter(|worker| !depended_on.contains(worker.id.as_str()))
        .map(|worker| worker.id.clone())
        .collect::<Vec<_>>();
    if terminals.is_empty() {
        workers.iter().map(|worker| worker.id.clone()).collect()
    } else {
        terminals
    }
}

fn ensure_unique_worker_ids(workers: &[WorkerRequest]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for worker in workers {
        if !ids.insert(worker.id.as_str()) {
            bail!("duplicate worker id '{}'", worker.id);
        }
    }
    Ok(())
}

fn apply_known_dependencies(workers: &mut [WorkerRequest], deps: &HashMap<String, Vec<String>>) {
    for (worker_id, depends_on) in deps {
        if let Some(worker) = workers.iter_mut().find(|worker| worker.id == *worker_id) {
            worker.depends_on = depends_on.clone();
        }
    }
}

fn apply_dependencies(
    workers: &mut [WorkerRequest],
    deps: HashMap<String, Vec<String>>,
) -> Result<()> {
    for (worker_id, depends_on) in deps {
        let worker = workers
            .iter_mut()
            .find(|worker| worker.id == worker_id)
            .ok_or_else(|| anyhow!("--depends references unknown worker '{worker_id}'"))?;
        worker.depends_on = depends_on;
    }
    Ok(())
}

fn parse_dependencies(specs: &[String]) -> Result<HashMap<String, Vec<String>>> {
    let mut deps = HashMap::new();
    for spec in specs {
        let (worker, raw_deps) = spec
            .split_once('=')
            .ok_or_else(|| anyhow!("--depends must be WORKER=DEP1,DEP2, got '{spec}'"))?;
        let worker = non_empty(worker)
            .ok_or_else(|| anyhow!("--depends worker id must not be empty: '{spec}'"))?;
        let depends_on = raw_deps
            .split(',')
            .filter_map(non_empty)
            .map(str::to_string)
            .collect::<Vec<_>>();
        deps.insert(worker.to_string(), depends_on);
    }
    Ok(deps)
}

fn workspace_json(args: &GoalArgs) -> Result<JsonValue> {
    let mode = match non_empty(args.workspace.as_str()) {
        Some(raw) => raw
            .parse::<OrchestrationWorkspaceMode>()
            .map_err(|_| anyhow!("--workspace must be one of: session, shared, git_worktree"))?,
        None => OrchestrationWorkspaceMode::Session,
    };

    let mut workspace = JsonMap::new();
    workspace.insert(
        "mode".to_string(),
        JsonValue::String(mode.as_str().to_string()),
    );
    workspace.insert(
        "cleanup".to_string(),
        JsonValue::String(OrchestrationWorkspaceCleanup::Keep.as_str().to_string()),
    );

    let repo_root = match (&args.repo_root, mode) {
        (Some(path), _) => Some(path.clone()),
        (None, OrchestrationWorkspaceMode::GitWorktree) => Some(
            std::env::current_dir()
                .context("failed to resolve current directory for --workspace git_worktree")?,
        ),
        _ => None,
    };
    if let Some(repo_root) = repo_root {
        workspace.insert(
            "repo_root".to_string(),
            JsonValue::String(path_to_string(repo_root)),
        );
    }
    if mode == OrchestrationWorkspaceMode::GitWorktree {
        workspace.insert(
            "base_ref".to_string(),
            JsonValue::String(args.base_ref.clone()),
        );
    }
    Ok(JsonValue::Object(workspace))
}

fn worker_to_json(worker: &WorkerRequest) -> JsonValue {
    let mut value = JsonMap::new();
    value.insert("id".to_string(), JsonValue::String(worker.id.clone()));
    value.insert("role".to_string(), JsonValue::String(worker.role.clone()));
    value.insert(
        "depends_on".to_string(),
        JsonValue::Array(
            worker
                .depends_on
                .iter()
                .cloned()
                .map(JsonValue::String)
                .collect(),
        ),
    );
    if let Some(profile) = &worker.profile {
        value.insert("profile".to_string(), JsonValue::String(profile.clone()));
        value.insert(
            "transport".to_string(),
            JsonValue::String(OrchestrationTransport::Acp.as_str().to_string()),
        );
    } else {
        value.insert(
            "transport".to_string(),
            JsonValue::String(OrchestrationTransport::Deterministic.as_str().to_string()),
        );
    }
    JsonValue::Object(value)
}

async fn call_mcp_tool(
    client: &reqwest::Client,
    base: &str,
    tool_name: &str,
    arguments: JsonValue,
) -> Result<JsonValue> {
    let url = format!("{}{}", base.trim_end_matches('/'), mcp_constants::ROUTE);
    let body = json!({
        (jsonrpc::JSONRPC): jsonrpc::VERSION,
        (jsonrpc::ID): 1,
        (jsonrpc::METHOD): mcp_constants::methods::TOOLS_CALL,
        (jsonrpc::PARAMS): {
            (mcp_fields::NAME): tool_name,
            (mcp_fields::ARGUMENTS): arguments,
        }
    });
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("failed to POST {url}"))?;
    let status = response.status();
    let payload = response
        .json::<JsonValue>()
        .await
        .with_context(|| format!("failed to decode MCP response from {url}"))?;
    if !status.is_success() {
        bail!("MCP HTTP {status}: {payload}");
    }
    if let Some(error) = payload.get(jsonrpc::ERROR) {
        bail!("MCP JSON-RPC error: {error}");
    }

    let result = payload
        .get(jsonrpc::RESULT)
        .ok_or_else(|| anyhow!("MCP response missing result: {payload}"))?;
    let text = result
        .get(mcp_fields::CONTENT)
        .and_then(JsonValue::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get(mcp_fields::TEXT))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| anyhow!("MCP tool result missing text content: {payload}"))?;
    if result
        .get(mcp_fields::IS_ERROR)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        bail!("{tool_name}: {text}");
    }
    serde_json::from_str(text).with_context(|| format!("{tool_name} returned non-JSON text"))
}

async fn goal_status(client: &reqwest::Client, base: &str, goal_id: &str) -> Result<JsonValue> {
    call_mcp_tool(
        client,
        base,
        mcp_tools::APXM_GOAL_STATUS,
        json!({ "goal_id": goal_id }),
    )
    .await
}

async fn goal_events(
    client: &reqwest::Client,
    base: &str,
    goal_id: &str,
    since: u64,
    limit: usize,
) -> Result<JsonValue> {
    call_mcp_tool(
        client,
        base,
        mcp_tools::APXM_GOAL_EVENTS,
        json!({
            "goal_id": goal_id,
            "since": since,
            "limit": limit,
        }),
    )
    .await
}

async fn follow_goal(
    client: &reqwest::Client,
    base: &str,
    goal_id: &str,
    limit: usize,
    timeout: Option<Duration>,
    render: bool,
) -> Result<FollowResult> {
    let mut events_seen = 0usize;
    let mut terminal_event_kind = None;
    let started_at = Instant::now();

    if render {
        println!("following goal: {goal_id}");
    }

    let mut response = client
        .get(goal_events_stream_url(base, goal_id, 0, limit))
        .header("Accept", "text/event-stream")
        .send()
        .await
        .context("failed to open goal event stream")?;
    if !response.status().is_success() {
        bail!(
            "goal event stream returned {} for {goal_id}",
            response.status()
        );
    }

    let mut parser = GoalSseParser::default();
    loop {
        let chunk =
            next_goal_stream_chunk(&mut response, timeout, started_at, client, base, goal_id)
                .await?;
        let Some(chunk) = chunk else {
            let status = goal_status(client, base, goal_id).await?;
            if status_is_terminal(&status) {
                return Ok(FollowResult {
                    events_seen,
                    terminal_event_kind,
                    status,
                });
            }
            bail!("goal event stream ended before terminal status for {goal_id}");
        };

        for frame in parser.feed(&chunk) {
            if let Some(event) = decode_goal_sse_frame(&frame) {
                events_seen += 1;
                if render
                    && let Some(line) = summarize_event(&event) {
                        println!("{line}");
                    }
                if event_is_goal_terminal(&event, goal_id) {
                    terminal_event_kind = event_kind(&event).map(str::to_string);
                    let status = goal_status(client, base, goal_id).await?;
                    return Ok(FollowResult {
                        events_seen,
                        terminal_event_kind,
                        status,
                    });
                }
            }
        }
    }
}

async fn next_goal_stream_chunk(
    response: &mut reqwest::Response,
    timeout: Option<Duration>,
    started_at: Instant,
    client: &reqwest::Client,
    base: &str,
    goal_id: &str,
) -> Result<Option<Vec<u8>>> {
    if let Some(timeout) = timeout {
        let Some(remaining) = timeout.checked_sub(started_at.elapsed()) else {
            let status = goal_status(client, base, goal_id).await?;
            bail!(
                "timed out while following {goal_id}; latest status: {}",
                status["status"]
            );
        };
        match tokio::time::timeout(remaining, response.chunk()).await {
            Ok(chunk) => chunk
                .map(|maybe_chunk| maybe_chunk.map(|chunk| chunk.to_vec()))
                .context("goal event stream read failed"),
            Err(_) => {
                let status = goal_status(client, base, goal_id).await?;
                bail!(
                    "timed out while following {goal_id}; latest status: {}",
                    status["status"]
                );
            }
        }
    } else {
        response
            .chunk()
            .await
            .map(|maybe_chunk| maybe_chunk.map(|chunk| chunk.to_vec()))
            .context("goal event stream read failed")
    }
}

fn goal_events_stream_url(base: &str, goal_id: &str, since: u64, limit: usize) -> String {
    format!(
        "{}/v1/goals/{}/events/stream?since={}&limit={}",
        base.trim_end_matches('/'),
        goal_id,
        since,
        limit
    )
}

#[derive(Debug, Clone, Default)]
struct GoalSseFrame {
    data: String,
}

#[derive(Debug, Default)]
struct GoalSseParser {
    buf: String,
    current: GoalSseFrame,
    has_current: bool,
}

impl GoalSseParser {
    fn feed(&mut self, bytes: &[u8]) -> Vec<GoalSseFrame> {
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
            if let Some((field, value)) = raw_line.split_once(':') {
                self.has_current = true;
                if field == "data" {
                    let value = value.strip_prefix(' ').unwrap_or(value);
                    if !self.current.data.is_empty() {
                        self.current.data.push('\n');
                    }
                    self.current.data.push_str(value);
                }
            }
        }
        out
    }
}

fn decode_goal_sse_frame(frame: &GoalSseFrame) -> Option<JsonValue> {
    if frame.data.is_empty() {
        return None;
    }
    serde_json::from_str(&frame.data).ok()
}

fn event_is_goal_terminal(event: &JsonValue, goal_id: &str) -> bool {
    let is_aggregate_event = event
        .get("meta")
        .and_then(|meta| meta.get("trace_id"))
        .and_then(JsonValue::as_str)
        == Some(goal_id);
    if !is_aggregate_event {
        return false;
    }
    matches!(
        event_kind(event),
        Some(name)
            if name == event_kind_constants::ORCHESTRATOR_WAKE.name()
                || name == event_kind_constants::ERROR.name()
                || name == event_kind_constants::TURN_ABORTED.name()
    )
}

fn status_is_terminal(status: &JsonValue) -> bool {
    matches!(
        status
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or_default(),
        goal_execution_status::SUCCEEDED | goal_execution_status::FAILED | "cancelled"
    )
}

fn print_start_summary(base: &str, started: &JsonValue) {
    let status = started
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    println!("goal status: {status}");
    if let Some(goal_id) = started.get("goal_id").and_then(JsonValue::as_str) {
        println!("goal: {goal_id}");
    }
    if let Some(execution_id) = started.get("execution_id").and_then(JsonValue::as_str) {
        println!("current execution: {execution_id}");
    }
    if let Some(session_id) = started.get("session_id").and_then(JsonValue::as_str) {
        println!("session: {session_id}");
    }
    if let Some(session_dir) = started.get("session_dir").and_then(JsonValue::as_str) {
        println!("session dir: {session_dir}");
    }
    if let Some(workflow_path) = started.get("workflow_path").and_then(JsonValue::as_str) {
        println!("workflow: {workflow_path}");
    }
    if let Some(artifacts) = started.get("artifacts") {
        if let Some(tracking_doc) = artifacts.get("tracking_doc").and_then(JsonValue::as_str) {
            println!("tracking: {tracking_doc}");
        }
        if let Some(worker_air_dir) = artifacts.get("worker_air_dir").and_then(JsonValue::as_str) {
            println!("worker AIR: {worker_air_dir}");
        }
        if let Some(gate_air) = artifacts.get("gate_air").and_then(JsonValue::as_str) {
            println!("gate AIR: {gate_air}");
        }
        if let Some(prompts_dir) = artifacts.get("prompts_dir").and_then(JsonValue::as_str) {
            println!("prompts: {prompts_dir}");
        }
        if let Some(reports_dir) = artifacts.get("reports_dir").and_then(JsonValue::as_str) {
            println!("reports: {reports_dir}");
        }
    }
    if let Some(workers) = started
        .get("plan")
        .and_then(|plan| plan.get("workers"))
        .and_then(JsonValue::as_array)
    {
        println!("workers:");
        for worker in workers {
            let id = worker
                .get("id")
                .and_then(JsonValue::as_str)
                .unwrap_or("<unknown>");
            let role = worker.get("role").and_then(JsonValue::as_str).unwrap_or("");
            let transport = worker
                .get("transport")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let profile = worker
                .get("profile")
                .and_then(JsonValue::as_str)
                .map(|value| format!(" profile={value}"))
                .unwrap_or_default();
            println!("  - {id}: {transport}{profile} - {role}");
        }
    }
    println!(
        "control: {base}{} ({}/{}/{})",
        mcp_constants::ROUTE,
        mcp_tools::APXM_GOAL_STATUS,
        mcp_tools::APXM_GOAL_EVENTS,
        mcp_tools::APXM_GOAL_CANCEL
    );
}

fn print_json_or_status(json_output: bool, status: JsonValue) -> Result<()> {
    if json_output {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        print_final_status(&status);
    }
    Ok(())
}

fn print_final_status(status: &JsonValue) {
    let goal_id = status
        .get("goal_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("<unknown>");
    let status_text = status
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    println!("final status: {status_text} ({goal_id})");
    if let Some(execution_id) = status
        .get("current_execution_id")
        .and_then(JsonValue::as_str)
    {
        println!("current execution: {execution_id}");
    }
    if let Some(session_dir) = status.get("session_dir").and_then(JsonValue::as_str) {
        println!("session dir: {session_dir}");
    }
    if let Some(goal) = status.get("goal")
        && let Some(reason) = goal
            .get("decision")
            .and_then(|decision| decision.get("reason"))
            .or_else(|| {
                goal.get("verdict")
                    .and_then(|verdict| verdict.get("reason"))
            })
            .and_then(JsonValue::as_str)
            .filter(|reason| !reason.is_empty())
        {
            println!("reason: {reason}");
        }
    if let Some(error) = status
        .get("error")
        .and_then(JsonValue::as_str)
        .filter(|error| !error.is_empty())
    {
        println!("error: {error}");
    }
}

fn summarize_event(event: &JsonValue) -> Option<String> {
    let event_name = event_kind(event)?;
    let payload = event.get("payload").unwrap_or(event);
    let seq = event_seq(event)
        .map(|seq| format!("#{seq} "))
        .unwrap_or_default();
    match event_name {
        name if name == event_kind_constants::ORCHESTRATOR_SLEEP.name() => Some(format!(
            "{seq}goal pass sleeping; runtime owns the workflow until wake"
        )),
        name if name == event_kind_constants::ORCHESTRATOR_WAKE.name() => Some(format!(
            "{seq}goal wake: {} via {}",
            payload_str(payload, "outcome").unwrap_or("unknown"),
            payload_str(payload, "terminal_event").unwrap_or("terminal event")
        )),
        name if name == event_kind_constants::WORKFLOW_STARTED.name() => Some(format!(
            "{seq}workflow {} started ({} steps)",
            payload_str(payload, "workflow_name").unwrap_or("<workflow>"),
            payload_usize(payload, "step_count").map_or_else(|| "?".to_string(), |value| value.to_string())
        )),
        name if name == event_kind_constants::WORKFLOW_STEP_STARTED.name() => Some(format!(
            "{seq}step {} started",
            payload_str(payload, "step_id").unwrap_or("<step>")
        )),
        name if name == event_kind_constants::WORKFLOW_STEP_COMPLETED.name() => {
            let step = payload_str(payload, "step_id").unwrap_or("<step>");
            let status = payload_str(payload, "status").unwrap_or("unknown");
            let duration = payload_u64(payload, "duration_ms")
                .map(|value| format!(" in {value}ms"))
                .unwrap_or_default();
            let session = payload_str(payload, "session_dir")
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            Some(format!("{seq}step {step} {status}{duration}{session}"))
        }
        name if name == event_kind_constants::WORKFLOW_FINISHED.name() => Some(format!(
            "{seq}workflow {}{}",
            payload_str(payload, "status").unwrap_or("finished"),
            payload_u64(payload, "duration_ms")
                .map(|value| format!(" in {value}ms"))
                .unwrap_or_default()
        )),
        name if name == event_kind_constants::OPERATION_START.name() => {
            let spawn_agent = AISOperationType::SpawnAgent.to_string();
            let communicate = AISOperationType::Communicate.to_string();
            match payload_str(payload, "op_type") {
                Some(op_type) if op_type == spawn_agent.as_str() => Some(format!(
                    "{seq}spawn agent node {}",
                    payload_u64(payload, "node_id").map_or_else(|| "?".to_string(), |value| value.to_string())
                )),
                Some(op_type) if op_type == communicate.as_str() => Some(format!(
                    "{seq}communicate with worker node {}",
                    payload_u64(payload, "node_id").map_or_else(|| "?".to_string(), |value| value.to_string())
                )),
                _ => None,
            }
        }
        name if name == event_kind_constants::EXECUTE_COMPLETE.name() => {
            Some(format!("{seq}execution complete"))
        }
        name if name == event_kind_constants::TURN_ABORTED.name() => Some(format!(
            "{seq}run aborted: {}",
            payload_str(payload, "reason").unwrap_or("cancelled")
        )),
        name if name == event_kind_constants::ERROR.name() => Some(format!(
            "{seq}error: {}",
            payload_str(payload, "message").unwrap_or("unknown")
        )),
        name if name == event_kind_constants::GOAL_GATE_VERDICT.name() => Some(format!(
            "{seq}gate verdict: {} — {}",
            payload_str(payload, "status").unwrap_or("unknown"),
            payload_str(payload, "reason").unwrap_or("")
        )),
        name if name == event_kind_constants::GOAL_CONVERGED.name() => Some(format!(
            "{seq}goal converged: {}",
            payload_str(payload, "reason").unwrap_or("met")
        )),
        name if name == event_kind_constants::GOAL_NEEDS_ANOTHER_PASS.name() => Some(format!(
            "{seq}goal needs another pass (next iteration {}): {}",
            payload_u64(payload, "next_iteration").map_or_else(|| "?".to_string(), |value| value.to_string()),
            payload_str(payload, "reason").unwrap_or("")
        )),
        name if name == event_kind_constants::GOAL_HALTED.name() => Some(format!(
            "{seq}goal halted{}: {}",
            if payload.get("exhausted").and_then(JsonValue::as_bool) == Some(true) {
                " (pass budget exhausted)"
            } else {
                ""
            },
            payload_str(payload, "reason").unwrap_or("")
        )),
        _ => None,
    }
}

fn event_kind(event: &JsonValue) -> Option<&str> {
    event
        .get("payload")
        .and_then(|payload| payload.get("kind"))
        .and_then(JsonValue::as_str)
        .or_else(|| event.get("kind").and_then(JsonValue::as_str))
}

fn event_seq(event: &JsonValue) -> Option<u64> {
    event
        .get("meta")
        .and_then(|meta| meta.get("seq"))
        .and_then(JsonValue::as_u64)
}

fn payload_str<'a>(payload: &'a JsonValue, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(JsonValue::as_str)
}

fn payload_u64(payload: &JsonValue, key: &str) -> Option<u64> {
    payload.get(key).and_then(JsonValue::as_u64)
}

fn payload_usize(payload: &JsonValue, key: &str) -> Option<usize> {
    payload_u64(payload, key).and_then(|value| usize::try_from(value).ok())
}

fn insert_optional_string(map: &mut JsonMap<String, JsonValue>, key: &str, value: Option<&str>) {
    if let Some(value) = value.and_then(non_empty) {
        map.insert(key.to_string(), JsonValue::String(value.to_string()));
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().to_string()
}
