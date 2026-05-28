use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use apxm_core::constants::env as apxm_env;
use apxm_core::paths::ApxmPaths;
use apxm_driver::{ApXmConfig, ServerConfig};
use apxm_rollout::{IndexDb, RolloutPaths};
use apxm_runtime::{Runtime, RuntimeConfig, SchedulerConfig};
use dashmap::DashMap;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::app::build_app;
use crate::checkpoints::CheckpointStore;
use crate::executions::ExecutionStore;
use crate::observability::{self, warn_init_failure};
use crate::rollout::RolloutRegistry;
use crate::runs::RunEventBus;
use crate::runtime_setup::{build_runtime_with_router, build_runtime_without_router};
use crate::skill_resources::prepend_builtin_skill_root;
use crate::skills::{SkillLibrary, parse_skill_roots};
use crate::state::{AppState, InferenceLimiter};
use crate::tasks::TaskQueueManager;
use crate::webhook::WebhookDispatcher;

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

// Default-config runtime constructor used exclusively by integration tests
// (see `tests/execute.rs`). The release binary builds its runtime through
// `run_server` with explicit CLI-derived configuration.
#[allow(dead_code)]
pub(crate) async fn build_server_runtime() -> Result<Runtime, apxm_core::error::RuntimeError> {
    build_runtime_without_router(server_runtime_config(&ServerConfig::default())).await
}

pub(crate) async fn run_server() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let server_config = server_config_from_layers()?;
    let skill_roots = prepend_builtin_skill_root(parse_skill_roots(&args));
    let skill_library = SkillLibrary::new(skill_roots);

    let runtime = build_runtime_with_router(server_runtime_config(&server_config)).await?;
    let mut runtime = Arc::new(runtime);
    crate::call_skill::install(&mut runtime, skill_library.clone());

    // Phase 14.8.C — wire the outbound lifecycle webhook if
    // `APXM_RUN_WEBHOOK_URL` is set. Optional + fire-and-forget.
    let webhook_dispatcher = WebhookDispatcher::from_env();

    // Phase 14.8.D — bring up the OTEL exporter if env-configured.
    // Initialization failures are logged + ignored: the in-process
    // tracing-subscriber keeps working.
    match observability::init() {
        Ok(_exporter) => {}
        Err(error) => warn_init_failure(&error),
    }

    // Phase 14.8.E — bring up the rollout layer. The index db is rebuilt
    // lazily from disk on first read if missing/corrupt; opening here is
    // fast and surfaces permission/path issues at boot.
    let rollout_paths = Arc::new(RolloutPaths::from_env());
    let rollout_index = match IndexDb::open(&rollout_paths.index_db_path()) {
        Ok(db) => Arc::new(Mutex::new(db)),
        Err(error) => {
            warn!(%error, "failed to open rollout index db; using in-memory");
            Arc::new(Mutex::new(
                IndexDb::open_in_memory().expect("in-memory rollout index"),
            ))
        }
    };
    let rollout_registry = RolloutRegistry::new();

    let state = AppState {
        runtime,
        agent_registry: Arc::new(DashMap::new()),
        task_manager: TaskQueueManager::new(),
        checkpoint_store: CheckpointStore::new(),
        start_time: SystemTime::now(),
        a2a_tasks: Arc::new(DashMap::new()),
        skill_library,
        execution_store: execution_store_from_paths(),
        run_event_bus: RunEventBus::with_config(&server_config.run_events),
        webhook_dispatcher,
        rollout_paths,
        rollout_index,
        rollout_registry,
        inference_limiter: InferenceLimiter::from_config(&server_config.inference),
        server_config: server_config.clone(),
    };

    let app = build_app(state);
    let addr = server_addr(&args, &server_config)?;
    info!(%addr, "starting apxm-server");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn server_config_from_layers() -> anyhow::Result<ServerConfig> {
    let mut config = ApXmConfig::load_scoped()?.server;
    apply_server_env_overrides(&mut config);
    Ok(config)
}

fn apply_server_env_overrides(config: &mut ServerConfig) {
    if let Some(value) = env_usize(apxm_env::APXM_RUNTIME_MAX_CONCURRENCY) {
        config.runtime.max_concurrency = Some(value);
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUNTIME_MAX_INFLIGHT) {
        config.runtime.max_inflight = Some(value);
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUNTIME_LLM_INFLIGHT) {
        config.runtime.llm_inflight = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_SERVER_MAX_INFERENCE) {
        config.inference.max_concurrent = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_SERVER_INFERENCE_WAIT_MS) {
        config.inference.acquire_timeout_ms = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_GENERATE_STREAM_CHANNEL_CAPACITY) {
        config.generate_stream.channel_capacity = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_GENERATE_STREAM_TIMEOUT_SECS) {
        config.generate_stream.inactivity_timeout_secs = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_GENERATE_STREAM_KEEP_ALIVE_SECS) {
        config.generate_stream.keep_alive_secs = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_EXECUTION_STREAM_CHANNEL_CAPACITY) {
        config.execution_stream.channel_capacity = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_EXECUTION_STREAM_KEEP_ALIVE_SECS) {
        config.execution_stream.keep_alive_secs = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUN_EVENT_STREAM_BUFFER) {
        config.run_events.stream_buffer = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUN_EVENT_RETAINED_EVENTS) {
        config.run_events.retained_events = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_RUN_EVENT_KEEP_ALIVE_SECS) {
        config.run_events.keep_alive_secs = value;
    }
    if let Ok(value) = std::env::var(apxm_env::APXM_PUBLIC_URL) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            config.public_url = Some(trimmed.to_string());
        }
    }
}

fn server_runtime_config(server_config: &ServerConfig) -> RuntimeConfig {
    let mut config = RuntimeConfig::default();
    let cores = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4);
    let default_compute = (cores / 2).max(2);

    let max_concurrency = server_config
        .runtime
        .max_concurrency
        .unwrap_or(default_compute);
    let max_inflight = server_config
        .runtime
        .max_inflight
        .unwrap_or_else(|| max_concurrency.saturating_mul(2).max(1));

    config.scheduler_config = SchedulerConfig::default()
        .with_max_concurrency(max_concurrency)
        .with_max_inflight(max_inflight)
        .with_llm_inflight(server_config.runtime.llm_inflight);
    config
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn server_addr(args: &[String], config: &ServerConfig) -> anyhow::Result<SocketAddr> {
    // Parse --port from CLI args (service-manager passes `--port <N>`)
    let cli_port = args
        .iter()
        .position(|arg| arg == "--port")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<u16>().ok());

    if let Some(port) = cli_port {
        return Ok(SocketAddr::from(([127, 0, 0, 1], port)));
    }

    if let Ok(value) = std::env::var(apxm_env::APXM_SERVER_ADDR) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed
                .parse::<SocketAddr>()
                .map_err(|error| anyhow::anyhow!("invalid APXM_SERVER_ADDR '{trimmed}': {error}"));
        }
    }

    if let Some(value) = config.bind_addr.as_deref() {
        return value
            .parse::<SocketAddr>()
            .map_err(|error| anyhow::anyhow!("invalid [server].bind_addr '{value}': {error}"));
    }

    crate::DEFAULT_ADDR
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid built-in default address: {error}"))
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
