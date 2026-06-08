//! `apxm goal` - user-facing bounded orchestration over APXM server MCP.
//!
//! The CLI stays thin: it builds a bounded worker plan, calls the server-owned
//! `apxm_orchestrate_start` tool, then follows the existing workflow
//! status/events/cancel tools by `execution_id`.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use super::cli::GoalArgs;

const DEFAULT_SERVER_BASE: &str = "http://127.0.0.1:18800";
const MCP_PATH: &str = "/v1/mcp";
const TOOL_ORCHESTRATE_START: &str = "apxm_orchestrate_start";
const TOOL_WORKFLOW_STATUS: &str = "apxm_workflow_status";
const TOOL_WORKFLOW_EVENTS: &str = "apxm_workflow_events";
const TOOL_WORKFLOW_CANCEL: &str = "apxm_workflow_cancel";
const ADMIT_SPAWN_AGENT: &str = "SPAWN_AGENT";

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
            let request = build_start_arguments(&args, &task)?;
            let started = call_mcp_tool(&client, &base, TOOL_ORCHESTRATE_START, request).await?;
            if !json_output {
                print_start_summary(&base, &started);
            }

            let should_follow = !args.no_follow
                && !args.dry_run
                && started
                    .get("execution_id")
                    .and_then(JsonValue::as_str)
                    .is_some();
            let follow = if should_follow {
                let execution_id = started
                    .get("execution_id")
                    .and_then(JsonValue::as_str)
                    .expect("checked execution_id");
                let terminal_kinds = terminal_kinds(&started);
                Some(
                    follow_goal(
                        &client,
                        &base,
                        execution_id,
                        terminal_kinds,
                        args.limit,
                        Duration::from_millis(args.poll_ms),
                        args.timeout_secs.map(Duration::from_secs),
                        !json_output,
                    )
                    .await?,
                )
            } else {
                None
            };

            if json_output {
                let output = match follow {
                    Some(follow) => json!({
                        "start": started,
                        "events_seen": follow.events_seen,
                        "terminal_event_kind": follow.terminal_event_kind,
                        "status": follow.status,
                    }),
                    None => json!({ "start": started }),
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else if let Some(follow) = follow {
                print_final_status(&follow.status);
            }
        }
        GoalMode::Status(execution_id) => {
            let status = workflow_status(&client, &base, &execution_id).await?;
            print_json_or_status(json_output, status)?;
        }
        GoalMode::Events(execution_id) => {
            let events = workflow_events(&client, &base, &execution_id, 0, args.limit).await?;
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
        GoalMode::Cancel(execution_id) => {
            let cancelled = call_mcp_tool(
                &client,
                &base,
                TOOL_WORKFLOW_CANCEL,
                json!({ "execution_id": execution_id }),
            )
            .await?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&cancelled)?);
            } else {
                println!(
                    "cancelled: {}",
                    cancelled
                        .get("execution_id")
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
            "provide a goal task, or use exactly one of --status, --events, or --cancel with an execution id"
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

fn build_start_arguments(args: &GoalArgs, task: &str) -> Result<JsonValue> {
    let workers = build_workers(args)?;
    let uses_profiles =
        workers.iter().any(|worker| worker.profile.is_some()) || args.supervisor_profile.is_some();

    let mut root = JsonMap::new();
    root.insert("task".to_string(), JsonValue::String(task.to_string()));
    insert_optional_string(&mut root, "context", args.context.as_deref());
    insert_optional_string(&mut root, "event", args.event.as_deref());
    insert_optional_string(&mut root, "trigger", args.trigger.as_deref());
    insert_optional_string(&mut root, "session_id", args.session_id.as_deref());
    root.insert(
        "workers".to_string(),
        JsonValue::Array(workers.iter().map(worker_to_json).collect()),
    );
    root.insert("workspace".to_string(), workspace_json(args)?);

    if let Some(profile) = args
        .supervisor_profile
        .as_ref()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        root.insert(
            "supervisor".to_string(),
            json!({
                "id": "gate",
                "profile": profile,
                "transport": "acp",
            }),
        );
    }

    let mut admit = BTreeSet::new();
    for cap in &args.admit {
        if !cap.trim().is_empty() {
            admit.insert(cap.trim().to_string());
        }
    }
    if args.admit_spawn || uses_profiles {
        admit.insert(ADMIT_SPAWN_AGENT.to_string());
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

fn build_workers(args: &GoalArgs) -> Result<Vec<WorkerRequest>> {
    let deps = parse_dependencies(&args.depends)?;
    let mut workers = if args.workers.is_empty() {
        let mut workers = default_workers(args);
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

fn default_workers(args: &GoalArgs) -> Vec<WorkerRequest> {
    vec![
        WorkerRequest {
            id: "planner".to_string(),
            role: "Act as the planner/orchestrator: split the goal, define worker briefs, and set acceptance checks for this bounded pass.".to_string(),
            profile: args
                .planner_profile
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            depends_on: Vec::new(),
        },
        WorkerRequest {
            id: "executor".to_string(),
            role: "Execute the work described by the planner/orchestrator for this bounded pass.".to_string(),
            profile: args
                .executor_profile
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            depends_on: vec!["planner".to_string()],
        },
        WorkerRequest {
            id: "verifier".to_string(),
            role: "Verify the result and report evidence.".to_string(),
            profile: args
                .verifier_profile
                .as_deref()
                .and_then(non_empty)
                .map(str::to_string),
            depends_on: vec!["executor".to_string()],
        },
    ]
}

fn parse_worker(raw: &str) -> Result<WorkerRequest> {
    parse_worker_with_default(raw, |id| format!("Complete the '{id}' worker slice."))
}

fn parse_worker_with_default(
    raw: &str,
    default_role: impl FnOnce(&str) -> String,
) -> Result<WorkerRequest> {
    let mut parts = raw.splitn(3, ':');
    let id = parts
        .next()
        .and_then(non_empty)
        .ok_or_else(|| anyhow!("worker spec must start with an id"))?;
    let role = parts
        .next()
        .and_then(non_empty)
        .map(str::to_string)
        .unwrap_or_else(|| default_role(id));
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
    let default_role = |id: &str| {
        format!(
            "Review the goal output as '{id}', preserving risks, dissent, missing evidence, and follow-up recommendations."
        )
    };
    let mut worker = if raw.contains(':') {
        parse_worker_with_default(raw, default_role)?
    } else {
        WorkerRequest {
            id: generated_id.to_string(),
            role: default_role(generated_id),
            profile: non_empty(raw).map(str::to_string),
            depends_on: Vec::new(),
        }
    };
    worker.depends_on = depends_on.to_vec();
    Ok(worker)
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
    let mode = non_empty(args.workspace.as_str()).unwrap_or("session");
    if !matches!(mode, "session" | "shared" | "git_worktree") {
        bail!("--workspace must be one of: session, shared, git_worktree");
    }

    let mut workspace = JsonMap::new();
    workspace.insert("mode".to_string(), JsonValue::String(mode.to_string()));
    workspace.insert("cleanup".to_string(), JsonValue::String("keep".to_string()));

    let repo_root = match (&args.repo_root, mode) {
        (Some(path), _) => Some(path.clone()),
        (None, "git_worktree") => Some(
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
    if mode == "git_worktree" {
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
            JsonValue::String("acp".to_string()),
        );
    } else {
        value.insert(
            "transport".to_string(),
            JsonValue::String("deterministic".to_string()),
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
    let url = format!("{}{}", base.trim_end_matches('/'), MCP_PATH);
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments,
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
    if let Some(error) = payload.get("error") {
        bail!("MCP JSON-RPC error: {error}");
    }

    let result = payload
        .get("result")
        .ok_or_else(|| anyhow!("MCP response missing result: {payload}"))?;
    let text = result
        .get("content")
        .and_then(JsonValue::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| anyhow!("MCP tool result missing text content: {payload}"))?;
    if result
        .get("isError")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        bail!("{tool_name}: {text}");
    }
    serde_json::from_str(text).with_context(|| format!("{tool_name} returned non-JSON text"))
}

async fn workflow_status(
    client: &reqwest::Client,
    base: &str,
    execution_id: &str,
) -> Result<JsonValue> {
    call_mcp_tool(
        client,
        base,
        TOOL_WORKFLOW_STATUS,
        json!({ "execution_id": execution_id }),
    )
    .await
}

async fn workflow_events(
    client: &reqwest::Client,
    base: &str,
    execution_id: &str,
    since: u64,
    limit: usize,
) -> Result<JsonValue> {
    call_mcp_tool(
        client,
        base,
        TOOL_WORKFLOW_EVENTS,
        json!({
            "execution_id": execution_id,
            "since": since,
            "limit": limit,
        }),
    )
    .await
}

async fn follow_goal(
    client: &reqwest::Client,
    base: &str,
    execution_id: &str,
    terminal_kinds: BTreeSet<String>,
    limit: usize,
    poll_interval: Duration,
    timeout: Option<Duration>,
    render: bool,
) -> Result<FollowResult> {
    let mut since = 0;
    let mut events_seen = 0usize;
    let mut terminal_event_kind = None;
    let started_at = Instant::now();

    if render {
        println!("following events: {execution_id}");
    }

    loop {
        if let Some(timeout) = timeout {
            if started_at.elapsed() > timeout {
                let status = workflow_status(client, base, execution_id).await?;
                bail!(
                    "timed out while following {execution_id}; latest status: {}",
                    status["status"]
                );
            }
        }

        let page = workflow_events(client, base, execution_id, since, limit).await?;
        let events = page
            .get("events")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        if !events.is_empty() {
            for event in &events {
                events_seen += 1;
                if render {
                    if let Some(line) = summarize_event(event) {
                        println!("{line}");
                    }
                }
                if let Some(kind) = event_kind(event) {
                    if terminal_kinds.contains(kind) {
                        terminal_event_kind = Some(kind.to_string());
                    }
                }
            }
            since = page
                .get("next_seq")
                .and_then(JsonValue::as_u64)
                .unwrap_or_else(|| next_since_from_events(since, &events));
        }

        let status = workflow_status(client, base, execution_id).await?;
        if terminal_event_kind.is_some() || status_is_terminal(&status) {
            return Ok(FollowResult {
                events_seen,
                terminal_event_kind,
                status,
            });
        }

        tokio::time::sleep(poll_interval).await;
    }
}

fn next_since_from_events(current: u64, events: &[JsonValue]) -> u64 {
    events
        .iter()
        .filter_map(event_seq)
        .max()
        .map_or(current, |seq| seq.saturating_add(1))
}

fn terminal_kinds(started: &JsonValue) -> BTreeSet<String> {
    started
        .get("orchestration")
        .and_then(|value| value.get("terminal_event_kinds"))
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| {
            [
                "orchestrator_wake",
                "execute_complete",
                "error",
                "turn_aborted",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        })
}

fn status_is_terminal(status: &JsonValue) -> bool {
    matches!(
        status
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or_default(),
        "succeeded" | "failed"
    )
}

fn print_start_summary(base: &str, started: &JsonValue) {
    let status = started
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    println!("goal status: {status}");
    if let Some(execution_id) = started.get("execution_id").and_then(JsonValue::as_str) {
        println!("execution: {execution_id}");
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
        "control: {base}{MCP_PATH} ({TOOL_WORKFLOW_STATUS}/{TOOL_WORKFLOW_EVENTS}/{TOOL_WORKFLOW_CANCEL})"
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
    let execution_id = status
        .get("execution_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("<unknown>");
    let status_text = status
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    println!("final status: {status_text} ({execution_id})");
    if let Some(session_dir) = status.get("session_dir").and_then(JsonValue::as_str) {
        println!("session dir: {session_dir}");
    }
    if let Some(content) = status
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(JsonValue::as_str)
        .filter(|content| !content.is_empty())
    {
        println!("result: {content}");
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
    let kind = event_kind(event)?;
    let payload = event.get("payload").unwrap_or(event);
    let seq = event_seq(event)
        .map(|seq| format!("#{seq} "))
        .unwrap_or_default();
    match kind {
        "orchestrator_sleep" => Some(format!(
            "{seq}orchestrator sleeping; APXM owns the workflow until wake"
        )),
        "orchestrator_wake" => Some(format!(
            "{seq}orchestrator wake: {} via {}",
            payload_str(payload, "outcome").unwrap_or("unknown"),
            payload_str(payload, "terminal_event").unwrap_or("terminal event")
        )),
        "workflow_started" => Some(format!(
            "{seq}workflow {} started ({} steps)",
            payload_str(payload, "workflow_name").unwrap_or("<workflow>"),
            payload_usize(payload, "step_count")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string())
        )),
        "workflow_step_started" => Some(format!(
            "{seq}step {} started",
            payload_str(payload, "step_id").unwrap_or("<step>")
        )),
        "workflow_step_completed" => {
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
        "workflow_finished" => Some(format!(
            "{seq}workflow {}{}",
            payload_str(payload, "status").unwrap_or("finished"),
            payload_u64(payload, "duration_ms")
                .map(|value| format!(" in {value}ms"))
                .unwrap_or_default()
        )),
        "operation_start" => match payload_str(payload, "op_type") {
            Some("SPAWN_AGENT") => Some(format!(
                "{seq}spawn agent node {}",
                payload_u64(payload, "node_id")
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".to_string())
            )),
            Some("COMMUNICATE") => Some(format!(
                "{seq}communicate with worker node {}",
                payload_u64(payload, "node_id")
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".to_string())
            )),
            _ => None,
        },
        "execute_complete" => Some(format!("{seq}execution complete")),
        "turn_aborted" => Some(format!(
            "{seq}run aborted: {}",
            payload_str(payload, "reason").unwrap_or("cancelled")
        )),
        "error" => Some(format!(
            "{seq}error: {}",
            payload_str(payload, "error").unwrap_or("unknown")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args_with_task() -> GoalArgs {
        GoalArgs {
            task: Some("ship the thing".to_string()),
            status: None,
            events: None,
            cancel: None,
            server: None,
            session_id: None,
            context: None,
            event: None,
            trigger: None,
            workers: Vec::new(),
            depends: Vec::new(),
            planner_profile: None,
            executor_profile: None,
            critics: Vec::new(),
            reviewers: Vec::new(),
            verifier_profile: None,
            supervisor_profile: None,
            workspace: "session".to_string(),
            repo_root: None,
            base_ref: "HEAD".to_string(),
            admit: Vec::new(),
            admit_spawn: false,
            import: Vec::new(),
            dry_run: false,
            no_follow: false,
            limit: 100,
            poll_ms: 500,
            timeout_secs: None,
        }
    }

    #[test]
    fn goal_mode_requires_exactly_one_action() {
        let args = args_with_task();
        assert_eq!(
            resolve_mode(&args).expect("mode"),
            GoalMode::Start("ship the thing".to_string())
        );

        let mut invalid = args_with_task();
        invalid.status = Some("exec".to_string());
        assert!(resolve_mode(&invalid).is_err());
    }

    #[test]
    fn default_goal_workers_are_minimal() {
        let args = args_with_task();
        let request = build_start_arguments(&args, "ship the thing").expect("request");
        let workers = request["workers"].as_array().expect("workers");
        assert_eq!(
            workers
                .iter()
                .filter_map(|worker| worker["id"].as_str())
                .collect::<Vec<_>>(),
            vec!["planner", "executor", "verifier"]
        );
        assert_eq!(workers[1]["depends_on"], json!(["planner"]));
        assert_eq!(workers[2]["depends_on"], json!(["executor"]));
    }

    #[test]
    fn critic_profiles_opt_into_review_workers() {
        let mut args = args_with_task();
        args.critics = vec![
            "profile-review".to_string(),
            "security:Security review:profile-sec".to_string(),
        ];

        let request = build_start_arguments(&args, "ship the thing").expect("request");
        let workers = request["workers"].as_array().expect("workers");
        assert_eq!(
            workers
                .iter()
                .filter_map(|worker| worker["id"].as_str())
                .collect::<Vec<_>>(),
            vec!["planner", "executor", "critic", "security", "verifier"]
        );
        assert_eq!(workers[2]["profile"], "profile-review");
        assert_eq!(workers[2]["depends_on"], json!(["executor"]));
        assert_eq!(workers[3]["profile"], "profile-sec");
        assert_eq!(workers[3]["role"], "Security review");
        assert_eq!(
            workers[4]["depends_on"],
            json!(["executor", "critic", "security"])
        );
    }

    #[test]
    fn profiles_auto_grant_spawn_agent() {
        let mut args = args_with_task();
        args.planner_profile = Some("profile-a".to_string());
        args.executor_profile = Some("profile-b".to_string());

        let request = build_start_arguments(&args, "ship the thing").expect("request");
        assert_eq!(request["workers"][0]["transport"], "acp");
        assert_eq!(request["workers"][0]["profile"], "profile-a");
        assert_eq!(request["workers"][1]["profile"], "profile-b");
        assert_eq!(request["admit_capabilities"], json!(["SPAWN_AGENT"]));
    }

    #[test]
    fn parses_custom_worker_and_dependencies() {
        let mut args = args_with_task();
        args.workers = vec![
            "research:Research the API:profile-a".to_string(),
            "build:Implement it:profile-b".to_string(),
        ];
        args.depends = vec!["build=research".to_string()];

        let workers = build_workers(&args).expect("workers");
        assert_eq!(workers[0].id, "research");
        assert_eq!(workers[0].profile.as_deref(), Some("profile-a"));
        assert_eq!(workers[1].depends_on, vec!["research"]);
    }

    #[test]
    fn custom_workers_can_add_repeatable_reviewers() {
        let mut args = args_with_task();
        args.workers = vec![
            "research:Research the API:profile-a".to_string(),
            "build:Implement it:profile-b".to_string(),
        ];
        args.depends = vec!["build=research".to_string()];
        args.reviewers = vec!["review:Review implementation:profile-review".to_string()];

        let workers = build_workers(&args).expect("workers");
        let reviewer = workers.iter().find(|worker| worker.id == "review").unwrap();
        assert_eq!(reviewer.profile.as_deref(), Some("profile-review"));
        assert_eq!(reviewer.depends_on, vec!["build"]);
    }
}
