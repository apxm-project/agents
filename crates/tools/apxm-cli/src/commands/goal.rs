//! `apxm goal` - user-facing bounded orchestration over APXM server MCP.
//!
//! The CLI stays thin: it builds a bounded worker plan, calls the server-owned
//! `apxm_orchestrate_start` tool, then follows the existing workflow
//! status/events/cancel tools by `execution_id`.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use apxm_core::constants::jsonrpc;
use apxm_core::constants::mcp::{self as mcp_constants, fields as mcp_fields, tools as mcp_tools};
use apxm_core::constants::orchestration::admission as orchestration_admission;
use apxm_core::constants::orchestration::execution_status as orchestration_execution_status;
use apxm_core::events::kind as event_kind_constants;
use apxm_core::types::OrchestrationWorkspaceMode;
use apxm_core::types::{AISOperationType, OrchestrationTransport, OrchestrationWorkspaceCleanup};
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use super::cli::GoalArgs;

const DEFAULT_SERVER_BASE: &str = "http://127.0.0.1:18800";
const TEMPLATE_ORCHESTRATION_GOAL_WORKER_ROLE: &str = "orchestration_goal_worker_role";

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
            // A goal is a bounded sequence of admitted passes. Each pass is an
            // ultracode-style fan-out; after it settles the runtime emits a
            // typed convergence decision, and this loop runs another admitted
            // pass only while the runtime asks for one (`iterate`). The runtime
            // bounds the loop via `max_iterations`, so it always terminates.
            let max_iterations = args.max_iterations.unwrap_or(1).max(1);
            let mut iteration = 0usize;
            let mut context_override: Option<String> = None;
            let mut passes: Vec<JsonValue> = Vec::new();

            loop {
                let request = build_start_arguments(
                    &args,
                    &task,
                    iteration,
                    max_iterations,
                    context_override.as_deref(),
                )?;
                let started =
                    call_mcp_tool(&client, &base, mcp_tools::APXM_ORCHESTRATE_START, request)
                        .await?;
                if !json_output {
                    if max_iterations > 1 {
                        println!("== goal pass {}/{} ==", iteration + 1, max_iterations);
                    }
                    print_start_summary(&base, &started);
                }

                let should_follow = !args.no_follow
                    && !args.dry_run
                    && started
                        .get("execution_id")
                        .and_then(JsonValue::as_str)
                        .is_some();
                if !should_follow {
                    if json_output {
                        passes.push(json!({ "start": started }));
                    }
                    break;
                }

                let execution_id = started
                    .get("execution_id")
                    .and_then(JsonValue::as_str)
                    .expect("checked execution_id")
                    .to_string();
                let terminal_kinds = terminal_kinds(&started);
                let follow = follow_goal(
                    &client,
                    &base,
                    &execution_id,
                    terminal_kinds,
                    args.limit,
                    Duration::from_millis(args.poll_ms),
                    args.timeout_secs.map(Duration::from_secs),
                    !json_output,
                )
                .await?;
                if !json_output {
                    print_final_status(&follow.status);
                }
                if json_output {
                    passes.push(json!({
                        "start": started,
                        "events_seen": follow.events_seen,
                        "terminal_event_kind": follow.terminal_event_kind,
                        "status": follow.status,
                    }));
                }

                match goal_loop_step(&follow.status, max_iterations) {
                    GoalLoopStep::Iterate {
                        next_iteration,
                        remaining,
                    } => {
                        if !json_output {
                            println!(
                                "goal: runtime requested another pass ({} item(s) remaining)",
                                remaining.len()
                            );
                        }
                        iteration = next_iteration;
                        context_override =
                            Some(next_pass_context(args.context.as_deref(), &remaining));
                    }
                    GoalLoopStep::Stop => break,
                }
            }

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({ "passes": passes }))?
                );
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
                mcp_tools::APXM_WORKFLOW_CANCEL,
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

fn build_start_arguments(
    args: &GoalArgs,
    task: &str,
    iteration: usize,
    max_iterations: usize,
    context_override: Option<&str>,
) -> Result<JsonValue> {
    let workers = build_workers(args)?;
    let uses_profiles =
        workers.iter().any(|worker| worker.profile.is_some()) || args.supervisor_profile.is_some();

    let mut root = JsonMap::new();
    root.insert("task".to_string(), JsonValue::String(task.to_string()));
    insert_optional_string(
        &mut root,
        "context",
        context_override.or(args.context.as_deref()),
    );
    root.insert(
        "iteration".to_string(),
        JsonValue::from(iteration as u64),
    );
    root.insert(
        "max_iterations".to_string(),
        JsonValue::from(max_iterations as u64),
    );
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
    if args.admit_spawn || uses_profiles {
        admit.insert(orchestration_admission::SPAWN_AGENT.to_string());
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
        TEMPLATE_ORCHESTRATION_GOAL_WORKER_ROLE,
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

async fn workflow_status(
    client: &reqwest::Client,
    base: &str,
    execution_id: &str,
) -> Result<JsonValue> {
    call_mcp_tool(
        client,
        base,
        mcp_tools::APXM_WORKFLOW_STATUS,
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
        mcp_tools::APXM_WORKFLOW_EVENTS,
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

/// One step of the bounded goal loop, derived from the runtime convergence
/// decision attached to a settled pass's status.
#[derive(Debug, PartialEq, Eq)]
enum GoalLoopStep {
    /// Run another admitted pass at `next_iteration`, carrying `remaining`.
    Iterate {
        next_iteration: usize,
        remaining: Vec<String>,
    },
    /// Stop: the goal converged, halted, or reported no decision.
    Stop,
}

/// Read the runtime goal decision off a terminal status and decide whether to
/// run another bounded pass. The runtime only emits `iterate` while the pass
/// budget allows it, but this also guards `next_iteration` against the ceiling
/// so the loop terminates even if the server contract drifts.
fn goal_loop_step(status: &JsonValue, max_iterations: usize) -> GoalLoopStep {
    let Some(goal) = status.get("goal") else {
        return GoalLoopStep::Stop;
    };
    let decision = goal.get("decision");
    let decision_kind = decision
        .and_then(|d| d.get("decision"))
        .and_then(JsonValue::as_str);
    if decision_kind != Some("iterate") {
        return GoalLoopStep::Stop;
    }
    let next_iteration = decision
        .and_then(|d| d.get("next_iteration"))
        .and_then(JsonValue::as_u64)
        .map(|value| value as usize);
    let Some(next_iteration) = next_iteration.filter(|next| *next < max_iterations) else {
        return GoalLoopStep::Stop;
    };
    let remaining = goal
        .get("verdict")
        .and_then(|verdict| verdict.get("remaining"))
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    GoalLoopStep::Iterate {
        next_iteration,
        remaining,
    }
}

/// Build the context for the next bounded pass: the original context plus the
/// remaining work items the gate verdict reported.
fn next_pass_context(base_context: Option<&str>, remaining: &[String]) -> String {
    let mut parts = Vec::new();
    if let Some(base) = base_context.and_then(non_empty) {
        parts.push(base.to_string());
    }
    if !remaining.is_empty() {
        parts.push(format!(
            "Remaining from the previous pass:\n- {}",
            remaining.join("\n- ")
        ));
    }
    parts.join("\n\n")
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
                event_kind_constants::ORCHESTRATOR_WAKE.name(),
                event_kind_constants::EXECUTE_COMPLETE.name(),
                event_kind_constants::ERROR.name(),
                event_kind_constants::TURN_ABORTED.name(),
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
        orchestration_execution_status::SUCCEEDED | orchestration_execution_status::FAILED
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
    if let Some(artifacts) = started.get("artifacts") {
        if let Some(tracking_doc) = artifacts.get("tracking_doc").and_then(JsonValue::as_str) {
            println!("tracking: {tracking_doc}");
        }
        if let Some(plan_json) = artifacts.get("plan_json").and_then(JsonValue::as_str) {
            println!("plan: {plan_json}");
        }
        if let Some(graph_json) = artifacts.get("graph_json").and_then(JsonValue::as_str) {
            println!("graph: {graph_json}");
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
        mcp_tools::APXM_WORKFLOW_STATUS,
        mcp_tools::APXM_WORKFLOW_EVENTS,
        mcp_tools::APXM_WORKFLOW_CANCEL
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
    let event_name = event_kind(event)?;
    let payload = event.get("payload").unwrap_or(event);
    let seq = event_seq(event)
        .map(|seq| format!("#{seq} "))
        .unwrap_or_default();
    match event_name {
        name if name == event_kind_constants::ORCHESTRATOR_SLEEP.name() => Some(format!(
            "{seq}orchestrator sleeping; runtime owns the workflow until wake"
        )),
        name if name == event_kind_constants::ORCHESTRATOR_WAKE.name() => Some(format!(
            "{seq}orchestrator wake: {} via {}",
            payload_str(payload, "outcome").unwrap_or("unknown"),
            payload_str(payload, "terminal_event").unwrap_or("terminal event")
        )),
        name if name == event_kind_constants::WORKFLOW_STARTED.name() => Some(format!(
            "{seq}workflow {} started ({} steps)",
            payload_str(payload, "workflow_name").unwrap_or("<workflow>"),
            payload_usize(payload, "step_count")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string())
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
                    payload_u64(payload, "node_id")
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "?".to_string())
                )),
                Some(op_type) if op_type == communicate.as_str() => Some(format!(
                    "{seq}communicate with worker node {}",
                    payload_u64(payload, "node_id")
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "?".to_string())
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
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

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
            max_iterations: None,
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
        let request = build_start_arguments(&args, "ship the thing", 0, 1, None).expect("request");
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
    fn build_start_arguments_carries_iteration_budget() {
        let args = args_with_task();
        let request = build_start_arguments(&args, "ship it", 2, 5, Some("carry")).expect("request");
        assert_eq!(request["iteration"], json!(2));
        assert_eq!(request["max_iterations"], json!(5));
        assert_eq!(request["context"], json!("carry"));
    }

    #[test]
    fn goal_loop_iterates_on_runtime_iterate_decision() {
        let status = json!({
            "goal": {
                "decision": { "decision": "iterate", "reason": "more", "next_iteration": 1 },
                "verdict": { "status": "needs_more", "remaining": ["finish auth", "rerun lint"] }
            }
        });
        assert_eq!(
            goal_loop_step(&status, 3),
            GoalLoopStep::Iterate {
                next_iteration: 1,
                remaining: vec!["finish auth".to_string(), "rerun lint".to_string()],
            }
        );
    }

    #[test]
    fn goal_loop_stops_on_converged_or_halted() {
        for decision in ["converged", "halted"] {
            let status = json!({ "goal": { "decision": { "decision": decision } } });
            assert_eq!(goal_loop_step(&status, 5), GoalLoopStep::Stop);
        }
        // No goal block at all (non-orchestration run) also stops.
        assert_eq!(goal_loop_step(&json!({}), 5), GoalLoopStep::Stop);
    }

    #[test]
    fn goal_loop_stops_when_next_iteration_hits_ceiling() {
        // Defensive: even if the runtime says iterate, never exceed the ceiling.
        let status = json!({
            "goal": { "decision": { "decision": "iterate", "next_iteration": 3 } }
        });
        assert_eq!(goal_loop_step(&status, 3), GoalLoopStep::Stop);
    }

    #[test]
    fn next_pass_context_appends_remaining() {
        let ctx = next_pass_context(Some("repo: /x"), &["do A".to_string(), "do B".to_string()]);
        assert!(ctx.contains("repo: /x"));
        assert!(ctx.contains("Remaining from the previous pass"));
        assert!(ctx.contains("- do A"));
        assert!(ctx.contains("- do B"));
        // No base context, no remaining → empty.
        assert_eq!(next_pass_context(None, &[]), "");
    }

    #[test]
    fn critic_profiles_opt_into_review_workers() {
        let mut args = args_with_task();
        args.critics = vec![
            "profile-review".to_string(),
            "security:Security review:profile-sec".to_string(),
        ];

        let request = build_start_arguments(&args, "ship the thing", 0, 1, None).expect("request");
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

        let request = build_start_arguments(&args, "ship the thing", 0, 1, None).expect("request");
        assert_eq!(
            request["workers"][0]["transport"],
            OrchestrationTransport::Acp.as_str()
        );
        assert_eq!(request["workers"][0]["profile"], "profile-a");
        assert_eq!(request["workers"][1]["profile"], "profile-b");
        assert_eq!(
            request["admit_capabilities"],
            json!([orchestration_admission::SPAWN_AGENT])
        );
    }

    #[tokio::test]
    async fn goal_follow_pages_events_until_wake() {
        let server = MockMcpServer::start(vec![
            ExpectedMcpCall::new(
                mcp_tools::APXM_ORCHESTRATE_START,
                Some(json!({ "task": "ship" })),
                json!({
                    "execution_id": "exec-1",
                    "orchestration": {
                        "terminal_event_kinds": [
                            event_kind_constants::ORCHESTRATOR_WAKE.name(),
                            event_kind_constants::EXECUTE_COMPLETE.name(),
                            event_kind_constants::ERROR.name(),
                            event_kind_constants::TURN_ABORTED.name()
                        ]
                    }
                }),
            ),
            ExpectedMcpCall::new(
                mcp_tools::APXM_WORKFLOW_EVENTS,
                Some(json!({ "execution_id": "exec-1", "since": 0, "limit": 100 })),
                json!({
                    "events": [
                        { "meta": { "seq": 0 }, "payload": { "kind": event_kind_constants::WORKFLOW_STARTED.name(), "workflow_name": "goal", "step_count": 1 } }
                    ],
                    "next_seq": 1
                }),
            ),
            ExpectedMcpCall::new(
                mcp_tools::APXM_WORKFLOW_STATUS,
                Some(json!({ "execution_id": "exec-1" })),
                json!({ "execution_id": "exec-1", "status": orchestration_execution_status::RUNNING }),
            ),
            ExpectedMcpCall::new(
                mcp_tools::APXM_WORKFLOW_EVENTS,
                Some(json!({ "execution_id": "exec-1", "since": 1, "limit": 100 })),
                json!({
                    "events": [
                        { "meta": { "seq": 1 }, "payload": { "kind": event_kind_constants::ORCHESTRATOR_WAKE.name(), "outcome": "done", "terminal_event": event_kind_constants::WORKFLOW_FINISHED.name() } }
                    ],
                    "next_seq": 2
                }),
            ),
            ExpectedMcpCall::new(
                mcp_tools::APXM_WORKFLOW_STATUS,
                Some(json!({ "execution_id": "exec-1" })),
                json!({ "execution_id": "exec-1", "status": orchestration_execution_status::SUCCEEDED }),
            ),
        ])
        .await;
        let client = reqwest::Client::builder().build().expect("client");

        let started = call_mcp_tool(
            &client,
            &server.base,
            mcp_tools::APXM_ORCHESTRATE_START,
            json!({ "task": "ship" }),
        )
        .await
        .expect("start");
        let follow = follow_goal(
            &client,
            &server.base,
            "exec-1",
            terminal_kinds(&started),
            100,
            Duration::from_millis(1),
            Some(Duration::from_secs(2)),
            false,
        )
        .await
        .expect("follow");

        assert_eq!(follow.events_seen, 2);
        assert_eq!(
            follow.terminal_event_kind.as_deref(),
            Some(event_kind_constants::ORCHESTRATOR_WAKE.name())
        );
        assert_eq!(
            follow.status["status"],
            orchestration_execution_status::SUCCEEDED
        );
        server.finish().await;
    }

    #[test]
    fn goal_event_summary_uses_error_payload_message() {
        let event = json!({
            "meta": { "seq": 7 },
            "payload": {
                "kind": event_kind_constants::ERROR.name(),
                "message": "worker failed validation"
            }
        });

        assert_eq!(
            summarize_event(&event).as_deref(),
            Some("#7 error: worker failed validation")
        );
    }

    #[tokio::test]
    async fn goal_status_events_and_cancel_call_native_workflow_tools() {
        let server = MockMcpServer::start(vec![
            ExpectedMcpCall::new(
                mcp_tools::APXM_WORKFLOW_STATUS,
                Some(json!({ "execution_id": "exec-2" })),
                json!({ "execution_id": "exec-2", "status": orchestration_execution_status::RUNNING }),
            ),
            ExpectedMcpCall::new(
                mcp_tools::APXM_WORKFLOW_EVENTS,
                Some(json!({ "execution_id": "exec-2", "since": 0, "limit": 50 })),
                json!({ "events": [], "next_seq": 0 }),
            ),
            ExpectedMcpCall::new(
                mcp_tools::APXM_WORKFLOW_CANCEL,
                Some(json!({ "execution_id": "exec-2" })),
                json!({ "execution_id": "exec-2", "status": "cancelling" }),
            ),
        ])
        .await;
        let client = reqwest::Client::builder().build().expect("client");

        let status = workflow_status(&client, &server.base, "exec-2")
            .await
            .expect("status");
        assert_eq!(status["status"], orchestration_execution_status::RUNNING);
        let events = workflow_events(&client, &server.base, "exec-2", 0, 50)
            .await
            .expect("events");
        assert_eq!(events["events"], json!([]));
        let cancelled = call_mcp_tool(
            &client,
            &server.base,
            mcp_tools::APXM_WORKFLOW_CANCEL,
            json!({ "execution_id": "exec-2" }),
        )
        .await
        .expect("cancel");
        assert_eq!(cancelled["status"], "cancelling");
        server.finish().await;
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

    struct ExpectedMcpCall {
        tool: &'static str,
        args: Option<JsonValue>,
        response: JsonValue,
    }

    impl ExpectedMcpCall {
        fn new(tool: &'static str, args: Option<JsonValue>, response: JsonValue) -> Self {
            Self {
                tool,
                args,
                response,
            }
        }
    }

    struct MockMcpServer {
        base: String,
        task: tokio::task::JoinHandle<()>,
    }

    impl MockMcpServer {
        async fn start(calls: Vec<ExpectedMcpCall>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let base = format!("http://{}", listener.local_addr().expect("local addr"));
            let calls = Arc::new(Mutex::new(VecDeque::from(calls)));
            let task_calls = Arc::clone(&calls);
            let task = tokio::spawn(async move {
                loop {
                    if task_calls.lock().expect("calls lock").is_empty() {
                        break;
                    }
                    let (mut stream, _) = listener.accept().await.expect("accept");
                    let request = read_http_json(&mut stream).await;
                    let expected = task_calls
                        .lock()
                        .expect("calls lock")
                        .pop_front()
                        .expect("expected call");
                    assert_eq!(request["params"]["name"], expected.tool);
                    if let Some(args) = expected.args {
                        assert_eq!(request["params"]["arguments"], args);
                    }
                    write_mcp_response(&mut stream, expected.response).await;
                }
            });
            Self { base, task }
        }

        async fn finish(self) {
            self.task.await.expect("mock server task");
        }
    }

    async fn read_http_json(stream: &mut tokio::net::TcpStream) -> JsonValue {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read = stream.read(&mut chunk).await.expect("read request");
            assert!(read > 0, "connection closed before complete request");
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(header_end) = http_header_end(&buffer) {
                let headers = std::str::from_utf8(&buffer[..header_end]).expect("headers utf8");
                let content_len = http_content_length(headers);
                if buffer.len() >= header_end + content_len {
                    let body = &buffer[header_end..header_end + content_len];
                    return serde_json::from_slice(body).expect("request JSON");
                }
            }
        }
    }

    async fn write_mcp_response(stream: &mut tokio::net::TcpStream, response: JsonValue) {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string(&response).expect("response text")
                    }
                ],
                "isError": false
            }
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    }

    fn http_header_end(buffer: &[u8]) -> Option<usize> {
        buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
    }

    fn http_content_length(headers: &str) -> usize {
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().expect("content-length"))
            })
            .expect("content-length header")
    }
}
