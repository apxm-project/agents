use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use apxm_core::constants::env as apxm_env;
use apxm_core::paths::ApxmPaths;
use apxm_driver::{ServerConfig, ServerExecutionsConfig};
use apxm_rollout::{IndexDb, RolloutPaths};
use apxm_runtime::{Runtime, RuntimeConfig, SchedulerConfig};
use dashmap::DashMap;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::app::build_app;
use crate::checkpoints::CheckpointStore;
use crate::executions::ExecutionStore;
use crate::goal_runs::GoalRunRegistry;
use crate::observability::{self, warn_init_failure};
use crate::rollout::RolloutRegistry;
use crate::runs::RunEventBus;
use crate::runtime_setup::{build_runtime_with_router, build_runtime_without_router};
use crate::skill_resources::prepend_builtin_skill_root;
use crate::skills::{SkillLibrary, parse_skill_roots};
use crate::state::{AppState, InferenceLimiter};
use crate::tasks::TaskQueueManager;
use crate::webhook::WebhookDispatcher;

pub(crate) fn execution_store_from_paths(config: &ServerExecutionsConfig) -> ExecutionStore {
    match ApxmPaths::discover() {
        Ok(paths) => {
            let store = ExecutionStore::from_session_roots_with_index_max_entries(
                paths.session_lookup_dirs(),
                config.index_max_entries,
            );
            let loaded = store.list().len();
            if loaded > 0 {
                info!(count = loaded, "loaded persisted execution records");
            }
            store
        }
        Err(error) => {
            warn!(%error, "failed to discover APXM paths for execution record reload");
            ExecutionStore::with_index_max_entries(config.index_max_entries)
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

#[allow(dead_code)]
pub(crate) async fn run_server() -> anyhow::Result<()> {
    run_server_with_config(crate::config_layers::server_config_from_layers()?).await
}

pub(crate) async fn run_server_with_config(server_config: ServerConfig) -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let skill_roots = prepend_builtin_skill_root(parse_skill_roots(&args));
    let skill_library = SkillLibrary::new(skill_roots.clone());

    let task_manager = TaskQueueManager::new();
    let runtime = build_runtime_with_router(
        server_runtime_config(&server_config),
        Some(crate::tasks::scheduled_prompt_on_fire(task_manager.clone())),
    )
    .await?;
    // Register pack action blocks as capabilities from two on-disk sources:
    //   1. the skill roots (a pack colocated with its skills), and
    //   2. `~/.apxm/libs/<pack>/` — the connector-pack library the studio seeds
    //      `pack.toml` + `tools.toml` into. Scanning the libs roots is the
    //      companion hop that makes an installed connector pack show up in
    //      `/v1/capabilities` so the studio install-gate sees its blocks as
    //      AVAILABLE without any per-provider Rust.
    let pack_scan_roots = pack_capability_roots(&skill_roots);
    crate::capability::register_pack_tools(&runtime, &pack_scan_roots);
    crate::search_skills::register(&runtime, skill_library.clone());
    let mut runtime = Arc::new(runtime);
    let (skill_resolver, workflow_spawner) = {
        let runtime_mut = Arc::get_mut(&mut runtime).ok_or_else(|| {
            anyhow::anyhow!("failed to install runtime bridges after runtime was shared")
        })?;
        let skill_resolver =
            crate::call_skill::install_unattached(runtime_mut, skill_library.clone());
        let workflow_spawner =
            apxm_driver::runtime::install_workflow_spawner_unattached(runtime_mut, None);
        (skill_resolver, workflow_spawner)
    };
    skill_resolver.attach_runtime(&runtime);
    workflow_spawner.attach_runtime(&runtime);

    // wire the outbound lifecycle webhook if configured.
    // Optional + fire-and-forget.
    let webhook_dispatcher = WebhookDispatcher::from_config(&server_config.webhook);

    // bring up the OTEL exporter if env-configured.
    // Initialization failures are logged + ignored: the in-process
    // tracing-subscriber keeps working.
    match observability::init(&server_config.observability) {
        Ok(_exporter) => {}
        Err(error) => warn_init_failure(&error),
    }

    // bring up the rollout layer. The index db is rebuilt
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
    let rollout_registry = RolloutRegistry::with_config(&server_config.rollout);

    // Durable checkpoint store so a parked PAUSE's checkpoint + resumed input
    // survive a server restart. Falls back to volatile in-memory on open error.
    let checkpoint_store = {
        let cp_path = rollout_paths.apxm_home.join("checkpoints.sqlite");
        match CheckpointStore::open(&cp_path) {
            Ok(s) => s,
            Err(error) => {
                warn!(%error, "failed to open durable checkpoint store; using in-memory");
                CheckpointStore::new()
            }
        }
    };

    let state = AppState {
        runtime,
        agent_registry: Arc::new(DashMap::new()),
        task_manager,
        checkpoint_store,
        start_time: SystemTime::now(),
        a2a_tasks: Arc::new(DashMap::new()),
        skill_library,
        execution_store: execution_store_from_paths(&server_config.executions),
        run_event_bus: RunEventBus::with_config(&server_config.run_events),
        webhook_dispatcher,
        rollout_paths,
        rollout_index,
        rollout_registry,
        inference_limiter: InferenceLimiter::from_config(&server_config.inference),
        server_config: server_config.clone(),
        cancel_registry: Arc::new(DashMap::new()),
        goal_runs: GoalRunRegistry::new(),
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

/// Build the set of directories scanned for pack `tools.toml` action blocks:
/// the skill roots plus the connector-pack library roots (`~/.apxm/libs`). Libs
/// roots are resolved from `ApxmPaths`; a discovery failure degrades to scanning
/// only the skill roots (the libs hop is additive, never fatal).
fn pack_capability_roots(skill_roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut roots = skill_roots.to_vec();
    match ApxmPaths::discover() {
        Ok(paths) => {
            for lib_root in paths.libs_dirs() {
                if !roots.contains(&lib_root) {
                    roots.push(lib_root);
                }
            }
        }
        Err(error) => {
            warn!(%error, "failed to discover APXM paths for connector-pack libs scan");
        }
    }
    roots
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
        .with_llm_inflight(server_config.runtime.llm_inflight)
        // Capture per-token outputs so a later `rerun-from-node` can recover this
        // run's values and seed a partial replay (the upstream-node boundary).
        .with_collect_all_outputs(true);
    config.llm_tool_dispatch.max_parallel_tool_calls =
        server_config.runtime.max_parallel_tool_calls;
    config
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
