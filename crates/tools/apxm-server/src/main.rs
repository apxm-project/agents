//! APXM Server — HTTP gateway for the APXM agent runtime.
//!
//! Exposes the runtime's capabilities over a REST+SSE API with support for:
//! - **Graph execution**: `POST /v1/execute`, `POST /v1/execute/stream`
//! - **Skill library**: inventory, validation, REST/SSE execution, and
//!   execution-record lookup through `/v1/skills/*` and `/v1/executions/*`
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
//! `main.rs` only wires modules and process startup. Request handlers live in
//! focused modules, with graph execution delegated to `apxm-runtime` and
//! `apxm-compiler`, and static skill execution using `apxm-skill` manifests plus
//! `apxm-artifact` containers.

mod a2a;
mod agent;
mod app;
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
mod routes;
mod skills;
mod startup;
mod state;
mod tasks;
mod types;

#[cfg(test)]
mod tests;

pub(crate) use app::build_app;
pub(crate) use startup::build_server_runtime;

pub(crate) const DEFAULT_ADDR: &str = "127.0.0.1:18800";
pub(crate) const DEFAULT_PUBLIC_URL: &str = "http://localhost:18800";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,apxm_server=debug".to_string()),
        )
        .init();

    startup::run_server().await
}
