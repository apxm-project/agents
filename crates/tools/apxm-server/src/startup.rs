use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use apxm_backends::BackendRegistration;
use apxm_core::paths::ApxmPaths;
use apxm_driver::runtime::sandbox::configure_sandbox_registry;
use apxm_runtime::{Runtime, RuntimeConfig};
use dashmap::DashMap;
use tracing::{info, warn};

use crate::app::build_app;
use crate::checkpoints::CheckpointStore;
use crate::executions::ExecutionStore;
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
    let mut runtime = Runtime::new(RuntimeConfig::default()).await?;
    runtime.set_sandbox_registry(configure_sandbox_registry());
    Ok(runtime)
}

pub(crate) async fn run_server() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let skill_roots = parse_skill_roots(&args);
    let skill_library = SkillLibrary::new(skill_roots);

    let runtime = Arc::new(build_server_runtime().await?);
    load_llm_backends(&runtime).await;

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

async fn load_llm_backends(runtime: &Runtime) {
    let mut loaded = 0u32;
    let mut first_name: Option<String> = None;
    match apxm_credentials::BackendStore::open() {
        Ok(store) => match store.list() {
            Ok(backends) if backends.is_empty() => {
                warn!("backend store is empty - no LLM backends registered");
            }
            Ok(backends) => {
                for backend in backends {
                    match BackendRegistration::from_backend_config(&backend) {
                        Ok(registration) => {
                            if let Err(e) = registration.register(runtime.llm_registry()).await {
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
                if let Some(ref default) = first_name
                    && let Err(e) = runtime.llm_registry().set_default(default)
                {
                    warn!(backend = %default, error = %e, "failed to set default backend");
                }
                info!(count = loaded, "loaded LLM backends from backend store");
            }
            Err(e) => {
                warn!(error = %e, "failed to read backend store");
            }
        },
        Err(e) => {
            warn!(error = %e, "credential store unavailable - no LLM backends registered");
        }
    }
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
