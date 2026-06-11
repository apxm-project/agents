//! Native goal MCP entry point.
//!
//! A goal is a server-owned sequence of bounded workflow passes. This layer
//! tracks aggregate goal state and lets the existing workflow runtime execute
//! each pass.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use apxm_backends::HealthStatus;
use apxm_backends::LLMRequest;
use apxm_core::constants::mcp::tools as mcp_tool_names;
use apxm_core::constants::orchestration::admission as goal_admission;
use apxm_core::events::kind;
use apxm_core::events::payload::{
    ErrorPayload, ExecutionStartedPayload, GoalConvergedPayload, GoalGateVerdictPayload,
    GoalHaltedPayload, GoalNeedsAnotherPassPayload, OrchestratorSleepPayload,
    OrchestratorWakePayload,
};
use apxm_core::events::{ApxmEvent, EventSource};
use apxm_core::paths::ApxmPaths;
use apxm_core::types::{
    AISOperationType, CommunicateProtocol, OrchestrationStartStatus, OrchestrationTransport,
    OrchestrationWakeOutcome, OrchestrationWorkspaceCleanup, OrchestrationWorkspaceMode,
};
use apxm_runtime::{
    AGENT_ROUTE_CAPABILITIES, AgentRouteCandidate, AgentRouteDecision, AgentRouteSource,
    AgentRouteTarget, AgentRouter, AgentRoutingError,
};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::error::ApiError;
use crate::executions::{ExecutionRecord, ExecutionStatus};
use crate::goal_runs::GoalRunStatus;
use crate::goals::{
    GoalCancelResponse, GoalEventsResponse, GoalStatusResponse, goal_cancel_for_state,
    goal_events_for_state, goal_status_for_state,
};
use crate::helpers::mcp_tool_result;
use crate::state::AppState;

use super::workflow::{WorkflowOrchestrationContract, WorkflowStartArgs, start_workflow_from_args};

pub(crate) const MCP_TOOL_APXM_GOAL_START: &str = mcp_tool_names::APXM_GOAL_START;
pub(crate) const MCP_TOOL_APXM_GOAL_STATUS: &str = mcp_tool_names::APXM_GOAL_STATUS;
pub(crate) const MCP_TOOL_APXM_GOAL_EVENTS: &str = mcp_tool_names::APXM_GOAL_EVENTS;
pub(crate) const MCP_TOOL_APXM_GOAL_CANCEL: &str = mcp_tool_names::APXM_GOAL_CANCEL;

const MAX_WORKERS: usize = 16;
const TEMPLATE_GOAL_WORKER: &str = "goal_worker";
const TEMPLATE_GOAL_SUPERVISOR: &str = "goal_supervisor";
const TEMPLATE_GOAL_TRACKING: &str = "goal_tracking";
const TEMPLATE_GOAL_CONTROLLER: &str = "goal_controller";
const TEMPLATE_GOAL_FLOWCHART: &str = "goal_flowchart";
const TEMPLATE_GOAL_REPORT_STUB: &str = "goal_report_stub";
const TEMPLATE_GOAL_WORKER_ROLE: &str = "goal_worker_role";
const TEMPLATE_GOAL_DEFAULT_WORKER: &str = "goal_default_worker_instructions";
const TEMPLATE_GOAL_DEFAULT_SUPERVISOR: &str = "goal_default_supervisor_instructions";
const DEFAULT_AUTO_PLAN_MAX_WORKERS: usize = 8;
const GOAL_PLANNER_TIMEOUT_MS: u64 = 30_000;
const GOAL_PLANNER_MAX_TOKENS: usize = 4096;
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalStartArgs {
    task: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
    #[serde(default)]
    workers: Option<Vec<WorkerSpec>>,
    #[serde(default)]
    planning: Option<GoalPlanningSpec>,
    #[serde(default)]
    supervisor: Option<SupervisorSpec>,
    #[serde(default)]
    selection: Option<GoalSelectionSpec>,
    #[serde(default)]
    workspace: Option<WorkspaceSpec>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    admit_capabilities: Vec<String>,
    #[serde(default)]
    imports: Vec<String>,
    /// Zero-based index of this bounded pass within a goal. Public callers
    /// always start at zero; the server-owned supervisor increments this for
    /// follow-up passes.
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
struct GoalSelectionSpec {
    #[serde(default)]
    agents: Option<String>,
    #[serde(default)]
    require_agents: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalPlanningSpec {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    max_workers: Option<usize>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    backend: Option<String>,
}

impl GoalStartArgs {
    fn workers(&self) -> &[WorkerSpec] {
        self.workers.as_deref().unwrap_or(&[])
    }

    fn workers_mut(&mut self) -> &mut Vec<WorkerSpec> {
        self.workers
            .as_mut()
            .expect("goal planning must prepare workers before mutation")
    }
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
    #[serde(default)]
    required_capabilities: Vec<String>,
    #[serde(default)]
    preferred_profiles: Vec<String>,
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

#[derive(Debug, Clone, Serialize)]
struct GoalStartResponse {
    status: OrchestrationStartStatus,
    goal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_id: Option<String>,
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_dir: Option<String>,
    workflow_path: String,
    bundle_dir: String,
    artifacts: GoalArtifacts,
    plan: GoalPlanSummary,
    planning: GoalPlanningSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection: Option<GoalSelectionSummary>,
    control: GoalControl,
    goal: GoalRuntimeContract,
    sleep_wake: SleepWakeContract,
    goal_prompt: String,
    flowchart: String,
}

#[derive(Debug, Clone, Serialize)]
struct GoalPlanSummary {
    task: String,
    workers: Vec<WorkerPlanSummary>,
    supervisor: SupervisorPlanSummary,
    workspace_mode: OrchestrationWorkspaceMode,
}

#[derive(Debug, Clone, Serialize)]
struct GoalPlanningSummary {
    mode: &'static str,
    planner: &'static str,
    generated: bool,
    worker_count: usize,
    max_workers: usize,
    reason: String,
}

#[derive(Debug, Clone, Serialize)]
struct GoalArtifacts {
    tracking_doc: String,
    worker_air_dir: String,
    gate_air: String,
    feedback_air: String,
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

#[derive(Debug, Clone, Serialize)]
struct WorkerPlanSummary {
    id: String,
    role: String,
    transport: OrchestrationTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    required_capabilities: Vec<String>,
    preferred_profiles: Vec<String>,
    route_source: String,
    route_reason: String,
    eligible_profiles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_description: Option<String>,
    depends_on: Vec<String>,
    cwd: String,
    workspace: WorkspaceBindingSummary,
}

#[derive(Debug, Clone, Serialize)]
struct SupervisorPlanSummary {
    id: String,
    transport: OrchestrationTransport,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkspaceBindingSummary {
    mode: OrchestrationWorkspaceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_commit: Option<String>,
    cleanup: OrchestrationWorkspaceCleanup,
}

#[derive(Debug, Clone, Serialize)]
struct GoalControl {
    status_tool: &'static str,
    events_tool: &'static str,
    cancel_tool: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct GoalRuntimeContract {
    goal_id: String,
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

#[derive(Debug, Clone, Serialize)]
struct SleepWakeContract {
    sleep_after_start: bool,
    wake_on: Vec<String>,
    event_loop: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalIdArgs {
    goal_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalEventsArgs {
    goal_id: String,
    #[serde(default)]
    since: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct GoalSelectionSummary {
    agents: String,
    candidates: Vec<AgentRouteCandidate>,
    workers: Vec<WorkerSelectionSummary>,
    backends: Vec<BackendSelectionSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkerSelectionSummary {
    id: String,
    profile: Option<String>,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_source: Option<String>,
    required_capabilities: Vec<String>,
    preferred_profiles: Vec<String>,
    eligible_profiles: Vec<String>,
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BackendSelectionSummary {
    name: String,
    health: String,
}

struct GoalBundle {
    session_id: String,
    bundle_dir: PathBuf,
    workflow_path: PathBuf,
    plan: GoalPlan,
}

struct GoalPassStart {
    iteration: usize,
    response: GoalStartResponse,
    artifacts_value: JsonValue,
    plan_value: JsonValue,
    planning_value: JsonValue,
    selection_value: Option<JsonValue>,
    control_value: JsonValue,
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
    required_capabilities: Vec<String>,
    preferred_profiles: Vec<String>,
    route_source: String,
    route_reason: String,
    eligible_profiles: Vec<String>,
    profile_source: Option<String>,
    profile_description: Option<String>,
    cwd: PathBuf,
    tracking_doc_path: PathBuf,
    air_path: PathBuf,
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
    air_path: PathBuf,
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
        "required": ["task"],
        "properties": {
            "task": {
                "type": "string",
                "description": "Task label and instructions for the server-owned goal run"
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
                "description": "Optional bounded worker workflow. When omitted, APXM creates a bounded worker workflow from task/context/event/trigger before admission. Independent workers run in parallel; depends_on creates fan-in/fan-out phases.",
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
                            "description": "Optional ACP profile from the APXM agent profile registry"
                        },
                        "transport": {
                            "type": "string",
                            "enum": OrchestrationTransport::WIRE_VALUES,
                            "description": "acp spawns a real APXM profile; deterministic writes a local fixture worker"
                        },
                        "depends_on": { "type": "array", "items": { "type": "string" } },
                        "mode": { "type": "string" },
                        "model": { "type": "string" },
                        "required_capabilities": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": AGENT_ROUTE_CAPABILITIES
                            },
                            "description": "Abstract route capabilities this worker needs"
                        },
                        "preferred_profiles": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional APXM profile preferences; APXM validates capability fit before binding"
                        }
                    }
                }
            },
            "planning": {
                "type": "object",
                "additionalProperties": false,
                "description": "Server-owned goal planning policy used when workers are omitted.",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["auto", "model", "static"],
                        "description": "auto lets APXM choose the planner, model requires the runtime model router, static uses the local bounded planner"
                    },
                    "max_workers": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_WORKERS,
                        "description": "Ceiling for server-generated workers; APXM may use fewer"
                    },
                    "model": {
                        "type": "string",
                        "description": "Optional model hint for model-router planning"
                    },
                    "backend": {
                        "type": "string",
                        "description": "Optional backend hint for model-router planning"
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
            "selection": {
                "type": "object",
                "additionalProperties": false,
                "description": "Optional APXM-native worker selection policy. The server binds from resolvable ACP profiles and reports current backend health before materializing the workflow.",
                "properties": {
                    "agents": {
                        "type": "string",
                        "enum": ["auto"],
                        "description": "auto binds workers without explicit profiles to resolvable APXM profiles"
                    },
                    "require_agents": {
                        "type": "boolean",
                        "description": "When true, fail if agent auto-selection cannot bind every unprofiled worker"
                    }
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

pub(crate) fn goal_status_input_schema() -> JsonValue {
    goal_id_input_schema()
}

pub(crate) fn goal_cancel_input_schema() -> JsonValue {
    goal_id_input_schema()
}

pub(crate) fn goal_events_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["goal_id"],
        "properties": {
            "goal_id": { "type": "string" },
            "since": {
                "type": "integer",
                "minimum": 0,
                "description": "Return goal-run events whose run-local sequence is >= since"
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 1000,
                "description": "Maximum events to return"
            }
        }
    })
}

fn goal_id_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["goal_id"],
        "properties": {
            "goal_id": { "type": "string" }
        }
    })
}

pub(crate) async fn call_goal_tool(
    state: &AppState,
    id: &JsonValue,
    tool_name: &str,
    tool_args: &JsonValue,
) -> Option<Json<JsonValue>> {
    match tool_name {
        MCP_TOOL_APXM_GOAL_START => Some(match goal_start(state, tool_args).await {
            Ok(response) => mcp_json_tool_result(id.clone(), response),
            Err(error) => mcp_tool_result(id.clone(), error.message, true),
        }),
        MCP_TOOL_APXM_GOAL_STATUS => Some(match goal_status(state, tool_args) {
            Ok(response) => mcp_json_tool_result(id.clone(), response),
            Err(error) => mcp_tool_result(id.clone(), error.message, true),
        }),
        MCP_TOOL_APXM_GOAL_EVENTS => Some(match goal_events(state, tool_args) {
            Ok(response) => mcp_json_tool_result(id.clone(), response),
            Err(error) => mcp_tool_result(id.clone(), error.message, true),
        }),
        MCP_TOOL_APXM_GOAL_CANCEL => Some(match goal_cancel(state, tool_args) {
            Ok(response) => mcp_json_tool_result(id.clone(), response),
            Err(error) => mcp_tool_result(id.clone(), error.message, true),
        }),
        _ => None,
    }
}

pub(crate) async fn post_goal(
    State(state): State<AppState>,
    Json(tool_args): Json<JsonValue>,
) -> Result<Json<JsonValue>, ApiError> {
    let response = goal_start(&state, &tool_args).await?;
    let value = serde_json::to_value(response).map_err(|error| {
        ApiError::internal_message(format!("failed to serialize goal response: {error}"))
    })?;
    Ok(Json(value))
}

async fn goal_start(
    state: &AppState,
    tool_args: &JsonValue,
) -> Result<GoalStartResponse, ApiError> {
    reject_public_goal_internal_fields(tool_args)?;
    let request: GoalStartArgs = serde_json::from_value(tool_args.clone())
        .map_err(|error| ApiError::bad_request(format!("invalid goal_start arguments: {error}")))?;
    let goal_id = format!("goal-{}", uuid::Uuid::new_v4());
    validate_component_id(&goal_id, "goal_id")?;
    if request.iteration >= request.max_iterations.unwrap_or(1).max(1) {
        return Err(ApiError::bad_request(
            "goal_start iteration must be less than max_iterations",
        ));
    }
    let max_iterations = request.max_iterations.unwrap_or(1).max(1);

    if !request.dry_run {
        if state.goal_runs.get(&goal_id).is_some() {
            return Err(ApiError::bad_request(format!(
                "goal run already exists: {goal_id}"
            )));
        }
        state
            .goal_runs
            .insert_running(goal_id.clone(), request.task.clone(), max_iterations);
        record_goal_run_event(
            state,
            &goal_id,
            ApxmEvent::root(
                ExecutionStartedPayload {
                    execution_id: goal_id.clone(),
                },
                EventSource::Server,
                &goal_id,
            ),
        );
    }

    let pass = match start_goal_pass(state, request.clone(), &goal_id).await {
        Ok(pass) => pass,
        Err(error) => {
            if !request.dry_run {
                finish_goal_run_failure(state, &goal_id, None, error.message.clone());
            }
            return Err(error);
        }
    };
    if !request.dry_run {
        register_goal_pass(state, &goal_id, &pass);
        record_goal_pass_sleep(state, &goal_id, &pass.response);
        if let Some(execution_id) = pass.response.execution_id.clone() {
            spawn_goal_run_supervisor(state.clone(), goal_id, request, execution_id);
        }
    }
    Ok(pass.response)
}

fn reject_public_goal_internal_fields(tool_args: &JsonValue) -> Result<(), ApiError> {
    let Some(args) = tool_args.as_object() else {
        return Ok(());
    };
    for field in ["goal_id", "iteration"] {
        if args.contains_key(field) {
            return Err(ApiError::bad_request(format!(
                "goal_start field '{field}' is server-owned"
            )));
        }
    }
    Ok(())
}

async fn start_goal_pass(
    state: &AppState,
    mut request: GoalStartArgs,
    goal_id: &str,
) -> Result<GoalPassStart, ApiError> {
    let planning = apply_goal_planning(state, &mut request).await?;
    normalize_goal_routing_fields(&mut request)?;
    let selection = apply_goal_selection(state, &mut request).await?;
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

    let bundle = materialize_goal_bundle(&request, selection.as_ref())?;
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
    let artifacts_value = serde_json::to_value(&artifacts).map_err(|error| {
        ApiError::internal_message(format!("failed to serialize goal artifacts: {error}"))
    })?;
    let plan_value = serde_json::to_value(&plan_summary).map_err(|error| {
        ApiError::internal_message(format!("failed to serialize goal plan: {error}"))
    })?;
    let planning_value = serde_json::to_value(&planning).map_err(|error| {
        ApiError::internal_message(format!("failed to serialize goal planning: {error}"))
    })?;
    let selection_value = selection
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| {
            ApiError::internal_message(format!("failed to serialize goal selection: {error}"))
        })?;
    let control_value = serde_json::to_value(&control).map_err(|error| {
        ApiError::internal_message(format!("failed to serialize goal control: {error}"))
    })?;

    let response = GoalStartResponse {
        status,
        goal_id: goal_id.to_string(),
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
        planning,
        selection,
        control,
        goal: goal_runtime_contract(
            goal_id,
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
    Ok(GoalPassStart {
        iteration: request.iteration,
        response,
        artifacts_value,
        plan_value,
        planning_value,
        selection_value,
        control_value,
    })
}

fn goal_status(state: &AppState, tool_args: &JsonValue) -> Result<GoalStatusResponse, ApiError> {
    let args: GoalIdArgs = serde_json::from_value(tool_args.clone()).map_err(|error| {
        ApiError::bad_request(format!("invalid goal_status arguments: {error}"))
    })?;
    goal_status_for_state(state, &args.goal_id)
}

fn goal_events(state: &AppState, tool_args: &JsonValue) -> Result<GoalEventsResponse, ApiError> {
    let args: GoalEventsArgs = serde_json::from_value(tool_args.clone()).map_err(|error| {
        ApiError::bad_request(format!("invalid goal_events arguments: {error}"))
    })?;
    let since = args.since.unwrap_or(0);
    let limit = args.limit.unwrap_or(100).clamp(1, 1000);
    goal_events_for_state(state, &args.goal_id, since, limit)
}

fn goal_cancel(state: &AppState, tool_args: &JsonValue) -> Result<GoalCancelResponse, ApiError> {
    let args: GoalIdArgs = serde_json::from_value(tool_args.clone()).map_err(|error| {
        ApiError::bad_request(format!("invalid goal_cancel arguments: {error}"))
    })?;
    goal_cancel_for_state(state, &args.goal_id)
}

fn register_goal_pass(state: &AppState, goal_id: &str, pass: &GoalPassStart) {
    let response = &pass.response;
    if let Some(execution_id) = &response.execution_id {
        state.goal_runs.set_current_pass(
            goal_id,
            pass.iteration,
            execution_id.clone(),
            response.session_id.clone(),
            response.session_dir.clone(),
            response.workflow_path.clone(),
            response.bundle_dir.clone(),
            pass.artifacts_value.clone(),
            pass.plan_value.clone(),
            pass.planning_value.clone(),
            pass.selection_value.clone(),
            pass.control_value.clone(),
        );
    }
}

fn spawn_goal_run_supervisor(
    state: AppState,
    goal_id: String,
    base_request: GoalStartArgs,
    first_execution_id: String,
) {
    tokio::spawn(async move {
        run_goal_supervisor(state, goal_id, base_request, first_execution_id).await;
    });
}

async fn run_goal_supervisor(
    state: AppState,
    goal_id: String,
    base_request: GoalStartArgs,
    mut execution_id: String,
) {
    loop {
        let Some(record) = watch_goal_pass(&state, &goal_id, &execution_id).await else {
            finish_goal_run_failure(
                &state,
                &goal_id,
                None,
                format!("goal pass disappeared before settlement: {execution_id}"),
            );
            return;
        };
        let goal = record.goal.clone();
        if let Some(goal) = &goal {
            state.goal_runs.set_goal_outcome(&goal_id, goal.clone());
            record_goal_outcome_events(&state, &goal_id, &execution_id, goal);
        }

        if state
            .goal_runs
            .get(&goal_id)
            .is_some_and(|run| run.cancel_requested)
        {
            finish_goal_run_cancelled(&state, &goal_id, goal);
            return;
        }

        match goal_decision_kind(goal.as_ref()) {
            Some("iterate") => {
                let Some(next_iteration) = goal
                    .as_ref()
                    .and_then(|goal| goal.get("decision"))
                    .and_then(|decision| decision.get("next_iteration"))
                    .and_then(JsonValue::as_u64)
                    .map(|value| value as usize)
                else {
                    finish_goal_run_failure(
                        &state,
                        &goal_id,
                        goal,
                        "goal iterate decision omitted next_iteration".to_string(),
                    );
                    return;
                };
                let mut next_request = base_request.clone();
                next_request.iteration = next_iteration;
                next_request.max_iterations = Some(base_request.max_iterations.unwrap_or(1).max(1));
                next_request.session_id =
                    goal_pass_session_id(base_request.session_id.as_deref(), next_iteration);
                next_request.context = Some(next_pass_context(
                    base_request.context.as_deref(),
                    &execution_id,
                    &record,
                    goal.as_ref(),
                ));

                match start_goal_pass(&state, next_request, &goal_id).await {
                    Ok(pass) => {
                        register_goal_pass(&state, &goal_id, &pass);
                        record_goal_pass_sleep(&state, &goal_id, &pass.response);
                        if let Some(next_execution_id) = pass.response.execution_id {
                            execution_id = next_execution_id;
                            continue;
                        }
                        finish_goal_run_failure(
                            &state,
                            &goal_id,
                            goal,
                            "next goal pass did not return an execution_id".to_string(),
                        );
                        return;
                    }
                    Err(error) => {
                        finish_goal_run_failure(&state, &goal_id, goal, error.message);
                        return;
                    }
                }
            }
            Some("converged") => {
                finish_goal_run_success(&state, &goal_id, goal);
                return;
            }
            Some("halted") => {
                finish_goal_run_failure(
                    &state,
                    &goal_id,
                    goal,
                    record
                        .error
                        .clone()
                        .or_else(|| goal_halt_reason(record.goal.as_ref()))
                        .unwrap_or_else(|| "goal halted".to_string()),
                );
                return;
            }
            _ => {
                let message = record
                    .error
                    .clone()
                    .unwrap_or_else(|| "goal pass settled without a runtime decision".to_string());
                finish_goal_run_failure(&state, &goal_id, goal, message);
                return;
            }
        }
    }
}

async fn watch_goal_pass(
    state: &AppState,
    goal_id: &str,
    execution_id: &str,
) -> Option<ExecutionRecord> {
    let mut next_source_seq = 0u64;
    let mut terminal_event_seen = false;
    let mut pass_events = state.run_event_bus.subscribe(execution_id);

    if state.execution_store.get(execution_id).is_none() {
        return None;
    }

    replay_goal_pass_events(
        state,
        goal_id,
        execution_id,
        &mut next_source_seq,
        &mut terminal_event_seen,
    );
    if let Some(record) = settled_goal_pass_record(state, execution_id, terminal_event_seen) {
        return Some(record);
    }

    loop {
        match pass_events.recv().await {
            Ok(event) => {
                mirror_goal_pass_event(
                    state,
                    goal_id,
                    event,
                    &mut next_source_seq,
                    &mut terminal_event_seen,
                );
                if let Some(record) =
                    settled_goal_pass_record(state, execution_id, terminal_event_seen)
                {
                    return Some(record);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                replay_goal_pass_events(
                    state,
                    goal_id,
                    execution_id,
                    &mut next_source_seq,
                    &mut terminal_event_seen,
                );
                if let Some(record) =
                    settled_goal_pass_record(state, execution_id, terminal_event_seen)
                {
                    return Some(record);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return state.execution_store.get(execution_id);
            }
        }
    }
}

fn replay_goal_pass_events(
    state: &AppState,
    goal_id: &str,
    execution_id: &str,
    next_source_seq: &mut u64,
    terminal_event_seen: &mut bool,
) {
    for event in state.run_event_bus.snapshot(execution_id) {
        mirror_goal_pass_event(state, goal_id, event, next_source_seq, terminal_event_seen);
    }
}

fn mirror_goal_pass_event(
    state: &AppState,
    goal_id: &str,
    event: ApxmEvent,
    next_source_seq: &mut u64,
    terminal_event_seen: &mut bool,
) {
    if event.meta.seq < *next_source_seq {
        return;
    }
    *next_source_seq = event.meta.seq.saturating_add(1);
    if is_goal_pass_terminal_event(&event) {
        *terminal_event_seen = true;
    }
    record_goal_run_event(state, goal_id, event);
}

fn settled_goal_pass_record(
    state: &AppState,
    execution_id: &str,
    terminal_event_seen: bool,
) -> Option<ExecutionRecord> {
    let record = state.execution_store.get(execution_id)?;
    if record.status == ExecutionStatus::Running {
        return None;
    }
    if terminal_event_seen {
        return Some(record);
    }
    None
}

fn is_goal_pass_terminal_event(event: &ApxmEvent) -> bool {
    let kind = event.kind();
    [kind::EXECUTE_COMPLETE, kind::ERROR, kind::TURN_ABORTED]
        .iter()
        .any(|terminal| kind == *terminal)
}

fn goal_decision_kind(goal: Option<&JsonValue>) -> Option<&str> {
    goal.and_then(|goal| goal.get("decision"))
        .and_then(|decision| decision.get("decision"))
        .and_then(JsonValue::as_str)
}

fn goal_halt_reason(goal: Option<&JsonValue>) -> Option<String> {
    goal.and_then(|goal| goal.get("decision"))
        .and_then(|decision| decision.get("reason"))
        .and_then(JsonValue::as_str)
        .filter(|reason| !reason.trim().is_empty())
        .map(str::to_string)
}

fn next_pass_context(
    base_context: Option<&str>,
    previous_execution_id: &str,
    record: &ExecutionRecord,
    goal: Option<&JsonValue>,
) -> String {
    let mut parts = Vec::new();
    if let Some(base) = base_context.and_then(non_empty) {
        parts.push(base.to_string());
    }

    let mut previous = vec![format!(
        "Previous pass:\n- execution_id: {previous_execution_id}"
    )];
    previous.push(format!("- workflow_status: {:?}", record.status));
    previous.push(format!("- session_dir: {}", record.session_dir));
    if let Some(goal) = goal {
        if let Some(iteration) = goal.get("iteration").and_then(JsonValue::as_u64) {
            previous.push(format!("- completed_iteration: {iteration}"));
        }
        if let Some(decision) = goal_decision_kind(Some(goal)) {
            previous.push(format!("- runtime_decision: {decision}"));
        }
        if let Some(reason) = goal
            .get("decision")
            .and_then(|decision| decision.get("reason"))
            .or_else(|| {
                goal.get("verdict")
                    .and_then(|verdict| verdict.get("reason"))
            })
            .and_then(JsonValue::as_str)
            .and_then(non_empty)
        {
            previous.push(format!("- reason: {reason}"));
        }
        if let Some(remaining) = goal
            .get("verdict")
            .and_then(|verdict| verdict.get("remaining"))
            .and_then(JsonValue::as_array)
        {
            let remaining = remaining
                .iter()
                .filter_map(JsonValue::as_str)
                .collect::<Vec<_>>();
            if !remaining.is_empty() {
                previous.push(format!("- remaining:\n  - {}", remaining.join("\n  - ")));
            }
        }
    }
    parts.push(previous.join("\n"));
    parts.join("\n\n")
}

fn goal_pass_session_id(base_session_id: Option<&str>, iteration: usize) -> Option<String> {
    let base = base_session_id.and_then(non_empty)?;
    if iteration == 0 {
        return Some(base.to_string());
    }
    let suffix = format!("-pass-{}", iteration + 1);
    let max_base_len = 96usize.saturating_sub(suffix.len()).max(1);
    let mut prefix = base.to_string();
    if prefix.len() > max_base_len {
        prefix = prefix.chars().take(max_base_len).collect();
    }
    Some(format!("{prefix}{suffix}"))
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn finish_goal_run_success(state: &AppState, goal_id: &str, goal: Option<JsonValue>) {
    state
        .goal_runs
        .finish(goal_id, GoalRunStatus::Succeeded, goal, None);
    record_goal_terminal_wake(
        state,
        goal_id,
        kind::GOAL_CONVERGED,
        OrchestrationWakeOutcome::Succeeded,
        "goal converged",
    );
}

fn finish_goal_run_cancelled(state: &AppState, goal_id: &str, goal: Option<JsonValue>) {
    state.goal_runs.finish(
        goal_id,
        GoalRunStatus::Cancelled,
        goal,
        Some("cancelled".to_string()),
    );
    record_goal_terminal_wake(
        state,
        goal_id,
        kind::TURN_ABORTED,
        OrchestrationWakeOutcome::Cancelled,
        "goal cancelled",
    );
}

fn finish_goal_run_failure(
    state: &AppState,
    goal_id: &str,
    goal: Option<JsonValue>,
    message: String,
) {
    state
        .goal_runs
        .finish(goal_id, GoalRunStatus::Failed, goal, Some(message.clone()));
    record_goal_run_event(
        state,
        goal_id,
        ApxmEvent::root(
            ErrorPayload {
                message: message.clone(),
                status: None,
                recoverable: false,
            },
            EventSource::Server,
            goal_id,
        ),
    );
    record_goal_terminal_wake(
        state,
        goal_id,
        kind::ERROR,
        OrchestrationWakeOutcome::Failed,
        &message,
    );
}

fn record_goal_pass_sleep(state: &AppState, goal_id: &str, response: &GoalStartResponse) {
    let Some(execution_id) = response.execution_id.as_ref() else {
        return;
    };
    record_goal_run_event(
        state,
        goal_id,
        ApxmEvent::root(
            OrchestratorSleepPayload {
                execution_id: execution_id.clone(),
                session_id: response.session_id.clone(),
                session_dir: response.session_dir.clone().unwrap_or_default(),
                workflow_path: response.workflow_path.clone(),
                bundle_dir: response.bundle_dir.clone(),
                artifacts: serde_json::to_value(&response.artifacts).unwrap_or(JsonValue::Null),
                plan: serde_json::to_value(&response.plan).unwrap_or(JsonValue::Null),
                control: serde_json::to_value(&response.control).unwrap_or(JsonValue::Null),
                wake_on: response.sleep_wake.wake_on.clone(),
                event_loop: response.sleep_wake.event_loop.to_string(),
            },
            EventSource::Server,
            goal_id,
        ),
    );
}

fn record_goal_outcome_events(
    state: &AppState,
    goal_id: &str,
    execution_id: &str,
    goal: &JsonValue,
) {
    let iteration = goal
        .get("iteration")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default() as usize;
    let max_iterations = goal
        .get("max_iterations")
        .and_then(JsonValue::as_u64)
        .unwrap_or(1) as usize;
    if let Some(verdict) = goal.get("verdict") {
        record_goal_run_event(
            state,
            goal_id,
            ApxmEvent::root(
                GoalGateVerdictPayload {
                    execution_id: execution_id.to_string(),
                    iteration,
                    max_iterations,
                    status: verdict
                        .get("status")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("blocked")
                        .to_string(),
                    reason: verdict
                        .get("reason")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    remaining: verdict
                        .get("remaining")
                        .and_then(JsonValue::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                },
                EventSource::Server,
                goal_id,
            ),
        );
    }

    let Some(decision) = goal.get("decision") else {
        return;
    };
    match decision.get("decision").and_then(JsonValue::as_str) {
        Some("converged") => record_goal_run_event(
            state,
            goal_id,
            ApxmEvent::root(
                GoalConvergedPayload {
                    execution_id: execution_id.to_string(),
                    iteration,
                    reason: decision
                        .get("reason")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("goal converged")
                        .to_string(),
                },
                EventSource::Server,
                goal_id,
            ),
        ),
        Some("iterate") => record_goal_run_event(
            state,
            goal_id,
            ApxmEvent::root(
                GoalNeedsAnotherPassPayload {
                    execution_id: execution_id.to_string(),
                    iteration,
                    next_iteration: decision
                        .get("next_iteration")
                        .and_then(JsonValue::as_u64)
                        .unwrap_or(iteration.saturating_add(1) as u64)
                        as usize,
                    reason: decision
                        .get("reason")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("goal needs another pass")
                        .to_string(),
                },
                EventSource::Server,
                goal_id,
            ),
        ),
        Some("halted") => record_goal_run_event(
            state,
            goal_id,
            ApxmEvent::root(
                GoalHaltedPayload {
                    execution_id: execution_id.to_string(),
                    iteration,
                    reason: decision
                        .get("reason")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("goal halted")
                        .to_string(),
                    exhausted: decision
                        .get("exhausted")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false),
                },
                EventSource::Server,
                goal_id,
            ),
        ),
        _ => {}
    }
}

fn record_goal_terminal_wake(
    state: &AppState,
    goal_id: &str,
    terminal_event: apxm_core::events::EventKind,
    outcome: OrchestrationWakeOutcome,
    reason: &str,
) {
    record_goal_run_event(
        state,
        goal_id,
        ApxmEvent::root(
            OrchestratorWakePayload {
                execution_id: goal_id.to_string(),
                session_id: goal_id.to_string(),
                terminal_event: terminal_event.name().to_string(),
                outcome: outcome.as_str().to_string(),
                reason: reason.to_string(),
            },
            EventSource::Server,
            goal_id,
        ),
    );
}

fn record_goal_run_event(state: &AppState, goal_id: &str, event: ApxmEvent) {
    let event = state.run_event_bus.record(goal_id, event);
    if let Some(dispatcher) = &state.webhook_dispatcher {
        dispatcher.dispatch(event);
    }
}

fn materialize_goal_bundle(
    request: &GoalStartArgs,
    selection: Option<&GoalSelectionSummary>,
) -> Result<GoalBundle, ApiError> {
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
    let plan = build_plan(
        request,
        &session_id,
        &bundle_dir,
        &workspace_policy,
        selection,
    )?;
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
    if request.workers().is_empty() {
        return Err(ApiError::bad_request(
            "goal_start workers must be non-empty",
        ));
    }
    if request.workers().len() > MAX_WORKERS {
        return Err(ApiError::bad_request(format!(
            "goal_start workers exceeds breadth cap of {MAX_WORKERS}"
        )));
    }

    let mut ids = HashSet::new();
    for worker in request.workers() {
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
        validate_text_list(
            &worker.required_capabilities,
            "worker.required_capabilities",
        )?;
        normalize_route_capabilities(
            &worker.required_capabilities,
            "worker.required_capabilities",
        )?;
        validate_text_list(&worker.preferred_profiles, "worker.preferred_profiles")?;
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
    validate_dependencies(request.workers())?;
    Ok(())
}

fn normalize_goal_routing_fields(request: &mut GoalStartArgs) -> Result<(), ApiError> {
    for worker in request.workers_mut() {
        worker.required_capabilities = normalize_route_capabilities(
            &worker.required_capabilities,
            "worker.required_capabilities",
        )?;
        worker.preferred_profiles = normalize_profile_names(&worker.preferred_profiles);
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalPlannerProposal {
    #[serde(default)]
    reason: Option<String>,
    workers: Vec<GoalPlannerWorkerProposal>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalPlannerWorkerProposal {
    id: String,
    role: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    required_capabilities: Vec<String>,
    #[serde(default)]
    preferred_profiles: Vec<String>,
}

struct PlannedWorkers {
    workers: Vec<WorkerSpec>,
    planner: &'static str,
    reason: String,
}

async fn apply_goal_planning(
    state: &AppState,
    request: &mut GoalStartArgs,
) -> Result<GoalPlanningSummary, ApiError> {
    let mode = request
        .planning
        .as_ref()
        .and_then(|planning| planning.mode.as_deref())
        .unwrap_or("auto")
        .trim();
    if !matches!(mode, "" | "auto" | "model" | "static") {
        return Err(ApiError::bad_request(
            "goal_start planning.mode must be 'auto', 'model', or 'static'",
        ));
    }
    let mode = if mode.is_empty() { "auto" } else { mode };

    let requested_max = request
        .planning
        .as_ref()
        .and_then(|planning| planning.max_workers)
        .unwrap_or(DEFAULT_AUTO_PLAN_MAX_WORKERS);
    if !(1..=MAX_WORKERS).contains(&requested_max) {
        return Err(ApiError::bad_request(format!(
            "goal_start planning.max_workers must be between 1 and {MAX_WORKERS}"
        )));
    }
    let max_workers = requested_max;

    let Some(workers) = request.workers.as_ref() else {
        let planned = match mode {
            "static" => static_goal_planning(request, max_workers)?,
            "model" => model_goal_planning(state, request, max_workers).await?,
            "auto" => match model_goal_planning(state, request, max_workers).await {
                Ok(planned) => planned,
                Err(error) => {
                    tracing::warn!(
                        error = %error.message,
                        "goal model planner unavailable; using static APXM planner"
                    );
                    static_goal_planning(request, max_workers)?
                }
            },
            _ => unreachable!("validated planning mode"),
        };
        let worker_count = planned.workers.len();
        request.workers = Some(planned.workers);
        return Ok(GoalPlanningSummary {
            mode: match mode {
                "model" => "model",
                "static" => "static",
                _ => "auto",
            },
            planner: planned.planner,
            generated: true,
            worker_count,
            max_workers,
            reason: planned.reason,
        });
    };

    if workers.is_empty() {
        return Err(ApiError::bad_request(
            "goal_start workers must be non-empty when provided",
        ));
    }

    if workers.len() > MAX_WORKERS {
        return Err(ApiError::bad_request(format!(
            "goal_start workers exceeds breadth cap of {MAX_WORKERS}"
        )));
    }

    Ok(GoalPlanningSummary {
        mode: "explicit",
        planner: "caller",
        generated: false,
        worker_count: workers.len(),
        max_workers,
        reason: "caller supplied an explicit bounded worker workflow".to_string(),
    })
}

fn static_goal_planning(
    request: &GoalStartArgs,
    max_workers: usize,
) -> Result<PlannedWorkers, ApiError> {
    let workers = auto_goal_workers(request, max_workers)?;
    Ok(PlannedWorkers {
        workers,
        planner: "static",
        reason: "workers were omitted; APXM generated a bounded static worker workflow from task context"
            .to_string(),
    })
}

async fn model_goal_planning(
    state: &AppState,
    request: &GoalStartArgs,
    max_workers: usize,
) -> Result<PlannedWorkers, ApiError> {
    let router = state.runtime.model_router().ok_or_else(|| {
        ApiError::bad_request(
            "goal_start planning.mode=model requires APXM server runtime ModelRouter",
        )
    })?;
    if state.runtime.llm_registry().backend_names().is_empty() {
        return Err(ApiError::bad_request(
            "goal_start planning.mode=model requires at least one configured LLM backend",
        ));
    }

    let agent_candidates = discover_goal_agent_candidates(state).await;
    let planning = request.planning.as_ref();
    let mut llm_request =
        LLMRequest::new(goal_planner_prompt(request, max_workers, &agent_candidates))
            .with_output_schema(goal_planner_output_schema(max_workers))
            .with_max_tokens(GOAL_PLANNER_MAX_TOKENS)
            .with_temperature(0.1)
            .with_operation_type(AISOperationType::Plan)
            .with_trace_id(format!("goal-plan-{}", uuid::Uuid::new_v4()));
    if let Some(model) = planning
        .and_then(|planning| planning.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        llm_request = llm_request.with_model(model.to_string());
    }
    if let Some(backend) = planning
        .and_then(|planning| planning.backend.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        llm_request = llm_request.with_backend(backend.to_string());
    }

    let response = match tokio::time::timeout(
        Duration::from_millis(GOAL_PLANNER_TIMEOUT_MS),
        router.generate(llm_request),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            return Err(ApiError::bad_request(format!(
                "goal_start model planner failed: {error}"
            )));
        }
        Err(_) => {
            return Err(ApiError::bad_request(format!(
                "goal_start model planner timed out after {GOAL_PLANNER_TIMEOUT_MS}ms"
            )));
        }
    };

    let value = extract_goal_planner_value(&response.content).map_err(|error| {
        ApiError::bad_request(format!(
            "goal_start model planner returned an invalid structured proposal: {error}"
        ))
    })?;
    let proposal: GoalPlannerProposal = serde_json::from_value(value).map_err(|error| {
        ApiError::bad_request(format!(
            "goal_start model planner proposal does not match schema: {error}"
        ))
    })?;
    let allowed_profiles = agent_candidates
        .iter()
        .map(|candidate| candidate.profile.clone())
        .collect::<HashSet<_>>();
    let workers = model_planner_workers(proposal.workers, max_workers, &allowed_profiles)?;
    Ok(PlannedWorkers {
        workers,
        planner: "model_router",
        reason: proposal.reason.unwrap_or_else(|| {
            "workers were omitted; APXM model planner proposed a bounded worker workflow"
                .to_string()
        }),
    })
}

fn goal_planner_prompt(
    request: &GoalStartArgs,
    max_workers: usize,
    agent_candidates: &[AgentRouteCandidate],
) -> String {
    let mut prompt = format!(
        "You are the APXM goal planner. Analyze the user task and propose one bounded worker workflow for the next APXM goal pass.\n\
         Return only the structured proposal requested by the runtime. APXM will validate the proposal before execution.\n\n\
         Rules:\n\
         - Use between 1 and {max_workers} workers.\n\
         - Worker ids must be stable ASCII identifiers using letters, digits, '_' or '-'.\n\
         - Keep every worker focused and independently executable.\n\
         - Use depends_on only for true ordering requirements; independent workers should be parallel.\n\
         - Do not include transports, credentials, shell commands, or tool calls.\n\
         - Use required_capabilities for abstract needs such as read, write, execute, critique, workflow_author.\n\
         - preferred_profiles is optional. When used, choose only profile names from the APXM inventory below; APXM still validates capabilities before binding.\n\
         - Include verification/review work when the task requires changes or high confidence.\n\
         - If the task needs another pass later, workers should report concrete remaining work to the gate.\n\n\
         Task:\n{}\n",
        request.task
    );
    if !agent_candidates.is_empty() {
        prompt.push_str("\nAPXM agent inventory (sanitized; no commands or credentials):\n");
        for candidate in agent_candidates {
            prompt.push_str(&format!(
                "- profile={} source={} route_capabilities=[{}] description={}\n",
                candidate.profile,
                candidate.source.as_deref().unwrap_or("unknown"),
                candidate.capabilities.join(", "),
                candidate.description.as_deref().unwrap_or("")
            ));
        }
    }
    if let Some(context) = request
        .context
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        prompt.push_str("\nContext:\n");
        prompt.push_str(context);
        prompt.push('\n');
    }
    if let Some(event) = request
        .event
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        prompt.push_str("\nEvent:\n");
        prompt.push_str(event);
        prompt.push('\n');
    }
    if let Some(trigger) = request
        .trigger
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        prompt.push_str("\nTrigger:\n");
        prompt.push_str(trigger);
        prompt.push('\n');
    }
    if request.iteration > 0 {
        prompt.push_str(&format!(
            "\nThis is pass {} of the goal.\n",
            request.iteration
        ));
    }
    prompt
}

fn goal_planner_output_schema(max_workers: usize) -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["workers"],
        "properties": {
            "reason": {
                "type": "string",
                "description": "Short explanation of the chosen worker split"
            },
            "workers": {
                "type": "array",
                "minItems": 1,
                "maxItems": max_workers,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "role"],
                    "properties": {
                        "id": { "type": "string" },
                        "role": {
                            "type": "string",
                            "description": "Concise role and acceptance criteria for this worker"
                        },
                        "prompt": {
                            "type": "string",
                            "description": "Optional detailed worker instructions"
                        },
                        "depends_on": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "required_capabilities": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": AGENT_ROUTE_CAPABILITIES
                            },
                            "description": "Abstract worker route capabilities needed for this role"
                        },
                        "preferred_profiles": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional APXM profile names from the sanitized inventory"
                        }
                    }
                }
            }
        }
    })
}

fn model_planner_workers(
    proposed: Vec<GoalPlannerWorkerProposal>,
    max_workers: usize,
    allowed_profiles: &HashSet<String>,
) -> Result<Vec<WorkerSpec>, ApiError> {
    if proposed.is_empty() {
        return Err(ApiError::bad_request(
            "goal_start model planner returned no workers",
        ));
    }
    if proposed.len() > max_workers {
        return Err(ApiError::bad_request(format!(
            "goal_start model planner returned {} workers, exceeding max_workers={max_workers}",
            proposed.len()
        )));
    }
    let mut workers = Vec::with_capacity(proposed.len());
    for worker in proposed {
        validate_component_id(&worker.id, "planner worker.id")?;
        validate_optional_text(Some(worker.role.as_str()), "planner worker.role")?;
        validate_optional_text(worker.prompt.as_deref(), "planner worker.prompt")?;
        validate_text_list(
            &worker.required_capabilities,
            "planner worker.required_capabilities",
        )?;
        let required_capabilities = normalize_route_capabilities(
            &worker.required_capabilities,
            "planner worker.required_capabilities",
        )?;
        validate_text_list(
            &worker.preferred_profiles,
            "planner worker.preferred_profiles",
        )?;
        let preferred_profiles = normalize_profile_names(&worker.preferred_profiles);
        for profile in &preferred_profiles {
            if !allowed_profiles.contains(profile) {
                return Err(ApiError::bad_request(format!(
                    "goal_start model planner preferred unknown APXM profile '{profile}'"
                )));
            }
        }
        workers.push(WorkerSpec {
            id: worker.id,
            role: Some(worker.role),
            prompt: worker.prompt,
            profile: None,
            transport: None,
            depends_on: worker.depends_on,
            mode: None,
            model: None,
            required_capabilities,
            preferred_profiles,
        });
    }
    validate_dependencies(&workers)?;
    Ok(workers)
}

fn extract_goal_planner_value(content: &str) -> Result<JsonValue, String> {
    let trimmed = content.trim();
    if let Ok(value) = serde_json::from_str::<JsonValue>(trimmed) {
        return Ok(value);
    }

    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    if let Ok(value) = serde_json::from_str::<JsonValue>(unfenced) {
        return Ok(value);
    }

    let Some(start) = unfenced.find('{') else {
        return Err("model response contained no structured object".to_string());
    };
    let Some(end) = unfenced.rfind('}') else {
        return Err("model response contained an unterminated structured object".to_string());
    };
    serde_json::from_str(&unfenced[start..=end])
        .map_err(|error| format!("failed to parse structured object from model response: {error}"))
}

fn auto_goal_workers(
    request: &GoalStartArgs,
    max_workers: usize,
) -> Result<Vec<WorkerSpec>, ApiError> {
    let task_text = goal_planning_text(request);
    let complex = contains_any(
        &task_text,
        &[
            "ultra",
            "investigate",
            "architecture",
            "consistent",
            "alignment",
            "migration",
            "migrate",
            "e2e",
            "end to end",
            "whole project",
            "all of it",
        ],
    );
    let needs_research = complex
        || contains_any(
            &task_text,
            &[
                "inspect", "research", "revise", "check", "review", "organize",
            ],
        );
    let needs_docs = contains_any(
        &task_text,
        &["doc", "docs", "documentation", "readme", "skill"],
    );
    let needs_release = contains_any(
        &task_text,
        &["release", "publish", "version", "packag", "ship"],
    );
    let needs_critic = complex
        || contains_any(
            &task_text,
            &["risk", "security", "critic", "dissent", "regression"],
        );

    let mut ids = match max_workers {
        1 => vec!["implement"],
        2 => vec!["implement", "verify"],
        _ => vec!["planner", "implement", "verify"],
    };
    add_auto_worker_id(&mut ids, "research", needs_research, max_workers);
    add_auto_worker_id(&mut ids, "docs", needs_docs, max_workers);
    add_auto_worker_id(&mut ids, "critic", needs_critic, max_workers);
    add_auto_worker_id(&mut ids, "release", needs_release, max_workers);
    let needs_synthesizer = ids.len() >= 5 || (complex && ids.len() >= 4);
    add_auto_worker_id(&mut ids, "synthesizer", needs_synthesizer, max_workers);

    ids.sort_by_key(|id| auto_worker_order(id));
    let included = ids.iter().copied().collect::<HashSet<_>>();
    ids.into_iter()
        .map(|id| auto_goal_worker(id, &included))
        .collect::<Result<Vec<_>, _>>()
}

fn add_auto_worker_id(ids: &mut Vec<&'static str>, id: &'static str, enabled: bool, max: usize) {
    if enabled && ids.len() < max && !ids.contains(&id) {
        ids.push(id);
    }
}

fn auto_goal_worker(id: &'static str, included: &HashSet<&str>) -> Result<WorkerSpec, ApiError> {
    let kind = match id {
        "implement" => "executor",
        "verify" => "verifier",
        "critic" => "reviewer",
        other => other,
    };
    Ok(WorkerSpec {
        id: id.to_string(),
        role: Some(goal_worker_role(kind, id)?),
        prompt: None,
        profile: None,
        transport: None,
        depends_on: auto_worker_dependencies(id, included),
        mode: None,
        model: None,
        required_capabilities: Vec::new(),
        preferred_profiles: Vec::new(),
    })
}

fn auto_worker_dependencies(id: &str, included: &HashSet<&str>) -> Vec<String> {
    let deps: Vec<&str> = match id {
        "research" if included.contains("planner") => vec!["planner"],
        "implement" => {
            let mut deps = Vec::new();
            if included.contains("planner") {
                deps.push("planner");
            }
            if included.contains("research") {
                deps.push("research");
            }
            deps
        }
        "docs" => vec!["implement"],
        "critic" => vec!["implement"],
        "verify" => {
            let mut deps = vec!["implement"];
            if included.contains("docs") {
                deps.push("docs");
            }
            deps
        }
        "release" => {
            let mut deps = vec!["verify"];
            if included.contains("docs") {
                deps.push("docs");
            }
            deps
        }
        "synthesizer" => {
            let mut deps = Vec::new();
            for dep in ["critic", "verify", "release"] {
                if included.contains(dep) {
                    deps.push(dep);
                }
            }
            if deps.is_empty() && included.contains("implement") {
                deps.push("implement");
            }
            deps
        }
        _ => Vec::new(),
    };
    deps.into_iter().map(str::to_string).collect()
}

fn auto_worker_order(id: &&str) -> usize {
    match *id {
        "planner" => 0,
        "research" => 1,
        "implement" => 2,
        "docs" => 3,
        "critic" => 4,
        "verify" => 5,
        "release" => 6,
        "synthesizer" => 7,
        _ => 99,
    }
}

fn goal_planning_text(request: &GoalStartArgs) -> String {
    [
        Some(request.task.as_str()),
        request.context.as_deref(),
        request.event.as_deref(),
        request.trigger.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase()
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn goal_worker_role(kind: &str, worker_id: &str) -> Result<String, ApiError> {
    render_goal_template(
        TEMPLATE_GOAL_WORKER_ROLE,
        &serde_json::json!({
            "kind": kind,
            "worker_id": worker_id
        }),
    )
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

async fn apply_goal_selection(
    state: &AppState,
    request: &mut GoalStartArgs,
) -> Result<Option<GoalSelectionSummary>, ApiError> {
    let Some(selection) = request.selection.as_ref() else {
        return Ok(None);
    };
    let agents_mode = selection.agents.as_deref().unwrap_or("");
    if agents_mode.is_empty() {
        if selection.require_agents {
            return Err(ApiError::bad_request(
                "goal_start selection.require_agents requires selection.agents='auto'",
            ));
        }
        return Ok(Some(GoalSelectionSummary {
            agents: "disabled".to_string(),
            candidates: Vec::new(),
            workers: request
                .workers()
                .iter()
                .map(|worker| {
                    let source = if worker.profile.is_some() {
                        "explicit"
                    } else {
                        "deterministic"
                    };
                    worker_selection_summary(worker, None, &HashMap::new(), source)
                })
                .collect(),
            backends: backend_selection_summary(state),
        }));
    }
    if agents_mode != "auto" {
        return Err(ApiError::bad_request(
            "goal_start selection.agents must be 'auto'",
        ));
    }

    let candidates = discover_goal_agent_candidates(state).await;
    let decisions = bind_goal_agent_selection(request, &candidates, selection.require_agents)?;
    let decisions_by_id: HashMap<String, AgentRouteDecision> = decisions
        .into_iter()
        .map(|decision| (decision.id.clone(), decision))
        .collect();
    let candidates_by_profile = candidates
        .iter()
        .map(|candidate| (candidate.profile.clone(), candidate.clone()))
        .collect::<HashMap<_, _>>();

    Ok(Some(GoalSelectionSummary {
        agents: "auto".to_string(),
        candidates,
        workers: request
            .workers()
            .iter()
            .map(|worker| {
                worker_selection_summary(
                    worker,
                    decisions_by_id.get(&worker.id),
                    &candidates_by_profile,
                    "deterministic",
                )
            })
            .collect(),
        backends: backend_selection_summary(state),
    }))
}

fn bind_goal_agent_selection(
    request: &mut GoalStartArgs,
    candidates: &[AgentRouteCandidate],
    require_agents: bool,
) -> Result<Vec<AgentRouteDecision>, ApiError> {
    let targets = request
        .workers()
        .iter()
        .map(|worker| AgentRouteTarget {
            id: worker.id.clone(),
            profile: worker.profile.clone(),
            mode: worker.mode.clone(),
            model: worker.model.clone(),
            required_capabilities: goal_worker_required_capabilities(worker),
            preferred_profiles: worker.preferred_profiles.clone(),
        })
        .collect::<Vec<_>>();
    let decisions = AgentRouter::new(candidates.to_vec())
        .route_targets(&targets, require_agents)
        .map_err(goal_agent_routing_error)?;
    let decisions_by_id: HashMap<String, AgentRouteDecision> = decisions
        .iter()
        .cloned()
        .map(|decision| (decision.id.clone(), decision))
        .collect();
    let candidates_by_profile = candidates
        .iter()
        .map(|candidate| (candidate.profile.clone(), candidate))
        .collect::<HashMap<_, _>>();
    for worker in request.workers_mut().iter_mut() {
        let Some(decision) = decisions_by_id.get(&worker.id) else {
            continue;
        };
        if decision.source == AgentRouteSource::Selected {
            worker.profile = decision.profile.clone();
            worker.transport = Some(OrchestrationTransport::Acp);
            worker.mode = decision.mode.clone();
            worker.model = decision.model.clone();
        } else if decision.source == AgentRouteSource::Explicit {
            let Some(profile) = worker.profile.as_deref() else {
                continue;
            };
            let Some(candidate) = candidates_by_profile.get(profile) else {
                return Err(ApiError::bad_request(format!(
                    "goal_start explicit APXM profile '{profile}' for worker '{}' is not resolvable; run `dekk apxm agent list` or fix the profile",
                    worker.id
                )));
            };
            if worker.mode.is_none() {
                worker.mode = decision
                    .mode
                    .clone()
                    .or_else(|| candidate.default_mode.clone());
            }
            if worker.model.is_none() {
                worker.model = decision
                    .model
                    .clone()
                    .or_else(|| candidate.default_model.clone());
            }
        }
    }
    Ok(decisions)
}

fn goal_agent_routing_error(error: AgentRoutingError) -> ApiError {
    match error {
        AgentRoutingError::NoCandidates { target_id } => ApiError::bad_request(format!(
            "goal_start selection.agents=auto found no APXM agent profiles with resolvable commands for worker '{target_id}'; run `dekk apxm agent list`, add or fix an agent profile, or omit selection for deterministic workers"
        )),
        AgentRoutingError::UnknownProfile { target_id, profile } => ApiError::bad_request(format!(
            "goal_start worker '{target_id}' requested APXM profile '{profile}', but that profile is not resolvable; run `dekk apxm agent list` or fix the profile"
        )),
        AgentRoutingError::ProfileCapabilityMismatch {
            target_id,
            profile,
            required_capabilities,
            candidate_capabilities,
        } => ApiError::bad_request(format!(
            "goal_start worker '{target_id}' requested APXM profile '{profile}', but it does not provide required capabilities [{}]; profile capabilities are [{}]",
            required_capabilities.join(", "),
            candidate_capabilities.join(", ")
        )),
        AgentRoutingError::NoMatchingCandidates {
            target_id,
            required_capabilities,
            candidate_count,
        } => ApiError::bad_request(format!(
            "goal_start selection.agents=auto found {candidate_count} APXM agent profile(s), but none matched worker '{target_id}' required capabilities [{}]",
            required_capabilities.join(", ")
        )),
    }
}

async fn discover_goal_agent_candidates(state: &AppState) -> Vec<AgentRouteCandidate> {
    let Some(spawner) = state.runtime.process_table().agent_spawner().await else {
        return Vec::new();
    };
    spawner.route_candidates()
}

fn goal_worker_required_capabilities(worker: &WorkerSpec) -> Vec<String> {
    if !worker.required_capabilities.is_empty() {
        return normalize_route_capabilities(
            &worker.required_capabilities,
            "worker.required_capabilities",
        )
        .expect("worker route capabilities are validated before planning");
    }
    let role = worker
        .role
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let prompt = worker
        .prompt
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let text = format!("{} {} {}", worker.id.to_ascii_lowercase(), role, prompt);
    let mut capabilities = Vec::<String>::new();
    if contains_any(
        &text,
        &[
            "planner",
            "plan",
            "graph",
            "architecture",
            "dag",
            "workflow",
        ],
    ) {
        push_capability(&mut capabilities, "read");
        push_capability(&mut capabilities, "workflow_author");
    }
    if contains_any(
        &text,
        &["critic", "critique", "adversarial", "risk", "review"],
    ) {
        push_capability(&mut capabilities, "read");
        push_capability(&mut capabilities, "critique");
    }
    if contains_any(
        &text,
        &[
            "implement",
            "executor",
            "write",
            "edit",
            "fix",
            "migrate",
            "migration",
            "docs",
            "documentation",
        ],
    ) {
        push_capability(&mut capabilities, "write");
        push_capability(&mut capabilities, "execute");
    }
    if contains_any(
        &text,
        &["execute", "verify", "verifier", "test", "release", "ship"],
    ) {
        push_capability(&mut capabilities, "execute");
    }
    if capabilities.is_empty() {
        push_capability(&mut capabilities, "read");
    }
    capabilities
}

fn push_capability(capabilities: &mut Vec<String>, capability: &str) {
    if !capabilities.iter().any(|value| value == capability) {
        capabilities.push(capability.to_string());
    }
}

fn worker_selection_summary(
    worker: &WorkerSpec,
    decision: Option<&AgentRouteDecision>,
    candidates: &HashMap<String, AgentRouteCandidate>,
    default_source: &'static str,
) -> WorkerSelectionSummary {
    let profile_candidate = worker
        .profile
        .as_ref()
        .and_then(|profile| candidates.get(profile));
    WorkerSelectionSummary {
        id: worker.id.clone(),
        profile: worker.profile.clone(),
        source: decision
            .map(|decision| decision.source.as_str())
            .unwrap_or(default_source),
        profile_source: profile_candidate.and_then(|candidate| candidate.source.clone()),
        required_capabilities: decision
            .map(|decision| decision.required_capabilities.clone())
            .unwrap_or_else(|| goal_worker_required_capabilities(worker)),
        preferred_profiles: decision
            .map(|decision| decision.preferred_profiles.clone())
            .unwrap_or_else(|| worker.preferred_profiles.clone()),
        eligible_profiles: decision
            .map(|decision| decision.eligible_profiles.clone())
            .unwrap_or_default(),
        reason: decision
            .map(|decision| decision.reason.clone())
            .unwrap_or_else(|| "selection disabled".to_string()),
        mode: worker.mode.clone(),
        model: worker.model.clone(),
        profile_description: profile_candidate.and_then(|candidate| candidate.description.clone()),
    }
}

fn backend_selection_summary(state: &AppState) -> Vec<BackendSelectionSummary> {
    let registry = state.runtime.llm_registry();
    registry
        .backend_names()
        .into_iter()
        .map(|name| BackendSelectionSummary {
            health: health_status_label(registry.backend_health(&name)).to_string(),
            name,
        })
        .collect()
}

fn health_status_label(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Degraded => "degraded",
        HealthStatus::Unhealthy => "unhealthy",
        HealthStatus::Unknown => "unknown",
    }
}

fn validate_optional_text(value: Option<&str>, field: &str) -> Result<(), ApiError> {
    if value.is_some_and(|text| text.contains('\0')) {
        return Err(ApiError::bad_request(format!(
            "{field} must not contain NUL"
        )));
    }
    Ok(())
}

fn validate_text_list(values: &[String], field: &str) -> Result<(), ApiError> {
    for value in values {
        validate_optional_text(Some(value.as_str()), field)?;
    }
    Ok(())
}

fn normalize_route_capabilities(values: &[String], field: &str) -> Result<Vec<String>, ApiError> {
    let mut normalized = values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    for capability in &normalized {
        if !AGENT_ROUTE_CAPABILITIES.contains(&capability.as_str()) {
            return Err(ApiError::bad_request(format!(
                "{field} contains unsupported route capability '{capability}'. Use one of: {}",
                AGENT_ROUTE_CAPABILITIES.join(", ")
            )));
        }
    }
    Ok(normalized)
}

fn normalize_profile_names(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        normalized.push(value.to_string());
    }
    normalized
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
    selection: Option<&GoalSelectionSummary>,
) -> Result<GoalPlan, ApiError> {
    let mut workers = Vec::with_capacity(request.workers().len());
    let tracking_doc_path = bundle_dir.join("goal.md");
    let workers_dir = bundle_dir.join("workers");
    for worker in request.workers() {
        let (cwd, workspace) = workspace_policy.allocate(&worker.id)?;
        let transport = effective_transport(worker.transport, worker.profile.as_deref());
        let default_source = if worker.profile.is_some() {
            "explicit"
        } else {
            "deterministic"
        };
        let route = selection
            .and_then(|selection| {
                selection
                    .workers
                    .iter()
                    .find(|summary| summary.id == worker.id)
            })
            .cloned()
            .unwrap_or_else(|| {
                worker_selection_summary(worker, None, &HashMap::new(), default_source)
            });
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
            required_capabilities: route.required_capabilities,
            preferred_profiles: route.preferred_profiles,
            route_source: route.source.to_string(),
            route_reason: route.reason,
            eligible_profiles: route.eligible_profiles,
            profile_source: route.profile_source,
            profile_description: route.profile_description,
            cwd,
            tracking_doc_path: tracking_doc_path.clone(),
            air_path: workers_dir.join(format!("{}.air", worker.id)),
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
            air_path: bundle_dir.join("gate.air"),
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
        std::fs::write(&worker.air_path, air).map_err(|error| {
            ApiError::internal_message(format!(
                "failed to write worker AIR '{}': {error}",
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
                &worker.air_path,
            )?,
            "worker report stub",
        )?;
    }

    std::fs::write(bundle_dir.join("gate.air"), gate_air(request, plan)?).map_err(|error| {
        ApiError::internal_message(format!("failed to write gate AIR: {error}"))
    })?;
    std::fs::write(bundle_dir.join("feedback.air"), feedback_air()).map_err(|error| {
        ApiError::internal_message(format!("failed to write feedback AIR: {error}"))
    })?;
    std::fs::write(
        bundle_dir.join("workflow.apxmw"),
        workflow_manifest(request, plan)?,
    )
    .map_err(|error| ApiError::internal_message(format!("failed to write workflow: {error}")))?;
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
            &plan.supervisor.air_path,
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

fn goal_artifacts(bundle_dir: &Path, plan: &GoalPlan) -> GoalArtifacts {
    GoalArtifacts {
        tracking_doc: path_string(&bundle_dir.join("goal.md")),
        worker_air_dir: path_string(&bundle_dir.join("workers")),
        gate_air: path_string(&bundle_dir.join("gate.air")),
        feedback_air: path_string(&bundle_dir.join("feedback.air")),
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
            "worker_air_dir": path_string(&bundle_dir.join("workers")),
            "gate_air": path_string(&bundle_dir.join("gate.air")),
            "feedback_air": path_string(&bundle_dir.join("feedback.air")),
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
    air_path: &Path,
) -> Result<String, ApiError> {
    render_goal_template(
        TEMPLATE_GOAL_REPORT_STUB,
        &serde_json::json!({
            "owner": {
                "id": owner_id,
                "kind": owner_kind,
                "prompt_path": path_string(prompt_path),
                "tracking_doc": path_string(tracking_doc_path),
                "air_path": path_string(air_path)
            }
        }),
    )
}

fn worker_prompt_context(worker: &WorkerPlan) -> JsonValue {
    serde_json::json!({
        "id": worker.id.as_str(),
        "role": worker.role.as_str(),
        "transport": worker.transport.as_str(),
        "profile": worker.profile.as_deref(),
        "cwd": path_string(&worker.cwd),
        "tracking_doc": path_string(&worker.tracking_doc_path),
        "air_path": path_string(&worker.air_path),
        "prompt_path": path_string(&worker.prompt_path),
        "report_path": path_string(&worker.report_path),
        "depends_on": &worker.depends_on,
        "required_capabilities": &worker.required_capabilities,
        "preferred_profiles": &worker.preferred_profiles,
        "route_source": worker.route_source.as_str(),
        "route_reason": worker.route_reason.as_str(),
        "eligible_profiles": &worker.eligible_profiles,
        "profile_source": worker.profile_source.as_deref(),
        "profile_description": worker.profile_description.as_deref(),
        "mode": worker.mode.as_deref(),
        "model": worker.model.as_deref(),
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
                "profile": worker.profile.as_deref().unwrap_or("deterministic"),
                "mode": worker.mode.as_deref().unwrap_or(""),
                "model": worker.model.as_deref().unwrap_or(""),
                "required_capabilities": worker.required_capabilities.join(", "),
                "preferred_profiles": worker.preferred_profiles.join(", "),
                "route_source": worker.route_source.as_str(),
                "route_reason": worker.route_reason.as_str(),
                "eligible_profiles": worker.eligible_profiles.join(", "),
                "profile_source": worker.profile_source.as_deref().unwrap_or(""),
                "profile_description": worker.profile_description.as_deref().unwrap_or(""),
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
        "air_path": path_string(&supervisor.air_path),
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
        &worker.required_capabilities,
        &worker.preferred_profiles,
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
            &[],
            &[],
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

fn workflow_manifest(request: &GoalStartArgs, plan: &GoalPlan) -> Result<Vec<u8>, ApiError> {
    let mut steps = Vec::with_capacity(plan.workers.len() + 2);
    for worker in &plan.workers {
        let upstream = upstream_template(&worker.depends_on);
        steps.push(serde_json::json!({
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
    steps.push(serde_json::json!({
        "id": plan.supervisor.id,
        "path": "gate.air",
        "depends_on": worker_ids,
        "params": {
            "summary": worker_summary
        }
    }));
    steps.push(serde_json::json!({
        "id": "feedback",
        "path": "feedback.air",
        "depends_on": [plan.supervisor.id.clone()],
        "params": {
            "gate": format!("{{{{{}.output}}}}", plan.supervisor.id)
        }
    }));

    serde_json::to_vec_pretty(&serde_json::json!({
        "name": "goal_pass",
        "description": "Generated by goal_start: event -> trigger -> parallel workers -> gate/eval -> feedback.",
        "steps": steps,
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
    required_capabilities: &[String],
    preferred_profiles: &[String],
) -> String {
    let mut attrs = Vec::new();
    if let Some(profile) = profile {
        attrs.push(format!("profile = {}", quote_air(profile)));
    }
    if !required_capabilities.is_empty() {
        attrs.push(format!(
            "required_capabilities = {}",
            quote_air_string_array(required_capabilities)
        ));
    }
    if !preferred_profiles.is_empty() {
        attrs.push(format!(
            "preferred_profiles = {}",
            quote_air_string_array(preferred_profiles)
        ));
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

fn quote_air_string_array(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| quote_air(value))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
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
    let compact = session_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    let suffix = if compact.chars().count() > 24 {
        let head = compact.chars().take(12).collect::<String>();
        let tail = compact
            .chars()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        format!("{head}{tail}")
    } else {
        compact
    };
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
    request.workers().iter().any(|worker| {
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
                    mode: worker.mode.clone(),
                    model: worker.model.clone(),
                    required_capabilities: worker.required_capabilities.clone(),
                    preferred_profiles: worker.preferred_profiles.clone(),
                    route_source: worker.route_source.clone(),
                    route_reason: worker.route_reason.clone(),
                    eligible_profiles: worker.eligible_profiles.clone(),
                    profile_source: worker.profile_source.clone(),
                    profile_description: worker.profile_description.clone(),
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
        status_tool: MCP_TOOL_APXM_GOAL_STATUS,
        events_tool: MCP_TOOL_APXM_GOAL_EVENTS,
        cancel_tool: MCP_TOOL_APXM_GOAL_CANCEL,
    }
}

fn goal_terminal_event_kinds() -> Vec<&'static str> {
    vec![
        kind::ORCHESTRATOR_WAKE.name(),
        kind::ERROR.name(),
        kind::TURN_ABORTED.name(),
    ]
}

fn goal_wake_on() -> Vec<String> {
    vec![
        format!(
            "{} returns {}",
            MCP_TOOL_APXM_GOAL_EVENTS,
            kind::ORCHESTRATOR_WAKE.name()
        ),
        format!(
            "{} returns {} or {}",
            MCP_TOOL_APXM_GOAL_EVENTS,
            kind::ERROR.name(),
            kind::TURN_ABORTED.name()
        ),
        format!("{MCP_TOOL_APXM_GOAL_STATUS} reports succeeded, failed, or cancelled"),
        format!(
            "{} is called by the supervisor/client",
            MCP_TOOL_APXM_GOAL_CANCEL
        ),
    ]
}

fn goal_event_loop() -> &'static str {
    "event -> trigger -> parallel worker actions -> gate/eval -> feedback -> next event"
}

fn goal_runtime_contract(
    goal_id: &str,
    execution_id: Option<String>,
    gate_step_id: &str,
) -> GoalRuntimeContract {
    let next_events_args = Some(serde_json::json!({
        "goal_id": goal_id,
        "since": 0,
        "limit": 100
    }));
    GoalRuntimeContract {
        goal_id: goal_id.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_workers(workers: Vec<WorkerSpec>) -> GoalStartArgs {
        GoalStartArgs {
            task: "ship".to_string(),
            context: None,
            event: None,
            trigger: None,
            workers: Some(workers),
            planning: None,
            supervisor: None,
            selection: None,
            workspace: None,
            session_id: None,
            admit_capabilities: Vec::new(),
            imports: Vec::new(),
            iteration: 0,
            max_iterations: None,
            dry_run: true,
        }
    }

    fn worker(id: &str, profile: Option<&str>) -> WorkerSpec {
        WorkerSpec {
            id: id.to_string(),
            role: None,
            prompt: None,
            profile: profile.map(str::to_string),
            transport: None,
            depends_on: Vec::new(),
            mode: None,
            model: None,
            required_capabilities: Vec::new(),
            preferred_profiles: Vec::new(),
        }
    }

    #[test]
    fn goal_agent_selection_binds_unprofiled_workers_round_robin() {
        let mut request = request_with_workers(vec![
            worker("planner", None),
            worker("executor", Some("explicit")),
            worker("verifier", None),
        ]);
        let candidates = vec![
            AgentRouteCandidate {
                profile: "agent-a".to_string(),
                description: None,
                source: None,
                executable: "agent-a".to_string(),
                capabilities: vec![
                    "read".to_string(),
                    "workflow_author".to_string(),
                    "execute".to_string(),
                ],
                default_mode: Some("architect".to_string()),
                default_model: Some("model-a".to_string()),
            },
            AgentRouteCandidate {
                profile: "agent-b".to_string(),
                description: None,
                source: None,
                executable: "agent-b".to_string(),
                capabilities: vec![
                    "read".to_string(),
                    "workflow_author".to_string(),
                    "execute".to_string(),
                ],
                default_mode: None,
                default_model: Some("model-b".to_string()),
            },
            AgentRouteCandidate {
                profile: "explicit".to_string(),
                description: None,
                source: None,
                executable: "explicit".to_string(),
                capabilities: apxm_acp::default_route_capabilities(),
                default_mode: None,
                default_model: None,
            },
        ];

        bind_goal_agent_selection(&mut request, &candidates, true).expect("selection");

        assert_eq!(request.workers()[0].profile.as_deref(), Some("agent-a"));
        assert_eq!(
            request.workers()[0].transport,
            Some(OrchestrationTransport::Acp)
        );
        assert_eq!(request.workers()[0].mode.as_deref(), Some("architect"));
        assert_eq!(request.workers()[0].model.as_deref(), Some("model-a"));
        assert_eq!(request.workers()[1].profile.as_deref(), Some("explicit"));
        assert_eq!(request.workers()[2].profile.as_deref(), Some("agent-b"));
        assert_eq!(request.workers()[2].model.as_deref(), Some("model-b"));
    }

    #[test]
    fn goal_agent_selection_can_require_real_agents() {
        let mut request = request_with_workers(vec![worker("planner", None)]);
        let error = bind_goal_agent_selection(&mut request, &[], true).expect_err("must fail");
        assert!(
            error
                .message
                .contains("found no APXM agent profiles with resolvable commands")
        );

        let decisions =
            bind_goal_agent_selection(&mut request, &[], false).expect("optional selection");
        assert!(request.workers()[0].profile.is_none());
        assert_eq!(decisions[0].source, AgentRouteSource::Deterministic);
    }

    #[test]
    fn goal_agent_selection_routes_by_worker_capability() {
        let mut request = request_with_workers(vec![
            WorkerSpec {
                id: "planner".to_string(),
                role: Some("Plan the workflow".to_string()),
                prompt: None,
                profile: None,
                transport: None,
                depends_on: Vec::new(),
                mode: None,
                model: None,
                required_capabilities: Vec::new(),
                preferred_profiles: Vec::new(),
            },
            WorkerSpec {
                id: "verifier".to_string(),
                role: Some("Run tests".to_string()),
                prompt: None,
                profile: None,
                transport: None,
                depends_on: Vec::new(),
                mode: None,
                model: None,
                required_capabilities: Vec::new(),
                preferred_profiles: Vec::new(),
            },
        ]);
        let candidates = vec![
            AgentRouteCandidate {
                profile: "reader".to_string(),
                description: None,
                source: None,
                executable: "reader".to_string(),
                capabilities: vec!["read".to_string(), "workflow_author".to_string()],
                default_mode: None,
                default_model: None,
            },
            AgentRouteCandidate {
                profile: "executor".to_string(),
                description: None,
                source: None,
                executable: "executor".to_string(),
                capabilities: vec!["read".to_string(), "execute".to_string()],
                default_mode: None,
                default_model: None,
            },
        ];

        bind_goal_agent_selection(&mut request, &candidates, true).expect("selection");

        assert_eq!(request.workers()[0].profile.as_deref(), Some("reader"));
        assert_eq!(request.workers()[1].profile.as_deref(), Some("executor"));
    }

    #[test]
    fn goal_agent_selection_rejects_explicit_profile_without_required_capabilities() {
        let mut request = request_with_workers(vec![WorkerSpec {
            id: "writer".to_string(),
            role: Some("Implement the patch".to_string()),
            prompt: None,
            profile: Some("reader".to_string()),
            transport: None,
            depends_on: Vec::new(),
            mode: None,
            model: None,
            required_capabilities: Vec::new(),
            preferred_profiles: Vec::new(),
        }]);
        let candidates = vec![AgentRouteCandidate {
            profile: "reader".to_string(),
            description: None,
            source: None,
            executable: "reader".to_string(),
            capabilities: vec!["read".to_string()],
            default_mode: None,
            default_model: None,
        }];

        let error =
            bind_goal_agent_selection(&mut request, &candidates, true).expect_err("mismatch");

        assert!(
            error
                .message
                .contains("does not provide required capabilities")
        );
        assert!(error.message.contains("write"));
        assert!(error.message.contains("execute"));
    }

    #[test]
    fn goal_agent_selection_honors_preferred_profiles_after_capability_filtering() {
        let mut preferred_worker = worker("verifier", None);
        preferred_worker.role = Some("Verify the change".to_string());
        preferred_worker.preferred_profiles = vec!["executor-b".to_string()];
        let mut request = request_with_workers(vec![preferred_worker]);
        let candidates = vec![
            AgentRouteCandidate {
                profile: "executor-a".to_string(),
                description: None,
                source: None,
                executable: "executor-a".to_string(),
                capabilities: vec!["execute".to_string()],
                default_mode: None,
                default_model: None,
            },
            AgentRouteCandidate {
                profile: "executor-b".to_string(),
                description: None,
                source: None,
                executable: "executor-b".to_string(),
                capabilities: vec!["execute".to_string()],
                default_mode: None,
                default_model: None,
            },
        ];

        let decisions =
            bind_goal_agent_selection(&mut request, &candidates, true).expect("selection");

        assert_eq!(request.workers()[0].profile.as_deref(), Some("executor-b"));
        assert_eq!(
            decisions[0].preferred_profiles,
            vec!["executor-b".to_string()]
        );
        assert!(decisions[0].reason.contains("preferred eligible"));
    }

    #[test]
    fn spawn_attrs_carries_route_constraints_into_air() {
        let required = vec!["execute".to_string(), "write".to_string()];
        let preferred = vec!["codex".to_string()];

        let attrs = spawn_attrs(Some("codex"), None, None, None, &required, &preferred);

        assert!(attrs.contains("profile = \"codex\""));
        assert!(attrs.contains("required_capabilities = [\"execute\", \"write\"]"));
        assert!(attrs.contains("preferred_profiles = [\"codex\"]"));
    }
}
