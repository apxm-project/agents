use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use apxm_core::paths::ApxmPaths;
use apxm_runtime::{Runtime, RuntimeConfig};
use dashmap::DashMap;
use tracing::{info, warn};

use crate::app::build_app;
use crate::checkpoints::CheckpointStore;
use crate::executions::ExecutionStore;
use crate::runtime_setup::{build_runtime_with_router, build_runtime_without_router};
use crate::skill_resources::prepend_builtin_skill_root;
use crate::skills::{SkillLibrary, parse_skill_roots};
use crate::state::AppState;
use crate::tasks::TaskQueueManager;

pub(crate) fn execution_store_from_paths() -> ExecutionStore {
    match ApxmPaths::discover() {
        Ok(paths) => {
            let store = ExecutionStore::from_session_roots(paths.session_lookup_dirs());
            let loaded = store.list().len();
            if loaded > 0 {
                info!(count = loaded, "loaded persisted execution records");
            }
            store
        }
        Err(error) => {
            warn!(%error, "failed to discover APXM paths for execution record reload");
            ExecutionStore::new()
        }
    }
}

pub(crate) async fn build_server_runtime() -> Result<Runtime, apxm_core::error::RuntimeError> {
    build_runtime_without_router(RuntimeConfig::default()).await
}

pub(crate) async fn run_server() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let skill_roots = prepend_builtin_skill_root(parse_skill_roots(&args));
    let skill_library = SkillLibrary::new(skill_roots);

    let runtime = Arc::new(build_runtime_with_router(RuntimeConfig::default()).await?);

    let state = AppState {
        runtime,
        agent_registry: Arc::new(DashMap::new()),
        task_manager: TaskQueueManager::new(),
        checkpoint_store: CheckpointStore::new(),
        start_time: SystemTime::now(),
        a2a_tasks: Arc::new(DashMap::new()),
        skill_library,
        execution_store: execution_store_from_paths(),
    };

    let app = build_app(state);
    let addr = server_addr(&args);
    info!(%addr, "starting apxm-server");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn server_addr(args: &[String]) -> SocketAddr {
    // Parse --port from CLI args (service-manager passes `--port <N>`)
    let cli_port = args
        .iter()
        .position(|arg| arg == "--port")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<u16>().ok());

    if let Some(port) = cli_port {
        SocketAddr::from(([127, 0, 0, 1], port))
    } else {
        std::env::var("APXM_SERVER_ADDR")
            .ok()
            .and_then(|value| value.parse::<SocketAddr>().ok())
            .unwrap_or_else(|| crate::DEFAULT_ADDR.parse().expect("valid default addr"))
    }
}

async fn shutdown_signal() {
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
}
