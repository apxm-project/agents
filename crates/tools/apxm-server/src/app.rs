use axum::http::{HeaderValue, Method, header};
use axum::{Router, routing::get, routing::post};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::a2a::{a2a_get_task, a2a_jsonrpc, a2a_send_task};
use crate::agent::{
    agent_card, deregister_agent, get_agent, list_agents, receive_message, register_agent,
};
use crate::capability::{list_capabilities, register_capability};
use crate::checkpoints::{create_checkpoint, get_checkpoint, resume_checkpoint};
use crate::execute::{compile_graph, compile_graph_stream, execute, execute_stream};
use crate::executions::{get_execution, get_execution_node, list_executions};
use crate::generate::{handle_generate, handle_generate_stream, handle_schema};
use crate::health::{health, list_backends, list_models};
use crate::mcp::mcp_jsonrpc;
use crate::memory::{delete_fact, search_facts, store_fact};
use crate::routes::ServerRoute;
use crate::runs::{
    get_run, get_run_blob, get_run_events_bulk, get_run_graph, get_run_node, list_runs,
    stream_run_events,
};
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
        .route(ServerRoute::Backends.path(), get(list_backends))
        // Execution
        .route(ServerRoute::Execute.path(), post(execute))
        .route(ServerRoute::ExecuteStream.path(), post(execute_stream))
        // Caller-supplied PlanGraph: lower → execute, bypassing LLM emission
        // but applying the same raw-execute admission gate.
        .route(ServerRoute::Compile.path(), post(compile_graph))
        .route(ServerRoute::CompileStream.path(), post(compile_graph_stream))
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
        // Task queue (CLAIM op backend)
        .route(ServerRoute::Tasks.path(), post(create_task))
        .route(ServerRoute::TaskQueue.path(), get(list_tasks))
        .route(ServerRoute::TaskClaim.path(), post(claim_task))
        .route(ServerRoute::TaskComplete.path(), post(complete_task))
        // Checkpoints (PAUSE/RESUME HITL)
        .route(ServerRoute::Checkpoints.path(), post(create_checkpoint))
        .route(ServerRoute::CheckpointDetail.path(), get(get_checkpoint))
        .route(
            ServerRoute::CheckpointResume.path(),
            post(resume_checkpoint),
        )
        // A2A v0.3 - AgentCard discovery + REST task lifecycle
        .route(ServerRoute::AgentCard.path(), get(agent_card))
        .route(ServerRoute::A2a.path(), post(a2a_jsonrpc))
        // Peer-convention alias: apxm-os / os-client post tasks/send to /a2a/v1.
        .route(ServerRoute::A2aV1.path(), post(a2a_jsonrpc))
        .route(ServerRoute::A2aTasksSend.path(), post(a2a_send_task))
        .route(ServerRoute::A2aTaskDetail.path(), get(a2a_get_task))
        // LLM generation routes
        .route(ServerRoute::Generate.path(), post(handle_generate))
        .route(
            ServerRoute::GenerateStream.path(),
            post(handle_generate_stream),
        )
        .route(ServerRoute::Schema.path(), get(handle_schema))
        // MCP 2025-11-25 - JSON-RPC tools endpoint
        .route(ServerRoute::Mcp.path(), post(mcp_jsonrpc))
        // Phase 14.8.B - observer endpoints
        .route(ServerRoute::Runs.path(), get(list_runs))
        .route(ServerRoute::RunDetail.path(), get(get_run))
        .route(ServerRoute::RunGraph.path(), get(get_run_graph))
        .route(ServerRoute::RunNodeDetail.path(), get(get_run_node))
        .route(ServerRoute::RunEvents.path(), get(get_run_events_bulk))
        .route(ServerRoute::RunEventsStream.path(), get(stream_run_events))
        // Phase 14.8.E - rollout blob fetch
        .route(ServerRoute::RunBlob.path(), get(get_run_blob))
        // F04: opt-in, fail-closed bearer auth on mutating routes. The layer
        // is always installed but is a transparent pass-through unless
        // `server_config.auth.require_auth` is enabled (default off), so tests
        // and local dev are unaffected.
        .layer(axum::middleware::from_fn_with_state(
            state.server_config.auth.clone(),
            crate::auth::require_bearer,
        ))
        .with_state(state)
        // F04: restrictive CORS — allow loopback origins only (keeps the
        // studio proxy working on localhost) instead of the previous
        // `CorsLayer::permissive()` wildcard.
        .layer(loopback_cors_layer())
        .layer(TraceLayer::new_for_http())
        // Propagate X-Request-Id from clients; generate one when absent
        .layer(PropagateRequestIdLayer::new(req_id_header.clone()))
        .layer(SetRequestIdLayer::new(req_id_header, MakeRequestUuid))
}

/// CORS layer that permits only loopback (`127.0.0.1`, `[::1]`, `localhost`)
/// origins on any port. This keeps the apxm-studio proxy and local tooling
/// working over loopback while removing the wildcard `Access-Control-Allow-Origin`
/// that `CorsLayer::permissive()` emitted.
fn loopback_cors_layer() -> CorsLayer {
    let predicate = |origin: &HeaderValue, _request_parts: &axum::http::request::Parts| {
        origin
            .to_str()
            .map(is_loopback_origin)
            .unwrap_or(false)
    };
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(predicate))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
}

/// Returns true when an `Origin` header value points at loopback on any port.
fn is_loopback_origin(origin: &str) -> bool {
    // Strip the scheme; accept http/https only.
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    let Some(host_port) = rest else {
        return false;
    };
    // Drop any path component defensively (Origin should not carry one).
    let authority = host_port.split('/').next().unwrap_or(host_port);
    // Split host and optional port. IPv6 literals are bracketed.
    let host = if let Some(after_bracket) = authority.strip_prefix('[') {
        match after_bracket.split_once(']') {
            Some((inner, _port)) => inner,
            None => return false,
        }
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    matches!(host, "127.0.0.1" | "::1" | "localhost")
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

#[cfg(test)]
mod tests {
    use super::is_loopback_origin;

    #[test]
    fn loopback_origins_are_allowed() {
        assert!(is_loopback_origin("http://127.0.0.1:5173"));
        assert!(is_loopback_origin("http://localhost:3000"));
        assert!(is_loopback_origin("https://localhost"));
        assert!(is_loopback_origin("http://[::1]:18800"));
        assert!(is_loopback_origin("http://127.0.0.1"));
    }

    #[test]
    fn non_loopback_origins_are_rejected() {
        assert!(!is_loopback_origin("http://example.com"));
        assert!(!is_loopback_origin("https://evil.example.com:443"));
        assert!(!is_loopback_origin("http://127.0.0.1.evil.com"));
        assert!(!is_loopback_origin("http://10.0.0.5:18800"));
        assert!(!is_loopback_origin("ftp://localhost"));
        assert!(!is_loopback_origin("localhost:3000"));
    }
}
