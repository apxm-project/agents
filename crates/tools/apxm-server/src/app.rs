use axum::{Router, routing::get, routing::post};
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::a2a::{a2a_get_task, a2a_jsonrpc, a2a_send_task};
use crate::agent::{
    agent_card, deregister_agent, get_agent, list_agents, receive_message, register_agent,
};
use crate::capability::{list_capabilities, register_capability};
use crate::checkpoints::{create_checkpoint, get_checkpoint, resume_checkpoint};
use crate::execute::{execute, execute_stream};
use crate::executions::{get_execution, get_execution_node, list_executions};
use crate::generate::{handle_generate, handle_generate_stream, handle_schema};
use crate::health::{health, list_models};
use crate::mcp::mcp_jsonrpc;
use crate::memory::{delete_fact, search_facts, store_fact};
use crate::routes::ServerRoute;
use crate::skills::{
    execute_skill, execute_skill_stream, get_skill, list_skills, register_skill_event_payloads,
    validate_skill,
};
use crate::state::{AppState, EXECUTE_COMPLETE, ExecuteCompletePayload};
use crate::tasks::{claim_task, complete_task, create_task, list_tasks};

/// Build the Axum router for the APXM server.
///
/// Extracted from `main()` so that integration tests can call it directly
/// without binding to a TCP port.
pub(crate) fn build_app(state: AppState) -> Router {
    register_server_event_payloads();
    let req_id_header = axum::http::HeaderName::from_static("x-request-id");
    Router::new()
        // Health + meta
        .route(ServerRoute::Health.path(), get(health))
        .route(ServerRoute::Models.path(), get(list_models))
        // Execution
        .route(ServerRoute::Execute.path(), post(execute))
        .route(ServerRoute::ExecuteStream.path(), post(execute_stream))
        // Memory
        .route(ServerRoute::MemoryFactsStore.path(), post(store_fact))
        .route(ServerRoute::MemoryFactsSearch.path(), post(search_facts))
        .route(ServerRoute::MemoryFactsDelete.path(), post(delete_fact))
        // Capabilities
        .route(ServerRoute::Capabilities.path(), get(list_capabilities))
        .route(
            ServerRoute::CapabilitiesRegister.path(),
            post(register_capability),
        )
        // Server-owned skill inventory and static skill execution
        .route(ServerRoute::Skills.path(), get(list_skills))
        .route(ServerRoute::SkillDetail.path(), get(get_skill))
        .route(ServerRoute::SkillValidate.path(), post(validate_skill))
        .route(ServerRoute::SkillExecute.path(), post(execute_skill))
        .route(
            ServerRoute::SkillExecuteStream.path(),
            post(execute_skill_stream),
        )
        .route(ServerRoute::Executions.path(), get(list_executions))
        .route(ServerRoute::ExecutionDetail.path(), get(get_execution))
        .route(
            ServerRoute::ExecutionNodeDetail.path(),
            get(get_execution_node),
        )
        // COMMUNICATE receive target
        .route(ServerRoute::Receive.path(), post(receive_message))
        // Agent registry
        .route(ServerRoute::Agents.path(), get(list_agents))
        .route(ServerRoute::AgentsRegister.path(), post(register_agent))
        .route(
            ServerRoute::AgentDetail.path(),
            get(get_agent).delete(deregister_agent),
        )
        // Task queue (Plan 07 - CLAIM op backend)
        .route(ServerRoute::Tasks.path(), post(create_task))
        .route(ServerRoute::TaskQueue.path(), get(list_tasks))
        .route(ServerRoute::TaskClaim.path(), post(claim_task))
        .route(ServerRoute::TaskComplete.path(), post(complete_task))
        // Checkpoints (Plan 07 - PAUSE/RESUME HITL)
        .route(ServerRoute::Checkpoints.path(), post(create_checkpoint))
        .route(ServerRoute::CheckpointDetail.path(), get(get_checkpoint))
        .route(
            ServerRoute::CheckpointResume.path(),
            post(resume_checkpoint),
        )
        // A2A v0.3 - AgentCard discovery + REST task lifecycle
        .route(ServerRoute::AgentCard.path(), get(agent_card))
        .route(ServerRoute::A2a.path(), post(a2a_jsonrpc))
        .route(ServerRoute::A2aTasksSend.path(), post(a2a_send_task))
        .route(ServerRoute::A2aTaskDetail.path(), get(a2a_get_task))
        // LLM generation (Phase A3 - LLM backend routes)
        .route(ServerRoute::Generate.path(), post(handle_generate))
        .route(
            ServerRoute::GenerateStream.path(),
            post(handle_generate_stream),
        )
        .route(ServerRoute::Schema.path(), get(handle_schema))
        // MCP 2025-11-05 - JSON-RPC tools endpoint
        .route(ServerRoute::Mcp.path(), post(mcp_jsonrpc))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        // Propagate X-Request-Id from clients; generate one when absent
        .layer(PropagateRequestIdLayer::new(req_id_header.clone()))
        .layer(SetRequestIdLayer::new(req_id_header, MakeRequestUuid))
}

fn register_server_event_payloads() {
    match apxm_core::events::register_event_payload::<ExecuteCompletePayload>(EXECUTE_COMPLETE) {
        Ok(()) | Err(apxm_core::events::EventRegistryError::AlreadyRegistered { .. }) => {}
        Err(apxm_core::events::EventRegistryError::CoreKind { kind }) => {
            panic!("server event kind `{kind}` conflicts with core event kind");
        }
    }
    register_skill_event_payloads();
}
