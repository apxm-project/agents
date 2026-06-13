//! APXM Server — HTTP gateway for the APXM agent runtime.
//!
//! Exposes the runtime's capabilities over a REST+SSE API with support for:
//! - **Workflow execution**: `POST /v1/execute`, `POST /v1/execute/stream`
//! - **Skill library**: inventory, validation, REST/SSE execution, and
//!   execution-record lookup through `/v1/skills/*` and `/v1/executions/*`
//! - **Goals**: `POST /v1/goals`, `GET /v1/goals/{goal_id}`, retained
//!   events, cancellation, and task-plan state for clients
//! - **LLM generation**: `POST /v1/generate`, `POST /v1/generate-stream`,
//!   `GET /v1/schema`
//! - **Memory**: `GET|POST /v1/memory`
//! - **Model discovery**: `GET /v1/models`
//! - **Agent registry**: `POST /v1/capabilities/register`
//! - **Task queue (CLAIM op)**: `POST /v1/tasks`, `GET /v1/tasks/{queue}`,
//!   `POST /v1/tasks/{queue}/claim`, `POST /v1/tasks/{id}/complete`
//! - **HITL checkpoints (PAUSE/RESUME)**: `POST /v1/checkpoints`,
//!   `GET /v1/checkpoints/{id}`, `POST /v1/checkpoints/{id}/resume`
//! - **MCP 2025-11-25** (JSON-RPC 2.0): `POST /v1/mcp`
//! - **A2A v0.3** (REST): `POST /a2a/tasks/send`, `GET /a2a/tasks/{id}`,
//!   `GET /.well-known/agent.json`
//!
//! # Running
//! ```bash
//! APXM_BACKEND=<backend> <PROVIDER_API_KEY>=<API_KEY> apxm-server --port 18800
//! ```
//!
//! # Architecture
//! `main.rs` only wires modules and process startup. Request handlers live in
//! focused modules, with workflow execution delegated to `apxm-runtime` and
//! `apxm-compiler`, and static skill execution using `apxm-skill` manifests plus
//! `apxm-artifact` containers.

// SSE streaming creates cross-thread alloc/free patterns where mimalloc beats the system allocator.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use apxm_driver::ServerConfig;

mod a2a;
mod agent;
mod app;
mod auth;
mod call_skill;
mod capability;
mod checkpoints;
mod config_layers;
mod credentials;
mod error;
mod execute;
mod execution_index;
mod executions;
mod fleet;
mod generate;
mod goal_runs;
mod goals;
mod health;
mod helpers;
mod mcp;
mod mcp_protocol;
mod mcp_tools;
mod memory;
mod observability;
mod rerun;
mod rollout;
mod routes;
mod runs;
mod runtime_setup;
mod search_skills;
mod skill_resources;
mod skills;
mod startup;
mod state;
mod tasks;
mod types;
mod webhook;
mod workflow_source;

#[cfg(test)]
mod tests;


#[allow(unused_imports)]
pub(crate) use app::build_app;
#[allow(unused_imports)]
pub(crate) use startup::build_server_runtime;

/// Canonical default port for the APXM server. Both `DEFAULT_ADDR` and
/// `DEFAULT_PUBLIC_URL` embed this value; update all three together.
#[allow(dead_code)]
pub(crate) const DEFAULT_PORT: u16 = 18800;
pub(crate) const DEFAULT_ADDR: &str = "127.0.0.1:18800";
pub(crate) const DEFAULT_PUBLIC_URL: &str = "http://localhost:18800";

fn main() -> anyhow::Result<()> {
    let server_config = config_layers::server_config_from_layers()?;
    let workers = server_worker_threads(&server_config);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?
        .block_on(async_main(server_config))
}

async fn async_main(server_config: ServerConfig) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(log_filter(&server_config))
        .init();

    startup::run_server_with_config(server_config).await
}

fn server_worker_threads(server_config: &ServerConfig) -> usize {
    server_config
        .process
        .tokio_worker_threads
        .filter(|workers| *workers > 0)
        .unwrap_or_else(default_server_worker_threads)
}

fn default_server_worker_threads() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|threads| threads.get())
        .unwrap_or(4);
    (cores / 2).max(2)
}

fn log_filter(server_config: &ServerConfig) -> String {
    let filter = server_config.process.log_filter.trim();
    if filter.is_empty() {
        apxm_driver::ServerProcessConfig::default().log_filter
    } else {
        filter.to_string()
    }
}

