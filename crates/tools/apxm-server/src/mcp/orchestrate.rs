//! Native task orchestration MCP entry point.
//!
//! This layer is intentionally a compiler/materializer, not a second runtime:
//! it turns a bounded worker plan into a generated workflow bundle and then
//! starts that bundle through `apxm_workflow_start`, so status/events/cancel
//! stay on the existing workflow control plane.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use apxm_core::paths::ApxmPaths;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::error::ApiError;
use crate::helpers::mcp_tool_result;
use crate::state::AppState;

use super::workflow::{WorkflowStartArgs, start_workflow_from_args};

pub(crate) const MCP_TOOL_APXM_ORCHESTRATE_START: &str = "apxm_orchestrate_start";

const MAX_WORKERS: usize = 16;
const ADMIT_SPAWN_AGENT: &str = "SPAWN_AGENT";
const DEFAULT_WORKSPACE_MODE: &str = "session";
const WORKSPACE_MODE_SESSION: &str = "session";
const WORKSPACE_MODE_SHARED: &str = "shared";
const WORKSPACE_MODE_GIT_WORKTREE: &str = "git_worktree";
const TRANSPORT_ACP: &str = "acp";
const TRANSPORT_DETERMINISTIC: &str = "deterministic";

#[derive(Debug, Deserialize)]
struct OrchestrateStartArgs {
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
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkerSpec {
    id: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SupervisorSpec {
    #[serde(default = "default_supervisor_id")]
    id: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkspaceSpec {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    repo_root: Option<String>,
    #[serde(default)]
    base_ref: Option<String>,
    #[serde(default)]
    cleanup: Option<String>,
}

#[derive(Debug, Serialize)]
struct OrchestrateStartResponse {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_id: Option<String>,
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_dir: Option<String>,
    workflow_path: String,
    bundle_dir: String,
    plan: OrchestrationPlanSummary,
    control: OrchestrationControl,
    sleep_wake: SleepWakeContract,
    orchestrator_prompt: String,
    flowchart: String,
}

#[derive(Debug, Serialize)]
struct OrchestrationPlanSummary {
    task: String,
    workers: Vec<WorkerPlanSummary>,
    supervisor: SupervisorPlanSummary,
    workspace_mode: String,
}

#[derive(Debug, Serialize)]
struct WorkerPlanSummary {
    id: String,
    role: String,
    transport: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    depends_on: Vec<String>,
    cwd: String,
    workspace: WorkspaceBindingSummary,
}

#[derive(Debug, Serialize)]
struct SupervisorPlanSummary {
    id: String,
    transport: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkspaceBindingSummary {
    mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_ref: Option<String>,
    cleanup: String,
}

#[derive(Debug, Serialize)]
struct OrchestrationControl {
    status_tool: &'static str,
    events_tool: &'static str,
    cancel_tool: &'static str,
}

#[derive(Debug, Serialize)]
struct SleepWakeContract {
    sleep_after_start: bool,
    wake_on: Vec<&'static str>,
    event_loop: &'static str,
}

struct OrchestrationBundle {
    session_id: String,
    bundle_dir: PathBuf,
    workflow_path: PathBuf,
    plan: OrchestrationPlan,
}

struct OrchestrationPlan {
    task: String,
    workers: Vec<WorkerPlan>,
    supervisor: SupervisorPlan,
    workspace_mode: String,
}

struct WorkerPlan {
    id: String,
    agent_name: String,
    role: String,
    prompt: String,
    profile: Option<String>,
    transport: String,
    depends_on: Vec<String>,
    mode: Option<String>,
    model: Option<String>,
    cwd: PathBuf,
    workspace: WorkspaceBinding,
}

struct SupervisorPlan {
    id: String,
    agent_name: String,
    prompt: String,
    profile: Option<String>,
    transport: String,
    mode: Option<String>,
    model: Option<String>,
    cwd: Option<PathBuf>,
}

struct WorkspaceBinding {
    mode: String,
    worktree_ref: Option<String>,
    cleanup: String,
}

pub(crate) fn orchestrate_start_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["task", "workers"],
        "properties": {
            "task": {
                "type": "string",
                "description": "Top-level task the autonomous orchestrator should split and supervise"
            },
            "context": {
                "type": "string",
                "description": "Optional repository, product, or run context passed to workers"
            },
            "event": {
                "type": "string",
                "description": "Optional event that triggered this orchestration loop"
            },
            "trigger": {
                "type": "string",
                "description": "Optional trigger rule or reason for this loop pass"
            },
            "workers": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_WORKERS,
                "description": "Bounded worker graph. Independent workers run in parallel; depends_on creates fan-in/fan-out phases.",
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
                            "description": "Optional ACP profile such as claude, codex, or any registered custom agent"
                        },
                        "transport": {
                            "type": "string",
                            "enum": [TRANSPORT_ACP, TRANSPORT_DETERMINISTIC],
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
                    "transport": { "type": "string", "enum": [TRANSPORT_ACP, TRANSPORT_DETERMINISTIC] },
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
                        "enum": [WORKSPACE_MODE_SESSION, WORKSPACE_MODE_SHARED, WORKSPACE_MODE_GIT_WORKTREE],
                        "description": "session creates APXM-owned directories; git_worktree creates one detached Git worktree per worker; shared uses repo_root/current cwd"
                    },
                    "repo_root": { "type": "string", "description": "Required for git_worktree; optional for shared" },
                    "base_ref": { "type": "string", "description": "Git ref for git_worktree mode; defaults to HEAD" },
                    "cleanup": { "type": "string", "enum": ["keep"], "description": "MVP keeps generated workspaces/worktrees for review" }
                }
            },
            "session_id": { "type": "string" },
            "admit_capabilities": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Must include SPAWN_AGENT when any worker/supervisor uses transport=acp"
            },
            "imports": { "type": "array", "items": { "type": "string" } },
            "dry_run": {
                "type": "boolean",
                "description": "Validate and materialize the workflow bundle without starting it"
            }
        }
    })
}

pub(crate) async fn call_orchestrate_tool(
    state: &AppState,
    id: &JsonValue,
    tool_name: &str,
    tool_args: &JsonValue,
) -> Option<Json<JsonValue>> {
    if tool_name != MCP_TOOL_APXM_ORCHESTRATE_START {
        return None;
    }
    Some(match orchestrate_start(state, tool_args).await {
        Ok(response) => mcp_json_tool_result(id.clone(), response),
        Err(error) => mcp_tool_result(id.clone(), error.message, true),
    })
}

async fn orchestrate_start(
    state: &AppState,
    tool_args: &JsonValue,
) -> Result<OrchestrateStartResponse, ApiError> {
    let request: OrchestrateStartArgs =
        serde_json::from_value(tool_args.clone()).map_err(|error| {
            ApiError::bad_request(format!("invalid orchestrate_start arguments: {error}"))
        })?;
    let uses_process_spawns = orchestration_uses_process_spawns(&request);
    if uses_process_spawns
        && !request
            .admit_capabilities
            .iter()
            .any(|capability| capability == ADMIT_SPAWN_AGENT)
    {
        return Err(ApiError::bad_request(format!(
            "{MCP_TOOL_APXM_ORCHESTRATE_START}: transport=acp requires admit_capabilities=[\"{ADMIT_SPAWN_AGENT}\"]"
        )));
    }

    let bundle = materialize_orchestration_bundle(&request)?;
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
                },
            )
            .await?,
        )
    };

    let status = started
        .as_ref()
        .map(|response| format!("{:?}", response.status).to_ascii_lowercase())
        .unwrap_or_else(|| "planned".to_string());
    let response = OrchestrateStartResponse {
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
        plan: bundle.plan.summary(),
        control: OrchestrationControl {
            status_tool: super::workflow::MCP_TOOL_APXM_WORKFLOW_STATUS,
            events_tool: super::workflow::MCP_TOOL_APXM_WORKFLOW_EVENTS,
            cancel_tool: super::workflow::MCP_TOOL_APXM_WORKFLOW_CANCEL,
        },
        sleep_wake: SleepWakeContract {
            sleep_after_start: !request.dry_run,
            wake_on: vec![
                "apxm_workflow_events returns a terminal execute_complete or turn_aborted event",
                "apxm_workflow_status reports succeeded or failed",
                "apxm_workflow_cancel is called by the supervisor/client",
            ],
            event_loop: "event -> trigger -> parallel worker actions -> gate/eval -> feedback -> next event",
        },
        orchestrator_prompt: orchestrator_prompt(),
        flowchart: orchestration_flowchart(),
    };
    Ok(response)
}

fn materialize_orchestration_bundle(
    request: &OrchestrateStartArgs,
) -> Result<OrchestrationBundle, ApiError> {
    validate_request(request)?;
    let session_id = request
        .session_id
        .clone()
        .unwrap_or_else(|| format!("orchestrate-{}", uuid::Uuid::new_v4()));
    validate_component_id(&session_id, "session_id")?;

    let paths = ApxmPaths::discover().map_err(|error| {
        ApiError::internal_message(format!("failed to resolve APXM paths: {error}"))
    })?;
    let root = paths
        .cache_component_dir("orchestrations")
        .map_err(|error| {
            ApiError::internal_message(format!("failed to create orchestration cache: {error}"))
        })?;
    let bundle_dir = root.join(&session_id);
    if bundle_dir.exists() {
        return Err(ApiError::bad_request(format!(
            "orchestration session already exists: {session_id}"
        )));
    }
    std::fs::create_dir_all(&bundle_dir).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create orchestration bundle '{}': {error}",
            bundle_dir.display()
        ))
    })?;

    let workspace_policy = WorkspacePolicy::from_spec(request.workspace.as_ref(), &bundle_dir)?;
    let plan = build_plan(request, &session_id, &bundle_dir, &workspace_policy)?;
    write_bundle_files(&bundle_dir, request, &plan)?;

    Ok(OrchestrationBundle {
        session_id,
        workflow_path: bundle_dir.join("workflow.apxmw"),
        bundle_dir,
        plan,
    })
}

fn validate_request(request: &OrchestrateStartArgs) -> Result<(), ApiError> {
    if request.task.trim().is_empty() {
        return Err(ApiError::bad_request(
            "orchestrate_start task must not be empty",
        ));
    }
    if request.workers.is_empty() {
        return Err(ApiError::bad_request(
            "orchestrate_start workers must be non-empty",
        ));
    }
    if request.workers.len() > MAX_WORKERS {
        return Err(ApiError::bad_request(format!(
            "orchestrate_start workers exceeds breadth cap of {MAX_WORKERS}"
        )));
    }

    let mut ids = HashSet::new();
    for worker in &request.workers {
        validate_component_id(&worker.id, "worker.id")?;
        if !ids.insert(worker.id.as_str()) {
            return Err(ApiError::bad_request(format!(
                "orchestrate_start duplicate worker id '{}'",
                worker.id
            )));
        }
        validate_transport(
            worker.transport.as_deref(),
            worker.profile.as_deref(),
            "worker",
        )?;
        validate_optional_text(worker.role.as_deref(), "worker.role")?;
        validate_optional_text(worker.prompt.as_deref(), "worker.prompt")?;
        validate_optional_text(worker.mode.as_deref(), "worker.mode")?;
        validate_optional_text(worker.model.as_deref(), "worker.model")?;
    }
    if let Some(supervisor) = &request.supervisor {
        validate_component_id(&supervisor.id, "supervisor.id")?;
        if ids.contains(supervisor.id.as_str()) {
            return Err(ApiError::bad_request(format!(
                "orchestrate_start supervisor id '{}' conflicts with a worker id",
                supervisor.id
            )));
        }
        validate_transport(
            supervisor.transport.as_deref(),
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
    transport: Option<&str>,
    profile: Option<&str>,
    owner: &str,
) -> Result<(), ApiError> {
    match transport.unwrap_or(if profile.is_some() {
        TRANSPORT_ACP
    } else {
        TRANSPORT_DETERMINISTIC
    }) {
        TRANSPORT_ACP => {
            if profile.is_none_or(str::is_empty) {
                return Err(ApiError::bad_request(format!(
                    "{owner} transport=acp requires a profile"
                )));
            }
        }
        TRANSPORT_DETERMINISTIC => {
            if profile.is_some() {
                return Err(ApiError::bad_request(format!(
                    "{owner} profile requires transport=acp"
                )));
            }
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "{owner} transport must be '{TRANSPORT_ACP}' or '{TRANSPORT_DETERMINISTIC}', got '{other}'"
            )));
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
            "orchestrate_start worker dependency cycle involving '{id}'"
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
    mode: String,
    repo_root: Option<PathBuf>,
    base_ref: String,
    cleanup: String,
    bundle_dir: PathBuf,
}

impl WorkspacePolicy {
    fn from_spec(spec: Option<&WorkspaceSpec>, bundle_dir: &Path) -> Result<Self, ApiError> {
        let mode = spec
            .and_then(|spec| spec.mode.as_deref())
            .unwrap_or(DEFAULT_WORKSPACE_MODE)
            .to_string();
        if !matches!(
            mode.as_str(),
            WORKSPACE_MODE_SESSION | WORKSPACE_MODE_SHARED | WORKSPACE_MODE_GIT_WORKTREE
        ) {
            return Err(ApiError::bad_request(format!(
                "workspace.mode must be '{WORKSPACE_MODE_SESSION}', '{WORKSPACE_MODE_SHARED}', or '{WORKSPACE_MODE_GIT_WORKTREE}'"
            )));
        }
        let cleanup = spec
            .and_then(|spec| spec.cleanup.as_deref())
            .unwrap_or("keep")
            .to_string();
        if cleanup != "keep" {
            return Err(ApiError::bad_request(
                "workspace.cleanup currently supports only 'keep'",
            ));
        }
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
        if mode == WORKSPACE_MODE_GIT_WORKTREE && repo_root.is_none() {
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
        match self.mode.as_str() {
            WORKSPACE_MODE_SESSION => {
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
                        mode: WORKSPACE_MODE_SESSION.to_string(),
                        worktree_ref: None,
                        cleanup: self.cleanup.clone(),
                    },
                ))
            }
            WORKSPACE_MODE_SHARED => {
                let cwd = self.repo_root.clone().unwrap_or_else(|| {
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                });
                Ok((
                    cwd,
                    WorkspaceBinding {
                        mode: WORKSPACE_MODE_SHARED.to_string(),
                        worktree_ref: None,
                        cleanup: self.cleanup.clone(),
                    },
                ))
            }
            WORKSPACE_MODE_GIT_WORKTREE => {
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
                Ok((
                    cwd,
                    WorkspaceBinding {
                        mode: WORKSPACE_MODE_GIT_WORKTREE.to_string(),
                        worktree_ref: Some(self.base_ref.clone()),
                        cleanup: self.cleanup.clone(),
                    },
                ))
            }
            _ => unreachable!("workspace mode validated"),
        }
    }
}

fn build_plan(
    request: &OrchestrateStartArgs,
    session_id: &str,
    bundle_dir: &Path,
    workspace_policy: &WorkspacePolicy,
) -> Result<OrchestrationPlan, ApiError> {
    let mut workers = Vec::with_capacity(request.workers.len());
    for worker in &request.workers {
        let (cwd, workspace) = workspace_policy.allocate(&worker.id)?;
        let transport = effective_transport(worker.transport.as_deref(), worker.profile.as_deref());
        workers.push(WorkerPlan {
            id: worker.id.clone(),
            agent_name: agent_name(session_id, &worker.id),
            role: worker
                .role
                .clone()
                .unwrap_or_else(|| format!("worker {}", worker.id)),
            prompt: worker
                .prompt
                .clone()
                .unwrap_or_else(|| default_worker_prompt(&worker.id)),
            profile: worker.profile.clone(),
            transport,
            depends_on: worker.depends_on.clone(),
            mode: worker.mode.clone(),
            model: worker.model.clone(),
            cwd,
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
        supervisor_spec.transport.as_deref(),
        supervisor_spec.profile.as_deref(),
    );
    let supervisor_cwd = if supervisor_transport == TRANSPORT_ACP {
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

    Ok(OrchestrationPlan {
        task: request.task.clone(),
        workers,
        supervisor: SupervisorPlan {
            agent_name: agent_name(session_id, &supervisor_spec.id),
            id: supervisor_spec.id,
            prompt: supervisor_spec
                .prompt
                .unwrap_or_else(default_supervisor_prompt),
            profile: supervisor_spec.profile,
            transport: supervisor_transport,
            mode: supervisor_spec.mode,
            model: supervisor_spec.model,
            cwd: supervisor_cwd,
        },
        workspace_mode: workspace_policy.mode.clone(),
    })
}

fn write_bundle_files(
    bundle_dir: &Path,
    request: &OrchestrateStartArgs,
    plan: &OrchestrationPlan,
) -> Result<(), ApiError> {
    let workers_dir = bundle_dir.join("workers");
    std::fs::create_dir_all(&workers_dir).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create workers dir '{}': {error}",
            workers_dir.display()
        ))
    })?;
    for worker in &plan.workers {
        let air = worker_air(request, worker);
        std::fs::write(workers_dir.join(format!("{}.air", worker.id)), air).map_err(|error| {
            ApiError::internal_message(format!(
                "failed to write worker graph '{}': {error}",
                worker.id
            ))
        })?;
    }

    std::fs::write(bundle_dir.join("gate.air"), gate_air(&plan.supervisor)).map_err(|error| {
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
    std::fs::write(
        bundle_dir.join("orchestrator_prompt.txt"),
        orchestrator_prompt(),
    )
    .map_err(|error| {
        ApiError::internal_message(format!("failed to write orchestrator prompt: {error}"))
    })?;
    Ok(())
}

fn worker_air(request: &OrchestrateStartArgs, worker: &WorkerPlan) -> String {
    if worker.transport == TRANSPORT_ACP {
        acp_worker_air(request, worker)
    } else {
        deterministic_worker_air(worker)
    }
}

fn acp_worker_air(request: &OrchestrateStartArgs, worker: &WorkerPlan) -> String {
    let message = worker_message(request, worker);
    let spawn_attrs = spawn_attrs(
        worker.profile.as_deref(),
        Some(&worker.cwd),
        worker.mode.as_deref(),
        worker.model.as_deref(),
    );
    format!(
        r#"module {{
  func.func @worker(%arg0: !ais.token {{ais.param_name = "task", ais.param_type = "str"}}, %arg1: !ais.token {{ais.param_name = "context", ais.param_type = "str"}}, %arg2: !ais.token {{ais.param_name = "event", ais.param_type = "str"}}, %arg3: !ais.token {{ais.param_name = "trigger", ais.param_type = "str"}}, %arg4: !ais.token {{ais.param_name = "upstream", ais.param_type = "str"}}) -> !ais.token attributes {{ais.entry}} {{
    %spawn = ais.spawn_agent {agent_name}{spawn_attrs} : !ais.token
    %turn = ais.communicate {message} to {agent_name} (%spawn : !ais.token) {{protocol = "acp"}} : !ais.token
    func.return %turn : !ais.token
  }}
}}
"#,
        agent_name = quote_air(&worker.agent_name),
        spawn_attrs = spawn_attrs,
        message = quote_air(&message),
    )
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

fn gate_air(supervisor: &SupervisorPlan) -> String {
    if supervisor.transport == TRANSPORT_ACP {
        let cwd = supervisor.cwd.as_deref();
        let attrs = spawn_attrs(
            supervisor.profile.as_deref(),
            cwd,
            supervisor.mode.as_deref(),
            supervisor.model.as_deref(),
        );
        let message = format!(
            "{}\n\nReturn gate decision, failed assumptions, merge/conflict notes, and next feedback action. Worker outputs are also persisted in the APXM workflow session for this gate step.",
            supervisor.prompt
        );
        format!(
            r#"module {{
  func.func @gate(%arg0: !ais.token {{ais.param_name = "summary", ais.param_type = "str"}}) -> !ais.token attributes {{ais.entry}} {{
    %spawn = ais.spawn_agent {agent_name}{attrs} : !ais.token
    %gate = ais.communicate {message} to {agent_name} (%spawn : !ais.token) {{protocol = "acp"}} : !ais.token
    func.return %gate : !ais.token
  }}
}}
"#,
            agent_name = quote_air(&supervisor.agent_name),
            attrs = attrs,
            message = quote_air(&message),
        )
    } else {
        format!(
            r#"module {{
  func.func @gate(%arg0: !ais.token {{ais.param_name = "summary", ais.param_type = "str"}}) -> !ais.token attributes {{ais.entry}} {{
    %label = ais.const_str {label} : !ais.token
    %out = ais.merge %label, %arg0 : !ais.token, !ais.token -> !ais.token
    func.return %out : !ais.token
  }}
}}
"#,
            label = quote_air("gate/eval: "),
        )
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

fn workflow_json(
    request: &OrchestrateStartArgs,
    plan: &OrchestrationPlan,
) -> Result<Vec<u8>, ApiError> {
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
        "name": "apxm_orchestrated_task",
        "description": "Generated by apxm_orchestrate_start: event -> trigger -> parallel workers -> gate/eval -> feedback.",
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

fn worker_message(request: &OrchestrateStartArgs, worker: &WorkerPlan) -> String {
    let upstream_note = if worker.depends_on.is_empty() {
        "No upstream worker dependencies.".to_string()
    } else {
        format!(
            "This worker is ordered after: {}. Use the APXM workflow session/event stream for upstream output context when needed.",
            worker.depends_on.join(", ")
        )
    };
    format!(
        "You are APXM worker '{id}'.\nRole: {role}\nAssigned workspace: {cwd}\nWorkspace mode: {workspace_mode}\n\nTask:\n{task}\n\nContext:\n{context}\n\nEvent:\n{event}\n\nTrigger:\n{trigger}\n\nUpstream:\n{upstream_note}\n\nInstructions:\n{prompt}\n\nReturn: status, concrete output, changed files if any, tests run, blockers, and handoff notes.",
        id = worker.id,
        role = worker.role,
        cwd = worker.cwd.display(),
        workspace_mode = worker.workspace.mode,
        task = request.task.as_str(),
        context = request.context.as_deref().unwrap_or(""),
        event = request.event.as_deref().unwrap_or(""),
        trigger = request.trigger.as_deref().unwrap_or(""),
        upstream_note = upstream_note,
        prompt = worker.prompt,
    )
}

fn agent_name(session_id: &str, id: &str) -> String {
    let suffix = session_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>();
    if suffix.is_empty() {
        format!("apxm_worker_{id}")
    } else {
        format!("apxm_worker_{id}_{suffix}")
    }
}

fn effective_transport(transport: Option<&str>, profile: Option<&str>) -> String {
    transport
        .unwrap_or(if profile.is_some() {
            TRANSPORT_ACP
        } else {
            TRANSPORT_DETERMINISTIC
        })
        .to_string()
}

fn orchestration_uses_process_spawns(request: &OrchestrateStartArgs) -> bool {
    request.workers.iter().any(|worker| {
        effective_transport(worker.transport.as_deref(), worker.profile.as_deref()) == TRANSPORT_ACP
    }) || request.supervisor.as_ref().is_some_and(|supervisor| {
        effective_transport(
            supervisor.transport.as_deref(),
            supervisor.profile.as_deref(),
        ) == TRANSPORT_ACP
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

impl OrchestrationPlan {
    fn summary(&self) -> OrchestrationPlanSummary {
        OrchestrationPlanSummary {
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

fn default_worker_prompt(id: &str) -> String {
    format!(
        "Complete the '{id}' slice independently. Stay inside your assigned workspace/worktree, avoid asking the human for prompts, and report enough detail for the supervisor to evaluate the result."
    )
}

fn default_supervisor_id() -> String {
    "gate".to_string()
}

fn default_supervisor_prompt() -> String {
    "Act as the APXM gatekeeper. Evaluate all worker outputs, identify conflicts or missing verification, decide whether the loop can finish, and emit feedback for the next loop if needed.".to_string()
}

fn orchestrator_prompt() -> String {
    "You are the APXM autonomous orchestrator. Convert the incoming event/task into a bounded worker graph, call apxm_orchestrate_start once with explicit workers and workspace policy, then go idle. Do not keep prompting the workers manually. Wake by reading apxm_workflow_events/status for the returned execution_id; cancel with apxm_workflow_cancel when policy or budget requires it. On completion, run the gate/eval feedback through the next event->trigger->action loop only if the returned feedback says another bounded pass is necessary.".to_string()
}

fn orchestration_flowchart() -> String {
    "[event/task]\n    |\n    v\n[trigger + bounded worker plan]\n    |\n    v\n[allocate per-worker workspace/worktree]\n    |\n    v\n[start APXM background workflow]\n    |\n    +--> [worker A SPAWN_AGENT -> COMMUNICATE]\n    +--> [worker B SPAWN_AGENT -> COMMUNICATE]\n    +--> [worker N SPAWN_AGENT -> COMMUNICATE]\n    |\n    v\n[gate/eval waits for all workers]\n    |\n    v\n[feedback]\n    |\n    v\n[sleep until status/events/cancel wakes the orchestrator]".to_string()
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
