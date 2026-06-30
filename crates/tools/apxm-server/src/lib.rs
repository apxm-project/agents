//! APXM Server library — HTTP gateway for the APXM agent runtime.
//!
//! The `apxm-server` binary is a thin entrypoint; integration and contract tests
//! link against this crate.

#![allow(
    dead_code,
    clippy::assigning_clones,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::cast_possible_wrap,
    clippy::doc_lazy_continuation,
    clippy::format_push_string,
    clippy::if_same_then_else,
    clippy::large_enum_variant,
    clippy::large_futures,
    clippy::manual_let_else,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::result_large_err,
    clippy::struct_field_names,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_sort_by,
    clippy::unused_async,
    clippy::used_underscore_binding,
    clippy::zero_sized_map_values
)]

mod a2a;
mod agent;
mod app;
pub mod artifacts;
mod auth;
mod bind;
mod call_skill;
mod capability;
pub mod consent_broker;
mod capability_discovery;
mod checkpoints;
mod config_layers;
mod conversations;
mod credentials;
mod capability_grants;
mod enrollment;
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
mod metrics;
mod observability;
pub mod openapi;
pub mod permissions;
mod principal;
mod rerun;
mod rollout;
mod routes;
pub mod run_history;
mod runs;
mod runtime_setup;
mod safety;
mod search_skills;
mod sessions;
mod shutdown;
mod skill_resources;
mod skills;
mod startup;
mod state;
mod tasks;
pub mod test_support;
pub mod types;
mod webhook;
mod workflow_source;

#[cfg(test)]
mod tests;

pub(crate) use app::build_app;
#[cfg(test)]
pub(crate) use startup::build_server_runtime;

/// Canonical default port for the APXM server.
pub const DEFAULT_PORT: u16 = 18800;
pub const DEFAULT_ADDR: &str = "127.0.0.1:18800";
pub const DEFAULT_PUBLIC_URL: &str = "http://localhost:18800";

/// Process entry used by the `apxm-server` binary.
pub fn run() -> anyhow::Result<()> {
    let server_config = config_layers::server_config_from_layers()?;
    let workers = server_worker_threads(&server_config);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?
        .block_on(async_main(server_config))
}

async fn async_main(server_config: apxm_driver::ServerConfig) -> anyhow::Result<()> {
    let log_filter = log_filter(&server_config);
    match observability::init_tracing_subscriber(&server_config.observability, &log_filter) {
        Ok(Some(exporter)) => {
            tracing::info!(endpoint = %exporter.endpoint(), "OTLP exporter configured");
        }
        Ok(None) => {}
        Err(error) => observability::warn_init_failure(&error),
    }

    startup::run_server_with_config(server_config).await
}

fn server_worker_threads(server_config: &apxm_driver::ServerConfig) -> usize {
    server_config
        .process
        .tokio_worker_threads
        .filter(|workers| *workers > 0)
        .unwrap_or_else(default_server_worker_threads)
}

fn default_server_worker_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(4, |threads| threads.get());
    (cores / 2).max(2)
}

fn log_filter(server_config: &apxm_driver::ServerConfig) -> String {
    let filter = server_config.process.log_filter.trim();
    if filter.is_empty() {
        apxm_driver::ServerProcessConfig::default().log_filter
    } else {
        filter.to_string()
    }
}
