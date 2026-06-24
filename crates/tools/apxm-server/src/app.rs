use axum::http::{HeaderValue, Method, header};
use axum::{Router, routing::get, routing::post};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::a2a::{a2a_get_task, a2a_jsonrpc, a2a_send_task};
use crate::agent::{
    agent_card, deregister_agent, get_agent, list_agents, receive_message, register_agent,
};
use crate::capability::{
    invoke_capability, list_capability_templates, reindex_capability_templates,
};
use crate::checkpoints::{create_checkpoint, get_checkpoint, resume_checkpoint};
use crate::conversations::post_conversation_message;
use crate::delegated_capabilities::{delegate_capability, revoke_capability};
use crate::execute::{
    compile_artifact, compile_workflow, compile_workflow_stream, execute, execute_stream,
};
use crate::executions::{get_execution, get_execution_node, list_executions};
use crate::fleet::get_fleet;
use crate::generate::{handle_generate, handle_generate_stream, handle_schema};
use crate::goals::{cancel_goal, get_goal, get_goal_events_bulk, list_goals, stream_goal_events};
use crate::health::{health, list_backends, list_models};
use crate::mcp::{mcp_jsonrpc, post_goal};
use crate::memory::{delete_fact, search_facts, store_fact};
use crate::metrics::scrape_metrics;
use crate::permissions::respond_permission;
use crate::rerun::{rerun_from_node, rerun_run};
use crate::routes::ServerRoute;
use crate::run_history::{get_run_summary, list_workflow_runs, reindex_runs};
use crate::runs::{
    cancel_run, clear_runs, get_run, get_run_artifact, get_run_blob, get_run_events_bulk,
    get_run_graph, get_run_node, get_session_history, list_run_artifacts, list_runs,
    stream_run_events,
};
use crate::sessions::{
    cancel_session, compact_session, get_session_status, list_session_events, stream_session_events,
};
use crate::skills::{
    execute_skill, execute_skill_stream, get_skill, list_skills, register_skill_event_payloads,
    validate_skill,
};
use crate::state::AppState;
use crate::tasks::{claim_task, complete_task, create_task, list_tasks};

/// Build the Axum router for the APXM server.
///
/// Extracted from `main()` so that integration tests can call it directly
/// without binding to a TCP port.
pub(crate) fn build_app(state: AppState) -> Router {
    register_server_event_payloads();
    // Warm the process-wide permission registry before handlers mount.
    let _ = crate::permissions::PermissionRegistry::global();
    let req_id_header = axum::http::HeaderName::from_static("x-request-id");
    let auth_policy = crate::auth::AuthPolicy {
        config: state.server_config.auth.clone(),
        effective_require_auth: state.effective_require_auth,
    };
    let safety_state = state.safety_state.clone();
    let shutdown = state.shutdown.clone();
    let body_limit = crate::safety::body_limit_layer(&state.server_config.safety);

    let mut router = Router::new()
        // Health + meta
        .route(ServerRoute::Health.path(), get(health))
        .route(ServerRoute::Metrics.path(), get(scrape_metrics))
        .route(ServerRoute::Models.path(), get(list_models))
        .route(ServerRoute::Backends.path(), get(list_backends))
        // Execution
        .route(ServerRoute::Execute.path(), post(execute))
        .route(ServerRoute::ExecuteStream.path(), post(execute_stream))
        // Caller-supplied workflow source: resolve AIR, then apply the same
        // raw-execute admission gate.
        .route(ServerRoute::CompileArtifact.path(), post(compile_artifact))
        .route(ServerRoute::Compile.path(), post(compile_workflow))
        .route(
            ServerRoute::CompileStream.path(),
            post(compile_workflow_stream),
        )
        // Memory
        .route(ServerRoute::MemoryFactsStore.path(), post(store_fact))
        .route(ServerRoute::MemoryFactsSearch.path(), post(search_facts))
        .route(ServerRoute::MemoryFactsDelete.path(), post(delete_fact))
        // Capability templates and delegated capability invocation.
        .route(
            ServerRoute::CapabilityTemplates.path(),
            get(list_capability_templates),
        )
        .route(
            ServerRoute::CapabilityTemplatesReindex.path(),
            post(reindex_capability_templates),
        )
        .route(
            ServerRoute::CapabilityDelegate.path(),
            post(delegate_capability),
        )
        .route(
            ServerRoute::CapabilityRevoke.path(),
            post(revoke_capability),
        )
        .route(
            ServerRoute::CapabilityInvoke.path(),
            post(invoke_capability),
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
        // Turn-input seam: deliver one user message to a session's parked recv
        // node (dumb-pipe contract; reply streams over the session's open SSE).
        .route(
            ServerRoute::ConversationMessage.path(),
            post(post_conversation_message),
        )
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
        // observer endpoints
        .route(ServerRoute::Runs.path(), get(list_runs))
        .route(ServerRoute::RunsClear.path(), post(clear_runs))
        .route(ServerRoute::RunDetail.path(), get(get_run))
        .route(ServerRoute::RunGraph.path(), get(get_run_graph))
        .route(ServerRoute::RunNodeDetail.path(), get(get_run_node))
        .route(ServerRoute::RunArtifacts.path(), get(list_run_artifacts))
        .route(ServerRoute::RunArtifactDetail.path(), get(get_run_artifact))
        .route(ServerRoute::RunEvents.path(), get(get_run_events_bulk))
        .route(ServerRoute::RunEventsStream.path(), get(stream_run_events))
        // rollout blob fetch
        .route(ServerRoute::RunBlob.path(), get(get_run_blob))
        // Mid-flight cancellation — trips the run's abort signal.
        .route(ServerRoute::RunCancel.path(), post(cancel_run))
        // Re-execute a prior run (optionally from a specific node).
        .route(ServerRoute::RunRerun.path(), post(rerun_run))
        .route(ServerRoute::RunRerunFromNode.path(), post(rerun_from_node))
        // Fleet observability rollup for the studio Fleet/observability views.
        .route(ServerRoute::ObservabilityFleet.path(), get(get_fleet))
        // Durable, role-tagged chat transcript by session_id (shared across
        // the `apxm chat` CLI and studio Chat hop).
        .route(ServerRoute::SessionHistory.path(), get(get_session_history))
        // Session control API: status, cancel, compact, events.
        .route(ServerRoute::SessionStatus.path(), get(get_session_status))
        .route(ServerRoute::SessionCancel.path(), post(cancel_session))
        .route(ServerRoute::SessionCompact.path(), post(compact_session))
        .route(ServerRoute::SessionEvents.path(), get(list_session_events))
        .route(
            ServerRoute::SessionEventsStream.path(),
            get(stream_session_events),
        )
        // Server-driven permission response.
        .route(
            ServerRoute::PermissionRespond.path(),
            post(respond_permission),
        )
        // Goal aggregate observer endpoints for frontend/client state.
        .route(ServerRoute::Goals.path(), get(list_goals).post(post_goal))
        .route(ServerRoute::GoalDetail.path(), get(get_goal))
        .route(ServerRoute::GoalEvents.path(), get(get_goal_events_bulk))
        .route(
            ServerRoute::GoalEventsStream.path(),
            get(stream_goal_events),
        )
        .route(ServerRoute::GoalCancel.path(), post(cancel_goal))
        // Workflow-scoped run history.
        .route(ServerRoute::WorkflowRuns.path(), get(list_workflow_runs))
        .route(ServerRoute::RunsReindex.path(), post(reindex_runs))
        // Run summary alias (lighter shape than /v1/executions/{id}).
        .route("/v1/runs/{execution_id}/summary", get(get_run_summary));

    if let Some(layer) = body_limit {
        router = router.layer(layer);
    }

    router
        // Opt-in, fail-closed bearer auth on mutating routes. The layer is
        // always installed; non-loopback binds are auth-on by default.
        .layer(axum::middleware::from_fn_with_state(
            safety_state,
            crate::safety::rate_limit_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            shutdown,
            crate::shutdown::track_in_flight,
        ))
        .layer(axum::middleware::from_fn(
            crate::metrics::record_request_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            auth_policy,
            crate::auth::require_bearer,
        ))
        .with_state(state)
        // Restrictive CORS allows loopback origins only, keeping the studio
        // proxy working on localhost without a wildcard origin.
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
        origin.to_str().is_ok_and(is_loopback_origin)
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
    register_skill_event_payloads();
}
