use std::fmt::Display;

pub(crate) const HEALTH: &str = "/health";
pub(crate) const MODELS: &str = "/v1/models";
pub(crate) const BACKENDS: &str = "/v1/backends";
pub(crate) const EXECUTE: &str = "/v1/execute";
pub(crate) const EXECUTE_STREAM: &str = "/v1/execute/stream";
pub(crate) const COMPILE: &str = "/v1/compile";
pub(crate) const COMPILE_STREAM: &str = "/v1/compile/stream";
pub(crate) const MEMORY_FACTS_STORE: &str = "/v1/memory/facts/store";
pub(crate) const MEMORY_FACTS_SEARCH: &str = "/v1/memory/facts/search";
pub(crate) const MEMORY_FACTS_DELETE: &str = "/v1/memory/facts/delete";
pub(crate) const CAPABILITIES: &str = "/v1/capabilities";
pub(crate) const CAPABILITIES_REGISTER: &str = "/v1/capabilities/register";
pub(crate) const SKILLS: &str = "/v1/skills";
pub(crate) const SKILL_DETAIL: &str = "/v1/skills/{id}";
pub(crate) const SKILL_VALIDATE: &str = "/v1/skills/{id}/validate";
pub(crate) const SKILL_EXECUTE: &str = "/v1/skills/{id}/execute";
pub(crate) const SKILL_EXECUTE_STREAM: &str = "/v1/skills/{id}/execute/stream";
pub(crate) const EXECUTIONS: &str = "/v1/executions";
pub(crate) const EXECUTION_DETAIL: &str = "/v1/executions/{execution_id}";
pub(crate) const EXECUTION_NODE_DETAIL: &str = "/v1/executions/{execution_id}/nodes/{node_id}";
pub(crate) const RECEIVE: &str = "/v1/receive";
pub(crate) const AGENTS: &str = "/v1/agents";
pub(crate) const AGENTS_REGISTER: &str = "/v1/agents/register";
pub(crate) const AGENT_DETAIL: &str = "/v1/agents/{name}";
pub(crate) const TASKS: &str = "/v1/tasks";
pub(crate) const TASK_QUEUE: &str = "/v1/tasks/{queue}";
pub(crate) const TASK_CLAIM: &str = "/v1/tasks/{queue}/claim";
pub(crate) const TASK_COMPLETE: &str = "/v1/tasks/{id}/complete";
pub(crate) const CHECKPOINTS: &str = "/v1/checkpoints";
pub(crate) const CHECKPOINT_DETAIL: &str = "/v1/checkpoints/{id}";
pub(crate) const CHECKPOINT_RESUME: &str = "/v1/checkpoints/{id}/resume";
pub(crate) const AGENT_CARD: &str = "/.well-known/agent.json";
pub(crate) const A2A: &str = "/a2a";
pub(crate) const A2A_TASKS_SEND: &str = "/a2a/tasks/send";
pub(crate) const A2A_TASK_DETAIL: &str = "/a2a/tasks/{id}";
pub(crate) const GENERATE: &str = "/v1/generate";
pub(crate) const GENERATE_STREAM: &str = "/v1/generate-stream";
pub(crate) const SCHEMA: &str = "/v1/schema";
pub(crate) const MCP: &str = apxm_core::constants::mcp::ROUTE;
// Phase 14.8.B — observer endpoints.
pub(crate) const RUNS: &str = "/v1/runs";
pub(crate) const RUN_DETAIL: &str = "/v1/runs/{execution_id}";
pub(crate) const RUN_GRAPH: &str = "/v1/runs/{execution_id}/graph";
pub(crate) const RUN_NODE_DETAIL: &str = "/v1/runs/{execution_id}/nodes/{node_id}";
pub(crate) const RUN_EVENTS: &str = "/v1/runs/{execution_id}/events";
pub(crate) const RUN_EVENTS_STREAM: &str = "/v1/runs/{execution_id}/events/stream";
// Phase 14.8.E — rollout blob endpoint.
pub(crate) const RUN_BLOB: &str = "/v1/runs/{execution_id}/blobs/{blob_ref}";
pub(crate) const RUN_CANCEL: &str = "/v1/runs/{execution_id}/cancel";
pub(crate) const GOALS: &str = "/v1/goals";
pub(crate) const GOAL_DETAIL: &str = "/v1/goals/{goal_id}";
pub(crate) const GOAL_EVENTS: &str = "/v1/goals/{goal_id}/events";
pub(crate) const GOAL_CANCEL: &str = "/v1/goals/{goal_id}/cancel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServerRoute {
    Health,
    Models,
    Backends,
    Execute,
    ExecuteStream,
    Compile,
    CompileStream,
    MemoryFactsStore,
    MemoryFactsSearch,
    MemoryFactsDelete,
    Capabilities,
    CapabilitiesRegister,
    Skills,
    SkillDetail,
    SkillValidate,
    SkillExecute,
    SkillExecuteStream,
    Executions,
    ExecutionDetail,
    ExecutionNodeDetail,
    Receive,
    Agents,
    AgentsRegister,
    AgentDetail,
    Tasks,
    TaskQueue,
    TaskClaim,
    TaskComplete,
    Checkpoints,
    CheckpointDetail,
    CheckpointResume,
    AgentCard,
    A2a,
    A2aTasksSend,
    A2aTaskDetail,
    Generate,
    GenerateStream,
    Schema,
    Mcp,
    Runs,
    RunDetail,
    RunGraph,
    RunNodeDetail,
    RunEvents,
    RunEventsStream,
    RunBlob,
    RunCancel,
    Goals,
    GoalDetail,
    GoalEvents,
    GoalCancel,
}

impl ServerRoute {
    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::Health => HEALTH,
            Self::Models => MODELS,
            Self::Backends => BACKENDS,
            Self::Execute => EXECUTE,
            Self::ExecuteStream => EXECUTE_STREAM,
            Self::Compile => COMPILE,
            Self::CompileStream => COMPILE_STREAM,
            Self::MemoryFactsStore => MEMORY_FACTS_STORE,
            Self::MemoryFactsSearch => MEMORY_FACTS_SEARCH,
            Self::MemoryFactsDelete => MEMORY_FACTS_DELETE,
            Self::Capabilities => CAPABILITIES,
            Self::CapabilitiesRegister => CAPABILITIES_REGISTER,
            Self::Skills => SKILLS,
            Self::SkillDetail => SKILL_DETAIL,
            Self::SkillValidate => SKILL_VALIDATE,
            Self::SkillExecute => SKILL_EXECUTE,
            Self::SkillExecuteStream => SKILL_EXECUTE_STREAM,
            Self::Executions => EXECUTIONS,
            Self::ExecutionDetail => EXECUTION_DETAIL,
            Self::ExecutionNodeDetail => EXECUTION_NODE_DETAIL,
            Self::Receive => RECEIVE,
            Self::Agents => AGENTS,
            Self::AgentsRegister => AGENTS_REGISTER,
            Self::AgentDetail => AGENT_DETAIL,
            Self::Tasks => TASKS,
            Self::TaskQueue => TASK_QUEUE,
            Self::TaskClaim => TASK_CLAIM,
            Self::TaskComplete => TASK_COMPLETE,
            Self::Checkpoints => CHECKPOINTS,
            Self::CheckpointDetail => CHECKPOINT_DETAIL,
            Self::CheckpointResume => CHECKPOINT_RESUME,
            Self::AgentCard => AGENT_CARD,
            Self::A2a => A2A,
            Self::A2aTasksSend => A2A_TASKS_SEND,
            Self::A2aTaskDetail => A2A_TASK_DETAIL,
            Self::Generate => GENERATE,
            Self::GenerateStream => GENERATE_STREAM,
            Self::Schema => SCHEMA,
            Self::Mcp => MCP,
            Self::Runs => RUNS,
            Self::RunDetail => RUN_DETAIL,
            Self::RunGraph => RUN_GRAPH,
            Self::RunNodeDetail => RUN_NODE_DETAIL,
            Self::RunEvents => RUN_EVENTS,
            Self::RunEventsStream => RUN_EVENTS_STREAM,
            Self::RunBlob => RUN_BLOB,
            Self::RunCancel => RUN_CANCEL,
            Self::Goals => GOALS,
            Self::GoalDetail => GOAL_DETAIL,
            Self::GoalEvents => GOAL_EVENTS,
            Self::GoalCancel => GOAL_CANCEL,
        }
    }
}

// Path constructors for the public REST surface. The release binary builds
// URLs from string literals at request-handler registration time, so these
// helpers look unused to `dead_code`; they are exercised by the in-crate
// integration tests (see `tests/`) which use them as the single source of
// truth for route shapes.
#[allow(dead_code)]
pub(crate) fn skill_detail_path(id: impl Display) -> String {
    format!("{SKILLS}/{id}")
}

#[allow(dead_code)]
pub(crate) fn skill_validate_path(id: impl Display) -> String {
    format!("{SKILLS}/{id}/validate")
}

#[allow(dead_code)]
pub(crate) fn skill_execute_path(id: impl Display) -> String {
    format!("{SKILLS}/{id}/execute")
}

#[allow(dead_code)]
pub(crate) fn skill_execute_stream_path(id: impl Display) -> String {
    format!("{SKILLS}/{id}/execute/stream")
}

#[allow(dead_code)]
pub(crate) fn execution_detail_path(id: impl Display) -> String {
    format!("{EXECUTIONS}/{id}")
}

#[allow(dead_code)]
pub(crate) fn execution_node_detail_path(
    execution_id: impl Display,
    node_id: impl Display,
) -> String {
    format!("{EXECUTIONS}/{execution_id}/nodes/{node_id}")
}

#[allow(dead_code)]
pub(crate) fn task_queue_path(queue: impl Display) -> String {
    format!("{TASKS}/{queue}")
}

#[allow(dead_code)]
pub(crate) fn task_claim_path(queue: impl Display) -> String {
    format!("{TASKS}/{queue}/claim")
}

#[allow(dead_code)]
pub(crate) fn task_complete_path(id: impl Display) -> String {
    format!("{TASKS}/{id}/complete")
}

pub(crate) fn checkpoint_detail_path(id: impl Display) -> String {
    format!("{CHECKPOINTS}/{id}")
}

pub(crate) fn checkpoint_resume_path(id: impl Display) -> String {
    format!("{CHECKPOINTS}/{id}/resume")
}

#[allow(dead_code)]
pub(crate) fn run_detail_path(execution_id: impl Display) -> String {
    format!("{RUNS}/{execution_id}")
}

#[allow(dead_code)]
pub(crate) fn run_graph_path(execution_id: impl Display) -> String {
    format!("{RUNS}/{execution_id}/graph")
}

#[allow(dead_code)]
pub(crate) fn run_cancel_path(execution_id: impl Display) -> String {
    format!("{RUNS}/{execution_id}/cancel")
}

#[allow(dead_code)]
pub(crate) fn goal_detail_path(goal_id: impl Display) -> String {
    format!("{GOALS}/{goal_id}")
}

#[allow(dead_code)]
pub(crate) fn goal_events_path(goal_id: impl Display) -> String {
    format!("{GOALS}/{goal_id}/events")
}

#[allow(dead_code)]
pub(crate) fn goal_cancel_path(goal_id: impl Display) -> String {
    format!("{GOALS}/{goal_id}/cancel")
}

#[allow(dead_code)]
pub(crate) fn run_node_detail_path(execution_id: impl Display, node_id: impl Display) -> String {
    format!("{RUNS}/{execution_id}/nodes/{node_id}")
}

#[allow(dead_code)]
pub(crate) fn run_events_path(execution_id: impl Display) -> String {
    format!("{RUNS}/{execution_id}/events")
}

#[allow(dead_code)]
pub(crate) fn run_events_stream_path(execution_id: impl Display) -> String {
    format!("{RUNS}/{execution_id}/events/stream")
}

#[allow(dead_code)]
pub(crate) fn run_blob_path(execution_id: impl Display, blob_ref: impl Display) -> String {
    format!("{RUNS}/{execution_id}/blobs/{blob_ref}")
}
