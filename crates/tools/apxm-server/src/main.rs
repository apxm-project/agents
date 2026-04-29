//! APXM Server — HTTP gateway for the APXM agent runtime.
//!
//! Exposes the runtime's capabilities over a REST+SSE API with support for:
//! - **Graph execution**: `POST /v1/execute`, `POST /v1/execute/stream`
//! - **Skill library**: `GET /v1/skills`, `POST /v1/skills/{id}/execute`
//! - **LLM generation**: `POST /v1/generate`, `POST /v1/generate-stream`,
//!   `GET /v1/schema`
//! - **Memory**: `GET|POST /v1/memory`
//! - **Model discovery**: `GET /v1/models`
//! - **Agent registry**: `POST /v1/capabilities/register`
//! - **Task queue (CLAIM op)**: `POST /v1/tasks`, `GET /v1/tasks/{queue}`,
//!   `POST /v1/tasks/{queue}/claim`, `POST /v1/tasks/{id}/complete`
//! - **HITL checkpoints (PAUSE/RESUME)**: `POST /v1/checkpoints`,
//!   `GET /v1/checkpoints/{id}`, `POST /v1/checkpoints/{id}/resume`
//! - **MCP 2025-11-05** (JSON-RPC 2.0): `POST /v1/mcp`
//! - **A2A v0.3** (REST): `POST /a2a/tasks/send`, `GET /a2a/tasks/{id}`,
//!   `GET /.well-known/agent.json`
//!
//! # Running
//! ```bash
//! APXM_BACKEND=<backend> <PROVIDER_API_KEY>=<API_KEY> apxm-server --port 18800
//! ```
//!
//! # Architecture
//! Each request handler is a thin Axum layer over [`Runtime`]; no business
//! logic lives in this file — heavy lifting is in `apxm-runtime` and
//! `apxm-compiler`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use apxm_backends::BackendRegistration;
use apxm_runtime::{Runtime, RuntimeConfig};
use axum::{Router, routing::get, routing::post};
use dashmap::DashMap;
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

mod a2a;
mod agent;
mod capability;
mod checkpoints;
mod error;
mod execute;
mod executions;
mod generate;
mod health;
mod helpers;
mod mcp;
mod memory;
mod skills;
mod state;
mod tasks;
mod types;

#[cfg(test)]
mod tests;

use crate::a2a::{a2a_get_task, a2a_jsonrpc, a2a_send_task};
use crate::agent::{
    agent_card, deregister_agent, get_agent, list_agents, receive_message, register_agent,
};
use crate::capability::{list_capabilities, register_capability};
use crate::checkpoints::{CheckpointStore, create_checkpoint, get_checkpoint, resume_checkpoint};
use crate::execute::{execute, execute_stream};
use crate::executions::{ExecutionStore, get_execution, get_execution_node};
use crate::generate::{handle_generate, handle_generate_stream, handle_schema};
use crate::health::{health, list_models};
use crate::mcp::mcp_jsonrpc;
use crate::memory::{delete_fact, search_facts, store_fact};
use crate::skills::{
    SkillLibrary, execute_skill, execute_skill_stream, get_skill, list_skills, parse_skill_roots,
    register_skill_event_payloads, validate_skill,
};
use crate::state::{AppState, EXECUTE_COMPLETE, ExecuteCompletePayload};
use crate::tasks::{TaskQueueManager, claim_task, complete_task, create_task, list_tasks};

pub(crate) const DEFAULT_ADDR: &str = "127.0.0.1:18800";
pub(crate) const DEFAULT_PUBLIC_URL: &str = "http://localhost:18800";

/// Build the Axum router for the APXM server.
///
/// Extracted from `main()` so that integration tests can call it directly
/// without binding to a TCP port.
fn build_app(state: AppState) -> Router {
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
        // Task queue (Plan 07 — CLAIM op backend)
        .route("/v1/tasks", post(create_task))
        .route("/v1/tasks/{queue}", get(list_tasks))
        .route("/v1/tasks/{queue}/claim", post(claim_task))
        .route("/v1/tasks/{id}/complete", post(complete_task))
        // Checkpoints (Plan 07 — PAUSE/RESUME HITL)
        .route("/v1/checkpoints", post(create_checkpoint))
        .route("/v1/checkpoints/{id}", get(get_checkpoint))
        .route("/v1/checkpoints/{id}/resume", post(resume_checkpoint))
        // A2A v0.3 — AgentCard discovery + REST task lifecycle
        .route("/.well-known/agent.json", get(agent_card))
        .route("/a2a", post(a2a_jsonrpc)) // JSON-RPC task endpoint
        .route("/a2a/tasks/send", post(a2a_send_task)) // REST: submit task + execute
        .route("/a2a/tasks/{id}", get(a2a_get_task)) // REST: poll task result
        // LLM generation (Phase A3 — LLM backend routes)
        .route("/v1/generate", post(handle_generate))
        .route("/v1/generate-stream", post(handle_generate_stream))
        .route("/v1/schema", get(handle_schema))
        // MCP 2025-11-05 — JSON-RPC tools endpoint
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,apxm_server=debug".to_string()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let skill_roots = parse_skill_roots(&args);
    let skill_library = SkillLibrary::new(skill_roots);

    let runtime = Arc::new(Runtime::new(RuntimeConfig::default()).await?);

    // ── Auto-load LLM backends from backend store ──
    {
        let mut loaded = 0u32;
        let mut first_name: Option<String> = None;
        match apxm_credentials::BackendStore::open() {
            Ok(store) => match store.list() {
                Ok(backends) if backends.is_empty() => {
                    warn!("backend store is empty — no LLM backends registered");
                }
                Ok(backends) => {
                    for backend in backends {
                        match BackendRegistration::from_backend_config(&backend) {
                            Ok(registration) => {
                                if let Err(e) = registration.register(runtime.llm_registry()).await
                                {
                                    warn!(name = %backend.name, error = %e, "failed to register LLM backend");
                                    continue;
                                }
                                if first_name.is_none() {
                                    first_name = Some(backend.name.clone());
                                }
                                loaded += 1;
                            }
                            Err(e) => {
                                warn!(name = %backend.name, error = %e, "failed to build backend registration");
                            }
                        }
                    }
                    if let Some(ref default) = first_name {
                        if let Err(e) = runtime.llm_registry().set_default(default) {
                            warn!(backend = %default, error = %e, "failed to set default backend");
                        }
                    }
                    info!(count = loaded, "loaded LLM backends from backend store");
                }
                Err(e) => {
                    warn!(error = %e, "failed to read backend store");
                }
            },
            Err(e) => {
                warn!(error = %e, "credential store unavailable — no LLM backends registered");
            }
        }
    }

    let state = AppState {
        runtime,
        agent_registry: Arc::new(DashMap::new()),
        task_manager: TaskQueueManager::new(),
        checkpoint_store: CheckpointStore::new(),
        start_time: SystemTime::now(),
        a2a_tasks: Arc::new(DashMap::new()),
        skill_library,
        execution_store: ExecutionStore::new(),
    };

    let app = build_app(state);

    // Parse --port from CLI args (service-manager passes `--port <N>`)
    let cli_port: Option<u16> = {
        args.iter()
            .position(|a| a == "--port")
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
    };

    let addr = if let Some(port) = cli_port {
        SocketAddr::from(([127, 0, 0, 1], port))
    } else {
        std::env::var("APXM_SERVER_ADDR")
            .ok()
            .and_then(|s| s.parse::<SocketAddr>().ok())
            .unwrap_or_else(|| DEFAULT_ADDR.parse().expect("valid default addr"))
    };
    info!(%addr, "starting apxm-server");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Graceful shutdown on Ctrl+C / SIGTERM
    let shutdown = async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = sigterm.recv() => {},
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await.expect("Ctrl+C handler");
        }
        info!("shutdown signal received");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}
