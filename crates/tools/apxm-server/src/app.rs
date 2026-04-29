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
        .route("/health", get(health))
        .route("/v1/models", get(list_models))
        // Execution
        .route("/v1/execute", post(execute))
        .route("/v1/execute/stream", post(execute_stream))
        // Memory
        .route("/v1/memory/facts/store", post(store_fact))
        .route("/v1/memory/facts/search", post(search_facts))
        .route("/v1/memory/facts/delete", post(delete_fact))
        // Capabilities
        .route("/v1/capabilities", get(list_capabilities))
        .route("/v1/capabilities/register", post(register_capability))
        // Server-owned skill inventory and static skill execution
        .route("/v1/skills", get(list_skills))
        .route("/v1/skills/{id}", get(get_skill))
        .route("/v1/skills/{id}/validate", post(validate_skill))
        .route("/v1/skills/{id}/execute", post(execute_skill))
        .route("/v1/skills/{id}/execute/stream", post(execute_skill_stream))
        .route("/v1/executions", get(list_executions))
        .route("/v1/executions/{execution_id}", get(get_execution))
        .route(
            "/v1/executions/{execution_id}/nodes/{node_id}",
            get(get_execution_node),
        )
        // COMMUNICATE receive target
        .route("/v1/receive", post(receive_message))
        // Agent registry
        .route("/v1/agents", get(list_agents))
        .route("/v1/agents/register", post(register_agent))
        .route("/v1/agents/{name}", get(get_agent).delete(deregister_agent))
        // Task queue (Plan 07 - CLAIM op backend)
        .route("/v1/tasks", post(create_task))
        .route("/v1/tasks/{queue}", get(list_tasks))
        .route("/v1/tasks/{queue}/claim", post(claim_task))
        .route("/v1/tasks/{id}/complete", post(complete_task))
        // Checkpoints (Plan 07 - PAUSE/RESUME HITL)
        .route("/v1/checkpoints", post(create_checkpoint))
        .route("/v1/checkpoints/{id}", get(get_checkpoint))
        .route("/v1/checkpoints/{id}/resume", post(resume_checkpoint))
        // A2A v0.3 - AgentCard discovery + REST task lifecycle
        .route("/.well-known/agent.json", get(agent_card))
        .route("/a2a", post(a2a_jsonrpc))
        .route("/a2a/tasks/send", post(a2a_send_task))
        .route("/a2a/tasks/{id}", get(a2a_get_task))
        // LLM generation (Phase A3 - LLM backend routes)
        .route("/v1/generate", post(handle_generate))
        .route("/v1/generate-stream", post(handle_generate_stream))
        .route("/v1/schema", get(handle_schema))
        // MCP 2025-11-05 - JSON-RPC tools endpoint
        .route("/v1/mcp", post(mcp_jsonrpc))
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
