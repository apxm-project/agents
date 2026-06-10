//! Native goal-start MCP entry point.
//!
//! This layer is intentionally a compiler/materializer, not a second runtime:
//! it turns a bounded worker plan into a generated workflow bundle and then
//! starts that bundle through `apxm_workflow_start`, so status/events/cancel
//! stay on the existing workflow control plane.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use apxm_core::constants::mcp::tools as mcp_tool_names;
use apxm_core::constants::orchestration::admission as goal_admission;
use apxm_core::events::kind;
use apxm_core::paths::ApxmPaths;
use apxm_core::types::{
    CommunicateProtocol, OrchestrationStartStatus, OrchestrationTransport,
    OrchestrationWorkspaceCleanup, OrchestrationWorkspaceMode,
};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::error::ApiError;
use crate::helpers::mcp_tool_result;
use crate::state::AppState;

use super::workflow::{WorkflowOrchestrationContract, WorkflowStartArgs, start_workflow_from_args};

pub(crate) const MCP_TOOL_APXM_GOAL_START: &str = mcp_tool_names::APXM_GOAL_START;

const MAX_WORKERS: usize = 16;
const TEMPLATE_GOAL_WORKER: &str = "goal_worker";
const TEMPLATE_GOAL_SUPERVISOR: &str = "goal_supervisor";
const TEMPLATE_GOAL_TRACKING: &str = "goal_tracking";
const TEMPLATE_GOAL_CONTROLLER: &str = "goal_controller";
const TEMPLATE_GOAL_FLOWCHART: &str = "goal_flowchart";
const TEMPLATE_GOAL_REPORT_STUB: &str = "goal_report_stub";
const TEMPLATE_GOAL_DEFAULT_WORKER: &str = "goal_default_worker_instructions";
const TEMPLATE_GOAL_DEFAULT_SUPERVISOR: &str = "goal_default_supervisor_instructions";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalStartArgs {
    task: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
    workers: Vec<WorkerSpec>,
    #[serde(default)]
    supervisor: Option<SupervisorSpec>,
    #[serde(default)]
    workspace: Option<WorkspaceSpec>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    admit_capabilities: Vec<String>,
    #[serde(default)]
    imports: Vec<String>,
    /// Zero-based index of this bounded pass within a goal. Callers running a
    /// multi-pass goal increment this on each admitted pass.
    #[serde(default)]
    iteration: usize,
    /// Hard ceiling on the number of bounded passes for the goal. Defaults to a
    /// single pass; the runtime convergence decision is bounded by this.
    #[serde(default)]
    max_iterations: Option<usize>,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerSpec {
    id: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    transport: Option<OrchestrationTransport>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupervisorSpec {
    #[serde(default = "default_supervisor_id")]
    id: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    transport: Option<OrchestrationTransport>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceSpec {
    #[serde(default)]
    mode: Option<OrchestrationWorkspaceMode>,
    #[serde(default)]
    repo_root: Option<String>,
    #[serde(default)]
    base_ref: Option<String>,
    #[serde(default)]
    cleanup: Option<OrchestrationWorkspaceCleanup>,
}

#[derive(Debug, Serialize)]
struct GoalStartResponse {
    status: OrchestrationStartStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_id: Option<String>,
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_dir: Option<String>,
    workflow_path: String,
    bundle_dir: String,
    artifacts: GoalArtifacts,
    plan: GoalPlanSummary,
    control: GoalControl,
    goal: GoalRuntimeContract,
    sleep_wake: SleepWakeContract,
    goal_prompt: String,
    flowchart: String,
}

#[derive(Debug, Serialize)]
struct GoalPlanSummary {
    task: String,
    workers: Vec<WorkerPlanSummary>,
    supervisor: SupervisorPlanSummary,
    workspace_mode: OrchestrationWorkspaceMode,
}

#[derive(Debug, Clone, Serialize)]
struct GoalArtifacts {
    tracking_doc: String,
    graph_json: String,
    plan_json: String,
    prompts_dir: String,
    reports_dir: String,
    worker_prompts: Vec<WorkerPromptArtifact>,
    supervisor_prompt: String,
    supervisor_report: String,
}

#[derive(Debug, Clone, Serialize)]
struct WorkerPromptArtifact {
    id: String,
    prompt: String,
    report: String,
}

#[derive(Debug, Serialize)]
struct WorkerPlanSummary {
    id: String,
    role: String,
    transport: OrchestrationTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    depends_on: Vec<String>,
    cwd: String,
    workspace: WorkspaceBindingSummary,
}

#[derive(Debug, Serialize)]
struct SupervisorPlanSummary {
    id: String,
    transport: OrchestrationTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceBindingSummary {
    mode: OrchestrationWorkspaceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_commit: Option<String>,
    cleanup: OrchestrationWorkspaceCleanup,
}

#[derive(Debug, Serialize)]
struct GoalControl {
    status_tool: &'static str,
    events_tool: &'static str,
    cancel_tool: &'static str,
}

#[derive(Debug, Serialize)]
struct GoalRuntimeContract {
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_id: Option<String>,
    initial_since: u64,
    gate_step_id: String,
    feedback_step_id: &'static str,
    terminal_event_kinds: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_events_args: Option<JsonValue>,
    sleep_event_kind: &'static str,
    wake_event_kind: &'static str,
}

#[derive(Debug, Serialize)]
struct SleepWakeContract {
    sleep_after_start: bool,
    wake_on: Vec<String>,
    event_loop: &'static str,
}

struct GoalBundle {
    session_id: String,
    bundle_dir: PathBuf,
    workflow_path: PathBuf,
    plan: GoalPlan,
}

struct GoalPlan {
    task: String,
    workers: Vec<WorkerPlan>,
    supervisor: SupervisorPlan,
    workspace_mode: OrchestrationWorkspaceMode,
}

struct WorkerPlan {
    id: String,
    agent_name: String,
    role: String,
    prompt: String,
    profile: Option<String>,
    transport: OrchestrationTransport,
    depends_on: Vec<String>,
    mode: Option<String>,
    model: Option<String>,
    cwd: PathBuf,
    tracking_doc_path: PathBuf,
    graph_path: PathBuf,
    prompt_path: PathBuf,
    report_path: PathBuf,
    workspace: WorkspaceBinding,
}

struct SupervisorPlan {
    id: String,
    agent_name: String,
    prompt: String,
    profile: Option<String>,
    transport: OrchestrationTransport,
    mode: Option<String>,
    model: Option<String>,
    cwd: Option<PathBuf>,
    tracking_doc_path: PathBuf,
    graph_path: PathBuf,
    prompt_path: PathBuf,
    report_path: PathBuf,
}

struct WorkspaceBinding {
    mode: OrchestrationWorkspaceMode,
    worktree_ref: Option<String>,
    base_commit: Option<String>,
    cleanup: OrchestrationWorkspaceCleanup,
}

pub(crate) fn goal_start_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["task", "workers"],
        "properties": {
            "task": {
                "type": "string",
                "description": "Task label and instructions for one bounded goal pass"
            },
            "context": {
                "type": "string",
                "description": "Optional repository, product, or run context passed to workers"
            },
            "event": {
                "type": "string",
                "description": "Optional event or provenance that triggered this goal pass"
            },
            "trigger": {
                "type": "string",
                "description": "Optional trigger rule or reason for this goal pass"
            },
            "workers": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_WORKERS,
                "description": "Bounded worker graph for one goal pass. Independent workers run in parallel; depends_on creates fan-in/fan-out phases.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id"],
                    "properties": {
                        "id": { "type": "string", "description": "Stable file-safe worker id" },
                        "role": { "type": "string" },
                        "prompt": { "type": "string" },
                        "profile": {
                            "type": "string",
                            "description": "Optional ACP profile from the APXM agent registry"
                        },
                        "transport": {
                            "type": "string",
                            "enum": OrchestrationTransport::WIRE_VALUES,
                            "description": "acp spawns a real registered profile; deterministic writes a local fixture worker"
                        },
                        "depends_on": { "type": "array", "items": { "type": "string" } },
                        "mode": { "type": "string" },
                        "model": { "type": "string" }
                    }
                }
            },
            "supervisor": {
                "type": "object",
                "additionalProperties": false,
                "description": "Gatekeeper/evaluator step that runs after all workers finish",
                "properties": {
                    "id": { "type": "string" },
                    "prompt": { "type": "string" },
                    "profile": { "type": "string" },
                    "transport": { "type": "string", "enum": OrchestrationTransport::WIRE_VALUES },
                    "mode": { "type": "string" },
                    "model": { "type": "string" }
                }
            },
            "workspace": {
                "type": "object",
                "additionalProperties": false,
                "description": "Server-owned workspace allocation policy for spawned workers",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": OrchestrationWorkspaceMode::WIRE_VALUES,
                        "description": "session creates APXM-owned directories; git_worktree creates one detached Git worktree per worker; shared uses repo_root/current cwd"
                    },
                    "repo_root": { "type": "string", "description": "Required for git_worktree; optional for shared" },
                    "base_ref": { "type": "string", "description": "Git ref for git_worktree mode; defaults to HEAD" },
                    "cleanup": { "type": "string", "enum": OrchestrationWorkspaceCleanup::WIRE_VALUES, "description": "MVP keeps generated workspaces/worktrees for review" }
                }
            },
            "session_id": { "type": "string" },
            "admit_capabilities": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Must include SPAWN_AGENT when any worker/supervisor uses transport=acp"
            },
            "imports": { "type": "array", "items": { "type": "string" } },
            "iteration": {
                "type": "integer",
                "minimum": 0,
                "description": "Zero-based index of this bounded pass within a goal (default 0)"
            },
            "max_iterations": {
                "type": "integer",
                "minimum": 1,
                "description": "Hard ceiling on bounded passes for the goal; the runtime convergence decision is bounded by this (default 1)"
            },
            "dry_run": {
                "type": "boolean",
                "description": "Validate and materialize the workflow bundle without starting it"
            }
        }
    })
}

pub(crate) async fn call_goal_tool(
    state: &AppState,
    id: &JsonValue,
    tool_name: &str,
    tool_args: &JsonValue,
) -> Option<Json<JsonValue>> {
    if tool_name != MCP_TOOL_APXM_GOAL_START {
        return None;
    }
    Some(match goal_start(state, tool_args).await {
        Ok(response) => mcp_json_tool_result(id.clone(), response),
        Err(error) => mcp_tool_result(id.clone(), error.message, true),
    })
}

async fn goal_start(
    state: &AppState,
    tool_args: &JsonValue,
) -> Result<GoalStartResponse, ApiError> {
    let request: GoalStartArgs = serde_json::from_value(tool_args.clone())
        .map_err(|error| ApiError::bad_request(format!("invalid goal_start arguments: {error}")))?;
    let uses_process_spawns = goal_uses_process_spawns(&request);
    if uses_process_spawns
        && !request
            .admit_capabilities
            .iter()
            .any(|capability| capability == goal_admission::SPAWN_AGENT)
    {
        return Err(ApiError::bad_request(format!(
            "{MCP_TOOL_APXM_GOAL_START}: transport=acp requires admit_capabilities=[\"{}\"]",
            goal_admission::SPAWN_AGENT,
        )));
    }

    let bundle = materialize_goal_bundle(&request)?;
    let plan_summary = bundle.plan.summary();
    let artifacts = goal_artifacts(&bundle.bundle_dir, &bundle.plan);
    let control = goal_control();
    let wake_on = goal_wake_on();
    let event_loop = goal_event_loop();
    let goal_contract = WorkflowOrchestrationContract {
        bundle_dir: bundle.bundle_dir.to_string_lossy().to_string(),
        artifacts: serde_json::to_value(&artifacts).map_err(|error| {
            ApiError::internal_message(format!("failed to serialize goal artifacts: {error}"))
        })?,
        plan: serde_json::to_value(&plan_summary).map_err(|error| {
            ApiError::internal_message(format!("failed to serialize goal plan: {error}"))
        })?,
        control: serde_json::to_value(&control).map_err(|error| {
            ApiError::internal_message(format!("failed to serialize goal control: {error}"))
        })?,
        wake_on: wake_on.clone(),
        event_loop: event_loop.to_string(),
        iteration: request.iteration,
        max_iterations: request.max_iterations.unwrap_or(1).max(1),
    };
    let started = if request.dry_run {
        None
    } else {
        Some(
            start_workflow_from_args(
                state,
                WorkflowStartArgs {
                    workflow_path: bundle.workflow_path.to_string_lossy().to_string(),
                    args: JsonMap::new(),
                    session_id: Some(bundle.session_id.clone()),
                    admit_capabilities: request.admit_capabilities.clone(),
                    imports: request.imports.clone(),
                    orchestration: Some(goal_contract),
                },
            )
            .await?,
        )
    };

    let status = if started.is_some() {
        OrchestrationStartStatus::Running
    } else {
        OrchestrationStartStatus::Planned
    };
    let response = GoalStartResponse {
        status,
        execution_id: started
            .as_ref()
            .map(|response| response.execution_id.clone()),
        session_id: bundle.session_id.clone(),
        session_dir: started
            .as_ref()
            .map(|response| response.session_dir.clone()),
        workflow_path: bundle.workflow_path.to_string_lossy().to_string(),
        bundle_dir: bundle.bundle_dir.to_string_lossy().to_string(),
        artifacts,
        plan: plan_summary,
        control,
        goal: goal_runtime_contract(
            started
                .as_ref()
                .map(|response| response.execution_id.clone()),
            &bundle.plan.supervisor.id,
        ),
        sleep_wake: SleepWakeContract {
            sleep_after_start: !request.dry_run,
            wake_on,
            event_loop,
        },
        goal_prompt: goal_prompt()?,
        flowchart: goal_flowchart()?,
    };
    Ok(response)
}

fn materialize_goal_bundle(request: &GoalStartArgs) -> Result<GoalBundle, ApiError> {
    validate_request(request)?;
    let session_id = request
        .session_id
        .clone()
        .unwrap_or_else(|| format!("goal-{}", uuid::Uuid::new_v4()));
    validate_component_id(&session_id, "session_id")?;

    let paths = ApxmPaths::discover().map_err(|error| {
        ApiError::internal_message(format!("failed to resolve APXM paths: {error}"))
    })?;
    let root = paths.cache_component_dir("goals").map_err(|error| {
        ApiError::internal_message(format!("failed to create goal cache: {error}"))
    })?;
    let bundle_dir = root.join(&session_id);
    if bundle_dir.exists() {
        return Err(ApiError::bad_request(format!(
            "goal session already exists: {session_id}"
        )));
    }
    std::fs::create_dir_all(&bundle_dir).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create goal bundle '{}': {error}",
            bundle_dir.display()
        ))
    })?;

    let workspace_policy = WorkspacePolicy::from_spec(request.workspace.as_ref(), &bundle_dir)?;
    let plan = build_plan(request, &session_id, &bundle_dir, &workspace_policy)?;
    write_bundle_files(&bundle_dir, request, &plan)?;

    Ok(GoalBundle {
        session_id,
        workflow_path: bundle_dir.join("workflow.apxmw"),
        bundle_dir,
        plan,
    })
}

fn validate_request(request: &GoalStartArgs) -> Result<(), ApiError> {
    if request.task.trim().is_empty() {
        return Err(ApiError::bad_request("goal_start task must not be empty"));
    }
    if request.workers.is_empty() {
        return Err(ApiError::bad_request(
            "goal_start workers must be non-empty",
        ));
    }
    if request.workers.len() > MAX_WORKERS {
        return Err(ApiError::bad_request(format!(
            "goal_start workers exceeds breadth cap of {MAX_WORKERS}"
        )));
    }

    let mut ids = HashSet::new();
    for worker in &request.workers {
        validate_component_id(&worker.id, "worker.id")?;
        if !ids.insert(worker.id.as_str()) {
            return Err(ApiError::bad_request(format!(
                "goal_start duplicate worker id '{}'",
                worker.id
            )));
        }
        validate_transport(worker.transport, worker.profile.as_deref(), "worker")?;
        validate_optional_text(worker.role.as_deref(), "worker.role")?;
        validate_optional_text(worker.prompt.as_deref(), "worker.prompt")?;
        validate_optional_text(worker.mode.as_deref(), "worker.mode")?;
        validate_optional_text(worker.model.as_deref(), "worker.model")?;
    }
    if let Some(supervisor) = &request.supervisor {
        validate_component_id(&supervisor.id, "supervisor.id")?;
        if ids.contains(supervisor.id.as_str()) {
            return Err(ApiError::bad_request(format!(
                "goal_start supervisor id '{}' conflicts with a worker id",
                supervisor.id
            )));
        }
        validate_transport(
            supervisor.transport,
            supervisor.profile.as_deref(),
            "supervisor",
        )?;
        validate_optional_text(supervisor.prompt.as_deref(), "supervisor.prompt")?;
        validate_optional_text(supervisor.mode.as_deref(), "supervisor.mode")?;
        validate_optional_text(supervisor.model.as_deref(), "supervisor.model")?;
    }
    validate_dependencies(&request.workers)?;
    Ok(())
}

fn validate_transport(
    transport: Option<OrchestrationTransport>,
    profile: Option<&str>,
    owner: &str,
) -> Result<(), ApiError> {
    match transport.unwrap_or(if profile.is_some() {
        OrchestrationTransport::Acp
    } else {
        OrchestrationTransport::Deterministic
    }) {
        OrchestrationTransport::Acp => {
            if profile.is_none_or(str::is_empty) {
                return Err(ApiError::bad_request(format!(
                    "{owner} transport=acp requires a profile"
                )));
            }
        }
        OrchestrationTransport::Deterministic => {
            if profile.is_some() {
                return Err(ApiError::bad_request(format!(
                    "{owner} profile requires transport=acp"
                )));
            }
        }
    }
    Ok(())
}

fn validate_optional_text(value: Option<&str>, field: &str) -> Result<(), ApiError> {
    if value.is_some_and(|text| text.contains('\0')) {
        return Err(ApiError::bad_request(format!(
            "{field} must not contain NUL"
        )));
    }
    Ok(())
}

fn validate_dependencies(workers: &[WorkerSpec]) -> Result<(), ApiError> {
    let ids: HashSet<&str> = workers.iter().map(|worker| worker.id.as_str()).collect();
    for worker in workers {
        for dep in &worker.depends_on {
            if !ids.contains(dep.as_str()) {
                return Err(ApiError::bad_request(format!(
                    "worker '{}' depends_on unknown worker '{}'",
                    worker.id, dep
                )));
            }
            if dep == &worker.id {
                return Err(ApiError::bad_request(format!(
                    "worker '{}' cannot depend on itself",
                    worker.id
                )));
            }
        }
    }

    let by_id: HashMap<&str, &WorkerSpec> = workers
        .iter()
        .map(|worker| (worker.id.as_str(), worker))
        .collect();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for worker in workers {
        visit_worker(worker.id.as_str(), &by_id, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn visit_worker<'a>(
    id: &'a str,
    by_id: &HashMap<&'a str, &'a WorkerSpec>,
    visiting: &mut HashSet<&'a str>,
    visited: &mut HashSet<&'a str>,
) -> Result<(), ApiError> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id) {
        return Err(ApiError::bad_request(format!(
            "goal_start worker dependency cycle involving '{id}'"
        )));
    }
    let worker = by_id
        .get(id)
        .ok_or_else(|| ApiError::bad_request(format!("unknown worker '{id}'")))?;
    for dep in &worker.depends_on {
        visit_worker(dep.as_str(), by_id, visiting, visited)?;
    }
    visiting.remove(id);
    visited.insert(id);
    Ok(())
}

struct WorkspacePolicy {
    mode: OrchestrationWorkspaceMode,
    repo_root: Option<PathBuf>,
    base_ref: String,
    cleanup: OrchestrationWorkspaceCleanup,
    bundle_dir: PathBuf,
}

impl WorkspacePolicy {
    fn from_spec(spec: Option<&WorkspaceSpec>, bundle_dir: &Path) -> Result<Self, ApiError> {
        let mode = spec.and_then(|spec| spec.mode).unwrap_or_default();
        let cleanup = spec.and_then(|spec| spec.cleanup).unwrap_or_default();
        let base_ref = spec
            .and_then(|spec| spec.base_ref.as_deref())
            .unwrap_or("HEAD")
            .to_string();
        if base_ref.trim().is_empty() || base_ref.contains('\0') {
            return Err(ApiError::bad_request(
                "workspace.base_ref must not be empty or contain NUL",
            ));
        }
        let repo_root = spec
            .and_then(|spec| spec.repo_root.as_deref())
            .map(resolve_repo_root)
            .transpose()?;
        if mode == OrchestrationWorkspaceMode::GitWorktree && repo_root.is_none() {
            return Err(ApiError::bad_request(
                "workspace.repo_root is required for git_worktree mode",
            ));
        }
        Ok(Self {
            mode,
            repo_root,
            base_ref,
            cleanup,
            bundle_dir: bundle_dir.to_path_buf(),
        })
    }

    fn allocate(&self, worker_id: &str) -> Result<(PathBuf, WorkspaceBinding), ApiError> {
        match self.mode {
            OrchestrationWorkspaceMode::Session => {
                let cwd = self.bundle_dir.join("workspaces").join(worker_id);
                std::fs::create_dir_all(&cwd).map_err(|error| {
                    ApiError::internal_message(format!(
                        "failed to create worker workspace '{}': {error}",
                        cwd.display()
                    ))
                })?;
                Ok((
                    cwd,
                    WorkspaceBinding {
                        mode: OrchestrationWorkspaceMode::Session,
                        worktree_ref: None,
                        base_commit: None,
                        cleanup: self.cleanup,
                    },
                ))
            }
            OrchestrationWorkspaceMode::Shared => {
                let cwd = self.repo_root.clone().unwrap_or_else(|| {
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                });
                Ok((
                    cwd,
                    WorkspaceBinding {
                        mode: OrchestrationWorkspaceMode::Shared,
                        worktree_ref: None,
                        base_commit: None,
                        cleanup: self.cleanup,
                    },
                ))
            }
            OrchestrationWorkspaceMode::GitWorktree => {
                let repo_root = self.repo_root.as_ref().expect("validated repo_root");
                let cwd = self.bundle_dir.join("worktrees").join(worker_id);
                std::fs::create_dir_all(cwd.parent().expect("worktree parent")).map_err(
                    |error| {
                        ApiError::internal_message(format!(
                            "failed to create worktree parent for '{}': {error}",
                            cwd.display()
                        ))
                    },
                )?;
                create_git_worktree(repo_root, &cwd, &self.base_ref)?;
                let base_commit = git_rev_parse(&cwd, "HEAD")?;
                Ok((
                    cwd,
                    WorkspaceBinding {
                        mode: OrchestrationWorkspaceMode::GitWorktree,
                        worktree_ref: Some(self.base_ref.clone()),
                        base_commit: Some(base_commit),
                        cleanup: self.cleanup,
                    },
                ))
            }
        }
    }
}

fn build_plan(
    request: &GoalStartArgs,
    session_id: &str,
    bundle_dir: &Path,
    workspace_policy: &WorkspacePolicy,
) -> Result<GoalPlan, ApiError> {
    let mut workers = Vec::with_capacity(request.workers.len());
    let tracking_doc_path = bundle_dir.join("goal.md");
    let graph_path = bundle_dir.join("graph.json");
    for worker in &request.workers {
        let (cwd, workspace) = workspace_policy.allocate(&worker.id)?;
        let transport = effective_transport(worker.transport, worker.profile.as_deref());
        workers.push(WorkerPlan {
            id: worker.id.clone(),
            agent_name: agent_name(session_id, &worker.id),
            role: worker
                .role
                .clone()
                .unwrap_or_else(|| format!("worker {}", worker.id)),
            prompt: match worker.prompt.clone() {
                Some(prompt) => prompt,
                None => default_worker_prompt(&worker.id)?,
            },
            profile: worker.profile.clone(),
            transport,
            depends_on: worker.depends_on.clone(),
            mode: worker.mode.clone(),
            model: worker.model.clone(),
            cwd,
            tracking_doc_path: tracking_doc_path.clone(),
            graph_path: graph_path.clone(),
            prompt_path: bundle_dir.join("prompts").join(format!("{}.md", worker.id)),
            report_path: bundle_dir.join("reports").join(format!("{}.md", worker.id)),
            workspace,
        });
    }

    let supervisor_spec = request.supervisor.clone().unwrap_or(SupervisorSpec {
        id: default_supervisor_id(),
        prompt: None,
        profile: None,
        transport: None,
        mode: None,
        model: None,
    });
    let supervisor_transport = effective_transport(
        supervisor_spec.transport,
        supervisor_spec.profile.as_deref(),
    );
    let supervisor_cwd = if supervisor_transport == OrchestrationTransport::Acp {
        Some(bundle_dir.join("supervisor"))
    } else {
        None
    };
    if let Some(cwd) = &supervisor_cwd {
        std::fs::create_dir_all(cwd).map_err(|error| {
            ApiError::internal_message(format!(
                "failed to create supervisor workspace '{}': {error}",
                cwd.display()
            ))
        })?;
    }
    let supervisor_id = supervisor_spec.id.clone();

    Ok(GoalPlan {
        task: request.task.clone(),
        workers,
        supervisor: SupervisorPlan {
            agent_name: agent_name(session_id, &supervisor_id),
            id: supervisor_id.clone(),
            prompt: match supervisor_spec.prompt {
                Some(prompt) => prompt,
                None => default_supervisor_prompt()?,
            },
            profile: supervisor_spec.profile,
            transport: supervisor_transport,
            mode: supervisor_spec.mode,
            model: supervisor_spec.model,
            cwd: supervisor_cwd,
            tracking_doc_path,
            graph_path,
            prompt_path: bundle_dir
                .join("prompts")
                .join(format!("{supervisor_id}.md")),
            report_path: bundle_dir
                .join("reports")
                .join(format!("{supervisor_id}.md")),
        },
        workspace_mode: workspace_policy.mode.clone(),
    })
}

fn write_bundle_files(
    bundle_dir: &Path,
    request: &GoalStartArgs,
    plan: &GoalPlan,
) -> Result<(), ApiError> {
    let workers_dir = bundle_dir.join("workers");
    let prompts_dir = bundle_dir.join("prompts");
    let reports_dir = bundle_dir.join("reports");
    std::fs::create_dir_all(&workers_dir).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create workers dir '{}': {error}",
            workers_dir.display()
        ))
    })?;
    std::fs::create_dir_all(&prompts_dir).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create prompts dir '{}': {error}",
            prompts_dir.display()
        ))
    })?;
    std::fs::create_dir_all(&reports_dir).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create reports dir '{}': {error}",
            reports_dir.display()
        ))
    })?;
    for worker in &plan.workers {
        let air = worker_air(request, worker)?;
        std::fs::write(workers_dir.join(format!("{}.air", worker.id)), air).map_err(|error| {
            ApiError::internal_message(format!(
                "failed to write worker graph '{}': {error}",
                worker.id
            ))
        })?;
        write_text_file(
            &worker.prompt_path,
            &render_worker_prompt(request, worker)?,
            "worker prompt",
        )?;
        write_text_file(
            &worker.report_path,
            &render_report_stub(
                worker.id.as_str(),
                "worker",
                &worker.prompt_path,
                &worker.tracking_doc_path,
                &worker.graph_path,
            )?,
            "worker report stub",
        )?;
    }

    std::fs::write(bundle_dir.join("gate.air"), gate_air(request, plan)?).map_err(|error| {
        ApiError::internal_message(format!("failed to write gate graph: {error}"))
    })?;
    std::fs::write(bundle_dir.join("feedback.air"), feedback_air()).map_err(|error| {
        ApiError::internal_message(format!("failed to write feedback graph: {error}"))
    })?;
    std::fs::write(
        bundle_dir.join("workflow.apxmw"),
        workflow_json(request, plan)?,
    )
    .map_err(|error| ApiError::internal_message(format!("failed to write workflow: {error}")))?;
    write_json_file(
        &bundle_dir.join("plan.json"),
        &plan_packet_json(request, plan, bundle_dir)?,
        "goal plan packet",
    )?;
    write_json_file(
        &bundle_dir.join("graph.json"),
        &graph_packet_json(plan),
        "goal graph packet",
    )?;
    write_text_file(
        &bundle_dir.join("goal.md"),
        &tracking_doc(request, plan, bundle_dir)?,
        "goal tracking doc",
    )?;
    write_text_file(
        &plan.supervisor.prompt_path,
        &render_supervisor_prompt(request, plan, "")?,
        "supervisor prompt",
    )?;
    write_text_file(
        &plan.supervisor.report_path,
        &render_report_stub(
            plan.supervisor.id.as_str(),
            "gate",
            &plan.supervisor.prompt_path,
            &plan.supervisor.tracking_doc_path,
            &plan.supervisor.graph_path,
        )?,
        "supervisor report stub",
    )?;
    std::fs::write(bundle_dir.join("goal_prompt.txt"), goal_prompt()?).map_err(|error| {
        ApiError::internal_message(format!("failed to write goal prompt: {error}"))
    })?;
    Ok(())
}

fn write_text_file(path: &Path, contents: &str, label: &str) -> Result<(), ApiError> {
    std::fs::write(path, contents).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to write {label} '{}': {error}",
            path.display()
        ))
    })
}

fn write_json_file(path: &Path, value: &JsonValue, label: &str) -> Result<(), ApiError> {
    let contents = serde_json::to_vec_pretty(value).map_err(|error| {
        ApiError::internal_message(format!("failed to serialize {label}: {error}"))
    })?;
    std::fs::write(path, contents).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to write {label} '{}': {error}",
            path.display()
        ))
    })
}

fn plan_packet_json(
    request: &GoalStartArgs,
    plan: &GoalPlan,
    bundle_dir: &Path,
) -> Result<JsonValue, ApiError> {
    Ok(serde_json::json!({
        "task": request.task.as_str(),
        "context": request.context.as_deref().unwrap_or(""),
        "event": request.event.as_deref().unwrap_or(""),
        "trigger": request.trigger.as_deref().unwrap_or(""),
        "workspace_mode": plan.workspace_mode.as_str(),
        "bundle_dir": path_string(bundle_dir),
        "workflow_path": path_string(&bundle_dir.join("workflow.apxmw")),
        "tracking_doc": path_string(&bundle_dir.join("goal.md")),
        "graph_json": path_string(&bundle_dir.join("graph.json")),
        "control": goal_control(),
        "workers": plan
            .workers
            .iter()
            .map(worker_packet_json)
            .collect::<Vec<_>>(),
        "supervisor": supervisor_packet_json(&plan.supervisor)
    }))
}

fn worker_packet_json(worker: &WorkerPlan) -> JsonValue {
    serde_json::json!({
        "id": worker.id.as_str(),
        "role": worker.role.as_str(),
        "transport": worker.transport.as_str(),
        "profile": worker.profile.as_deref(),
        "depends_on": &worker.depends_on,
        "cwd": path_string(&worker.cwd),
        "prompt_path": path_string(&worker.prompt_path),
        "report_path": path_string(&worker.report_path),
        "tracking_doc": path_string(&worker.tracking_doc_path),
        "graph_json": path_string(&worker.graph_path),
        "workspace": workspace_binding_json(&worker.workspace)
    })
}

fn supervisor_packet_json(supervisor: &SupervisorPlan) -> JsonValue {
    serde_json::json!({
        "id": supervisor.id.as_str(),
        "transport": supervisor.transport.as_str(),
        "profile": supervisor.profile.as_deref(),
        "cwd": supervisor.cwd.as_ref().map(|path| path_string(path)),
        "prompt_path": path_string(&supervisor.prompt_path),
        "report_path": path_string(&supervisor.report_path),
        "tracking_doc": path_string(&supervisor.tracking_doc_path),
        "graph_json": path_string(&supervisor.graph_path)
    })
}

fn graph_packet_json(plan: &GoalPlan) -> JsonValue {
    let mut nodes = plan
        .workers
        .iter()
        .map(|worker| {
            serde_json::json!({
                "id": worker.id.as_str(),
                "kind": "worker",
                "depends_on": &worker.depends_on,
                "prompt_path": path_string(&worker.prompt_path),
                "report_path": path_string(&worker.report_path),
                "cwd": path_string(&worker.cwd)
            })
        })
        .collect::<Vec<_>>();
    let worker_ids = plan
        .workers
        .iter()
        .map(|worker| worker.id.clone())
        .collect::<Vec<_>>();
    nodes.push(serde_json::json!({
        "id": plan.supervisor.id.as_str(),
        "kind": "gate",
        "depends_on": worker_ids,
        "prompt_path": path_string(&plan.supervisor.prompt_path),
        "report_path": path_string(&plan.supervisor.report_path)
    }));
    nodes.push(serde_json::json!({
        "id": "feedback",
        "kind": "feedback",
        "depends_on": [plan.supervisor.id.clone()]
    }));

    serde_json::json!({
        "description": "event -> trigger -> parallel workers -> gate/eval -> feedback",
        "workspace_mode": plan.workspace_mode.as_str(),
        "nodes": nodes,
        "edges": graph_edges(plan)
    })
}

fn graph_edges(plan: &GoalPlan) -> Vec<JsonValue> {
    let mut edges = Vec::new();
    for worker in &plan.workers {
        for dep in &worker.depends_on {
            edges.push(serde_json::json!({
                "from": dep,
                "to": worker.id.as_str(),
                "reason": "depends_on"
            }));
        }
        edges.push(serde_json::json!({
            "from": worker.id.as_str(),
            "to": plan.supervisor.id.as_str(),
            "reason": "gate fan-in"
        }));
    }
    edges.push(serde_json::json!({
        "from": plan.supervisor.id.as_str(),
        "to": "feedback",
        "reason": "gate decision"
    }));
    edges
}

fn goal_artifacts(bundle_dir: &Path, plan: &GoalPlan) -> GoalArtifacts {
    GoalArtifacts {
        tracking_doc: path_string(&bundle_dir.join("goal.md")),
        graph_json: path_string(&bundle_dir.join("graph.json")),
        plan_json: path_string(&bundle_dir.join("plan.json")),
        prompts_dir: path_string(&bundle_dir.join("prompts")),
        reports_dir: path_string(&bundle_dir.join("reports")),
        worker_prompts: plan
            .workers
            .iter()
            .map(|worker| WorkerPromptArtifact {
                id: worker.id.clone(),
                prompt: path_string(&worker.prompt_path),
                report: path_string(&worker.report_path),
            })
            .collect(),
        supervisor_prompt: path_string(&plan.supervisor.prompt_path),
        supervisor_report: path_string(&plan.supervisor.report_path),
    }
}

fn workspace_binding_json(workspace: &WorkspaceBinding) -> JsonValue {
    serde_json::json!({
        "mode": workspace.mode.as_str(),
        "worktree_ref": workspace.worktree_ref.as_deref(),
        "base_commit": workspace.base_commit.as_deref(),
        "cleanup": workspace.cleanup.as_str()
    })
}

fn depends_label(depends_on: &[String]) -> String {
    if depends_on.is_empty() {
        "none".to_string()
    } else {
        depends_on
            .iter()
            .map(|dep| format!("`{dep}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "<br>")
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn tracking_doc(
    request: &GoalStartArgs,
    plan: &GoalPlan,
    bundle_dir: &Path,
) -> Result<String, ApiError> {
    render_goal_template(
        TEMPLATE_GOAL_TRACKING,
        &serde_json::json!({
            "task": request.task.as_str(),
            "context": request.context.as_deref().unwrap_or(""),
            "event": request.event.as_deref().unwrap_or(""),
            "trigger": request.trigger.as_deref().unwrap_or(""),
            "bundle_dir": path_string(bundle_dir),
            "workflow_path": path_string(&bundle_dir.join("workflow.apxmw")),
            "plan_json": path_string(&bundle_dir.join("plan.json")),
            "graph_json": path_string(&bundle_dir.join("graph.json")),
            "control": goal_control(),
            "workers": worker_prompt_rows(plan),
            "supervisor": supervisor_tracking_row(plan)
        }),
    )
}

fn render_worker_prompt(request: &GoalStartArgs, worker: &WorkerPlan) -> Result<String, ApiError> {
    render_goal_template(
        TEMPLATE_GOAL_WORKER,
        &serde_json::json!({
            "task": request.task.as_str(),
            "context": request.context.as_deref().unwrap_or(""),
            "event": request.event.as_deref().unwrap_or(""),
            "trigger": request.trigger.as_deref().unwrap_or(""),
            "worker": worker_prompt_context(worker)
        }),
    )
}

fn render_supervisor_prompt(
    request: &GoalStartArgs,
    plan: &GoalPlan,
    worker_summary: &str,
) -> Result<String, ApiError> {
    render_goal_template(
        TEMPLATE_GOAL_SUPERVISOR,
        &serde_json::json!({
            "task": request.task.as_str(),
            "context": request.context.as_deref().unwrap_or(""),
            "event": request.event.as_deref().unwrap_or(""),
            "trigger": request.trigger.as_deref().unwrap_or(""),
            "workers": worker_prompt_rows(plan),
            "supervisor": supervisor_prompt_context(&plan.supervisor, worker_summary)
        }),
    )
}

fn render_report_stub(
    owner_id: &str,
    owner_kind: &str,
    prompt_path: &Path,
    tracking_doc_path: &Path,
    graph_path: &Path,
) -> Result<String, ApiError> {
    render_goal_template(
        TEMPLATE_GOAL_REPORT_STUB,
        &serde_json::json!({
            "owner": {
                "id": owner_id,
                "kind": owner_kind,
                "prompt_path": path_string(prompt_path),
                "tracking_doc": path_string(tracking_doc_path),
                "graph_json": path_string(graph_path)
            }
        }),
    )
}

fn worker_prompt_context(worker: &WorkerPlan) -> JsonValue {
    serde_json::json!({
        "id": worker.id.as_str(),
        "role": worker.role.as_str(),
        "cwd": path_string(&worker.cwd),
        "tracking_doc": path_string(&worker.tracking_doc_path),
        "graph_json": path_string(&worker.graph_path),
        "prompt_path": path_string(&worker.prompt_path),
        "report_path": path_string(&worker.report_path),
        "depends_on": &worker.depends_on,
        "depends_label": depends_label(&worker.depends_on),
        "has_upstream": !worker.depends_on.is_empty(),
        "instructions": worker.prompt.as_str(),
        "workspace": workspace_binding_json(&worker.workspace)
    })
}

fn worker_prompt_rows(plan: &GoalPlan) -> Vec<JsonValue> {
    plan.workers
        .iter()
        .map(|worker| {
            serde_json::json!({
                "id": worker.id.as_str(),
                "role": worker.role.as_str(),
                "role_cell": markdown_cell(&worker.role),
                "depends_label": depends_label(&worker.depends_on),
                "cwd": path_string(&worker.cwd),
                "prompt_path": path_string(&worker.prompt_path),
                "report_path": path_string(&worker.report_path)
            })
        })
        .collect()
}

fn supervisor_prompt_context(supervisor: &SupervisorPlan, worker_summary: &str) -> JsonValue {
    serde_json::json!({
        "id": supervisor.id.as_str(),
        "tracking_doc": path_string(&supervisor.tracking_doc_path),
        "graph_json": path_string(&supervisor.graph_path),
        "prompt_path": path_string(&supervisor.prompt_path),
        "report_path": path_string(&supervisor.report_path),
        "worker_summary": worker_summary,
        "instructions": supervisor.prompt.as_str()
    })
}

fn supervisor_tracking_row(plan: &GoalPlan) -> JsonValue {
    serde_json::json!({
        "id": plan.supervisor.id.as_str(),
        "depends_label": plan
            .workers
            .iter()
            .map(|worker| format!("`{}`", worker.id))
            .collect::<Vec<_>>()
            .join(", "),
        "workspace_cwd": plan.supervisor.cwd.as_ref().map(|path| path_string(path)),
        "prompt_path": path_string(&plan.supervisor.prompt_path),
        "report_path": path_string(&plan.supervisor.report_path)
    })
}

fn render_goal_template<T: serde::Serialize>(
    template: &str,
    context: &T,
) -> Result<String, ApiError> {
    apxm_backends::render_prompt(template, context).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to render goal prompt template '{template}': {error}"
        ))
    })
}

fn worker_air(request: &GoalStartArgs, worker: &WorkerPlan) -> Result<String, ApiError> {
    if worker.transport == OrchestrationTransport::Acp {
        acp_worker_air(request, worker)
    } else {
        Ok(deterministic_worker_air(worker))
    }
}

fn acp_worker_air(request: &GoalStartArgs, worker: &WorkerPlan) -> Result<String, ApiError> {
    let message = worker_message(request, worker)?;
    let spawn_attrs = spawn_attrs(
        worker.profile.as_deref(),
        Some(&worker.cwd),
        worker.mode.as_deref(),
        worker.model.as_deref(),
    );
    let protocol = quote_air(CommunicateProtocol::Acp.as_str());
    Ok(format!(
        r#"module {{
  func.func @worker(%arg0: !ais.token {{ais.param_name = "task", ais.param_type = "str"}}, %arg1: !ais.token {{ais.param_name = "context", ais.param_type = "str"}}, %arg2: !ais.token {{ais.param_name = "event", ais.param_type = "str"}}, %arg3: !ais.token {{ais.param_name = "trigger", ais.param_type = "str"}}, %arg4: !ais.token {{ais.param_name = "upstream", ais.param_type = "str"}}) -> !ais.token attributes {{ais.entry}} {{
    %spawn = ais.spawn_agent {agent_name}{spawn_attrs} : !ais.token
    %turn = ais.communicate {message} to {agent_name} (%spawn : !ais.token) {{protocol = {protocol}}} : !ais.token
    func.return %turn : !ais.token
  }}
}}
"#,
        agent_name = quote_air(&worker.agent_name),
        spawn_attrs = spawn_attrs,
        message = quote_air(&message),
        protocol = protocol,
    ))
}

fn deterministic_worker_air(worker: &WorkerPlan) -> String {
    let label = format!(
        "worker:{} role:{} cwd:{} ",
        worker.id,
        worker.role,
        worker.cwd.display()
    );
    format!(
        r#"module {{
  func.func @worker(%arg0: !ais.token {{ais.param_name = "task", ais.param_type = "str"}}, %arg1: !ais.token {{ais.param_name = "context", ais.param_type = "str"}}, %arg2: !ais.token {{ais.param_name = "event", ais.param_type = "str"}}, %arg3: !ais.token {{ais.param_name = "trigger", ais.param_type = "str"}}, %arg4: !ais.token {{ais.param_name = "upstream", ais.param_type = "str"}}) -> !ais.token attributes {{ais.entry}} {{
    %label = ais.const_str {label} : !ais.token
    %out = ais.merge %label, %arg0, %arg1, %arg2, %arg3, %arg4 : !ais.token, !ais.token, !ais.token, !ais.token, !ais.token, !ais.token -> !ais.token
    func.return %out : !ais.token
  }}
}}
"#,
        label = quote_air(&label),
    )
}

fn gate_air(request: &GoalStartArgs, plan: &GoalPlan) -> Result<String, ApiError> {
    let supervisor = &plan.supervisor;
    if supervisor.transport == OrchestrationTransport::Acp {
        let cwd = supervisor.cwd.as_deref();
        let attrs = spawn_attrs(
            supervisor.profile.as_deref(),
            cwd,
            supervisor.mode.as_deref(),
            supervisor.model.as_deref(),
        );
        let message = runtime_message_with_summary_placeholder(&render_supervisor_prompt(
            request,
            plan,
            "__APXM_WORKER_SUMMARY__",
        )?);
        let protocol = quote_air(CommunicateProtocol::Acp.as_str());
        Ok(format!(
            r#"module {{
  func.func @gate(%arg0: !ais.token {{ais.param_name = "summary", ais.param_type = "str"}}) -> !ais.token attributes {{ais.entry}} {{
    %spawn = ais.spawn_agent {agent_name}{attrs} : !ais.token
    %gate = ais.communicate {message} to {agent_name} (%arg0, %spawn : !ais.token, !ais.token) {{protocol = {protocol}, input_names = ["summary"]}} : !ais.token
    func.return %gate : !ais.token
  }}
}}
"#,
            agent_name = quote_air(&supervisor.agent_name),
            attrs = attrs,
            message = quote_air(&message),
            protocol = protocol,
        ))
    } else {
        Ok(format!(
            r#"module {{
  func.func @gate(%arg0: !ais.token {{ais.param_name = "summary", ais.param_type = "str"}}) -> !ais.token attributes {{ais.entry}} {{
    %label = ais.const_str {label} : !ais.token
    %out = ais.merge %label, %arg0 : !ais.token, !ais.token -> !ais.token
    func.return %out : !ais.token
  }}
}}
"#,
            label = quote_air("gate/eval: "),
        ))
    }
}

fn feedback_air() -> String {
    format!(
        r#"module {{
  func.func @feedback(%arg0: !ais.token {{ais.param_name = "gate", ais.param_type = "str"}}) -> !ais.token attributes {{ais.entry}} {{
    %label = ais.const_str {label} : !ais.token
    %out = ais.merge %label, %arg0 : !ais.token, !ais.token -> !ais.token
    func.return %out : !ais.token
  }}
}}
"#,
        label = quote_air("feedback: "),
    )
}

fn workflow_json(request: &GoalStartArgs, plan: &GoalPlan) -> Result<Vec<u8>, ApiError> {
    let mut graphs = Vec::with_capacity(plan.workers.len() + 2);
    for worker in &plan.workers {
        let upstream = upstream_template(&worker.depends_on);
        graphs.push(serde_json::json!({
            "id": worker.id,
            "path": format!("workers/{}.air", worker.id),
            "depends_on": worker.depends_on,
            "params": {
                "task": request.task,
                "context": request.context.as_deref().unwrap_or(""),
                "event": request.event.as_deref().unwrap_or(""),
                "trigger": request.trigger.as_deref().unwrap_or(""),
                "upstream": upstream
            }
        }));
    }
    let worker_summary = plan
        .workers
        .iter()
        .map(|worker| format!("{}={{{{{}.output}}}}", worker.id, worker.id))
        .collect::<Vec<_>>()
        .join("\n");
    let worker_ids = plan
        .workers
        .iter()
        .map(|worker| worker.id.clone())
        .collect::<Vec<_>>();
    graphs.push(serde_json::json!({
        "id": plan.supervisor.id,
        "path": "gate.air",
        "depends_on": worker_ids,
        "params": {
            "summary": worker_summary
        }
    }));
    graphs.push(serde_json::json!({
        "id": "feedback",
        "path": "feedback.air",
        "depends_on": [plan.supervisor.id.clone()],
        "params": {
            "gate": format!("{{{{{}.output}}}}", plan.supervisor.id)
        }
    }));

    serde_json::to_vec_pretty(&serde_json::json!({
        "name": "goal_pass",
        "description": "Generated by apxm_goal_start: event -> trigger -> parallel workers -> gate/eval -> feedback.",
        "graphs": graphs,
        "output": "{{feedback.output}}"
    }))
    .map_err(|error| ApiError::internal_message(format!("failed to serialize workflow: {error}")))
}

fn upstream_template(depends_on: &[String]) -> String {
    depends_on
        .iter()
        .map(|dep| format!("{dep}={{{{{dep}.output}}}}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn spawn_attrs(
    profile: Option<&str>,
    cwd: Option<&Path>,
    mode: Option<&str>,
    model: Option<&str>,
) -> String {
    let mut attrs = Vec::new();
    if let Some(profile) = profile {
        attrs.push(format!("profile = {}", quote_air(profile)));
    }
    if let Some(cwd) = cwd {
        attrs.push(format!("cwd = {}", quote_air(&cwd.to_string_lossy())));
    }
    if let Some(mode) = mode {
        attrs.push(format!("mode = {}", quote_air(mode)));
    }
    if let Some(model) = model {
        attrs.push(format!("model = {}", quote_air(model)));
    }
    if attrs.is_empty() {
        String::new()
    } else {
        format!(" {{{}}}", attrs.join(", "))
    }
}

fn worker_message(request: &GoalStartArgs, worker: &WorkerPlan) -> Result<String, ApiError> {
    render_worker_prompt(request, worker)
}

fn runtime_message_with_summary_placeholder(message: &str) -> String {
    escape_runtime_template_literals(message).replace("__APXM_WORKER_SUMMARY__", "{summary}")
}

fn escape_runtime_template_literals(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '{' => escaped.push_str("{{"),
            '}' => escaped.push_str("}}"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn agent_name(session_id: &str, id: &str) -> String {
    let suffix = session_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>();
    if suffix.is_empty() {
        format!("goal_worker_{id}")
    } else {
        format!("goal_worker_{id}_{suffix}")
    }
}

fn effective_transport(
    transport: Option<OrchestrationTransport>,
    profile: Option<&str>,
) -> OrchestrationTransport {
    transport.unwrap_or(if profile.is_some() {
        OrchestrationTransport::Acp
    } else {
        OrchestrationTransport::Deterministic
    })
}

fn goal_uses_process_spawns(request: &GoalStartArgs) -> bool {
    request.workers.iter().any(|worker| {
        effective_transport(worker.transport, worker.profile.as_deref())
            == OrchestrationTransport::Acp
    }) || request.supervisor.as_ref().is_some_and(|supervisor| {
        effective_transport(supervisor.transport, supervisor.profile.as_deref())
            == OrchestrationTransport::Acp
    })
}

fn validate_component_id(id: &str, field: &str) -> Result<(), ApiError> {
    if id.is_empty() || id.len() > 96 || id == "." || id == ".." {
        return Err(ApiError::bad_request(format!(
            "{field} must be 1-96 characters and not '.' or '..'"
        )));
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(ApiError::bad_request(format!(
            "{field} may contain only ASCII letters, digits, '_' and '-'"
        )));
    }
    Ok(())
}

fn resolve_repo_root(raw: &str) -> Result<PathBuf, ApiError> {
    if raw.trim().is_empty() || raw.contains('\0') {
        return Err(ApiError::bad_request(
            "workspace.repo_root must not be empty or contain NUL",
        ));
    }
    let canonical = std::fs::canonicalize(raw).map_err(|error| {
        ApiError::bad_request(format!(
            "workspace.repo_root is not readable: {raw}: {error}"
        ))
    })?;
    if !canonical.is_dir() {
        return Err(ApiError::bad_request(format!(
            "workspace.repo_root is not a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn create_git_worktree(repo_root: &Path, cwd: &Path, base_ref: &str) -> Result<(), ApiError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .map_err(|error| ApiError::bad_request(format!("failed to run git: {error}")))?;
    if !output.status.success() {
        return Err(ApiError::bad_request(format!(
            "workspace.repo_root is not a git repository: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let top_level = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let top_level = std::fs::canonicalize(&top_level).map_err(|error| {
        ApiError::bad_request(format!("failed to canonicalize git root: {error}"))
    })?;
    if top_level != repo_root {
        return Err(ApiError::bad_request(format!(
            "workspace.repo_root must be the git top-level '{}'",
            top_level.display()
        )));
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("add")
        .arg("--detach")
        .arg(cwd)
        .arg(base_ref)
        .output()
        .map_err(|error| {
            ApiError::bad_request(format!("failed to create git worktree: {error}"))
        })?;
    if !output.status.success() {
        return Err(ApiError::bad_request(format!(
            "git worktree add failed for '{}': {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn git_rev_parse(repo_root: &Path, rev: &str) -> Result<String, ApiError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("rev-parse")
        .arg(rev)
        .output()
        .map_err(|error| ApiError::bad_request(format!("failed to run git rev-parse: {error}")))?;
    if !output.status.success() {
        return Err(ApiError::bad_request(format!(
            "git rev-parse {rev} failed in '{}': {}",
            repo_root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

impl GoalPlan {
    fn summary(&self) -> GoalPlanSummary {
        GoalPlanSummary {
            task: self.task.clone(),
            workers: self
                .workers
                .iter()
                .map(|worker| WorkerPlanSummary {
                    id: worker.id.clone(),
                    role: worker.role.clone(),
                    transport: worker.transport.clone(),
                    profile: worker.profile.clone(),
                    depends_on: worker.depends_on.clone(),
                    cwd: worker.cwd.to_string_lossy().to_string(),
                    workspace: WorkspaceBindingSummary {
                        mode: worker.workspace.mode.clone(),
                        worktree_ref: worker.workspace.worktree_ref.clone(),
                        base_commit: worker.workspace.base_commit.clone(),
                        cleanup: worker.workspace.cleanup.clone(),
                    },
                })
                .collect(),
            supervisor: SupervisorPlanSummary {
                id: self.supervisor.id.clone(),
                transport: self.supervisor.transport.clone(),
                profile: self.supervisor.profile.clone(),
            },
            workspace_mode: self.workspace_mode.clone(),
        }
    }
}

fn default_worker_prompt(id: &str) -> Result<String, ApiError> {
    render_goal_template(
        TEMPLATE_GOAL_DEFAULT_WORKER,
        &serde_json::json!({ "worker_id": id }),
    )
}

fn default_supervisor_id() -> String {
    "gate".to_string()
}

fn default_supervisor_prompt() -> Result<String, ApiError> {
    render_goal_template(TEMPLATE_GOAL_DEFAULT_SUPERVISOR, &serde_json::json!({}))
}

fn goal_control() -> GoalControl {
    GoalControl {
        status_tool: super::workflow::MCP_TOOL_APXM_WORKFLOW_STATUS,
        events_tool: super::workflow::MCP_TOOL_APXM_WORKFLOW_EVENTS,
        cancel_tool: super::workflow::MCP_TOOL_APXM_WORKFLOW_CANCEL,
    }
}

fn goal_terminal_event_kinds() -> Vec<&'static str> {
    vec![
        kind::ORCHESTRATOR_WAKE.name(),
        kind::EXECUTE_COMPLETE.name(),
        kind::ERROR.name(),
        kind::TURN_ABORTED.name(),
    ]
}

fn goal_wake_on() -> Vec<String> {
    vec![
        format!(
            "{} returns {}",
            super::workflow::MCP_TOOL_APXM_WORKFLOW_EVENTS,
            kind::ORCHESTRATOR_WAKE.name()
        ),
        format!(
            "{} returns {}, {}, or {}",
            super::workflow::MCP_TOOL_APXM_WORKFLOW_EVENTS,
            kind::EXECUTE_COMPLETE.name(),
            kind::ERROR.name(),
            kind::TURN_ABORTED.name()
        ),
        format!(
            "{} reports succeeded or failed",
            super::workflow::MCP_TOOL_APXM_WORKFLOW_STATUS
        ),
        format!(
            "{} is called by the supervisor/client",
            super::workflow::MCP_TOOL_APXM_WORKFLOW_CANCEL
        ),
    ]
}

fn goal_event_loop() -> &'static str {
    "event -> trigger -> parallel worker actions -> gate/eval -> feedback -> next event"
}

fn goal_runtime_contract(execution_id: Option<String>, gate_step_id: &str) -> GoalRuntimeContract {
    let next_events_args = execution_id.as_ref().map(|execution_id| {
        serde_json::json!({
            "execution_id": execution_id,
            "since": 0,
            "limit": 100
        })
    });
    GoalRuntimeContract {
        execution_id,
        initial_since: 0,
        gate_step_id: gate_step_id.to_string(),
        feedback_step_id: "feedback",
        terminal_event_kinds: goal_terminal_event_kinds(),
        next_events_args,
        sleep_event_kind: kind::ORCHESTRATOR_SLEEP.name(),
        wake_event_kind: kind::ORCHESTRATOR_WAKE.name(),
    }
}

fn goal_prompt() -> Result<String, ApiError> {
    render_goal_template(
        TEMPLATE_GOAL_CONTROLLER,
        &serde_json::json!({
            "start_tool": MCP_TOOL_APXM_GOAL_START,
            "control": goal_control(),
            "terminal_event_kinds": goal_terminal_event_kinds(),
            "sleep_event_kind": kind::ORCHESTRATOR_SLEEP.name(),
            "wake_event_kind": kind::ORCHESTRATOR_WAKE.name()
        }),
    )
}

fn goal_flowchart() -> Result<String, ApiError> {
    render_goal_template(TEMPLATE_GOAL_FLOWCHART, &serde_json::json!({}))
}

fn quote_air(value: &str) -> String {
    format!("\"{}\"", apxm_ais::chat::escape_air_string(value))
}

fn mcp_json_tool_result<T: serde::Serialize>(id: JsonValue, value: T) -> Json<JsonValue> {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|error| {
        serde_json::json!({ "error": format!("failed to serialize MCP result: {error}") })
            .to_string()
    });
    mcp_tool_result(id, text, false)
}
