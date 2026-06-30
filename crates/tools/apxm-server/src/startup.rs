use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use apxm_core::constants::env as apxm_env;
use apxm_core::paths::ApxmPaths;
use apxm_driver::{ServerConfig, ServerExecutionsConfig};
use apxm_rollout::{IndexDb, RolloutPaths};
use apxm_runtime::{Runtime, RuntimeConfig, SchedulerConfig};
use apxm_core::types::consent::NoOpConsentBroker;
use apxm_runtime::host_dispatch::NoOpHostDispatchGateway;
use apxm_server_api::{AgentRuntimeApi, RuntimeApiAdapter};
use dashmap::DashMap;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::app::build_app;
use crate::bind::effective_require_auth;
use crate::checkpoints::CheckpointStore;
use crate::executions::ExecutionStore;
use crate::goal_runs::GoalRunRegistry;
use crate::rollout::RolloutRegistry;
use crate::run_history::storage::RunHistoryIndex;
use crate::runs::RunEventBus;
use crate::runtime_setup::{build_runtime_with_router, build_runtime_without_router};
use crate::safety::SafetyState;
use crate::shutdown::ShutdownCoordinator;
use crate::skill_resources::prepend_builtin_skill_root;
use crate::skills::{SkillLibrary, parse_skill_roots};
use crate::state::{AppState, InferenceLimiter};
use crate::tasks::TaskQueueManager;
use crate::webhook::WebhookDispatcher;

pub(crate) fn execution_store_from_paths(config: &ServerExecutionsConfig) -> ExecutionStore {
    let paths = ApxmPaths::discover().unwrap_or_else(|error| {
        panic!("failed to discover APXM paths for execution store: {error}")
    });
    let run_history_path = paths.sessions_dir_for_read().join("runs.sqlite");
    let run_history = RunHistoryIndex::open(&run_history_path).unwrap_or_else(|error| {
        panic!(
            "failed to open required run-history sqlite index at {}: {error}",
            run_history_path.display()
        )
    });
    info!(path = %run_history_path.display(), "opened run-history sqlite index");
    let store = ExecutionStore::from_session_roots_with_run_history(
        paths.session_lookup_dirs(),
        config.index_max_entries,
        run_history,
    );
    let loaded = store.list().len();
    if loaded > 0 {
        info!(count = loaded, "loaded persisted execution records");
    }
    store
}

// Default-config runtime constructor used exclusively by integration tests
// (see `tests/execute.rs`). The release binary builds its runtime through
// `run_server_with_config` with explicit CLI-derived configuration.
#[allow(dead_code)]
pub(crate) async fn build_server_runtime() -> Result<Runtime, apxm_core::error::RuntimeError> {
    build_runtime_without_router(server_runtime_config(&ServerConfig::default())).await
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
    //      `/v1/capability-templates` so the studio install-gate sees its
    //      blocks as AVAILABLE without any per-provider Rust.
    let pack_scan_roots = integration_capability_roots();
    crate::capability::rescan_pack_tools(&runtime, &pack_scan_roots, Arc::new(NoOpHostDispatchGateway));
    crate::capability_discovery::register(&runtime);
    crate::search_skills::register(&runtime, skill_library.clone());
    let mut runtime_arc = Arc::new(runtime);
    let (skill_resolver, workflow_spawner) = {
        let runtime_mut = Arc::get_mut(&mut runtime_arc).ok_or_else(|| {
            anyhow::anyhow!("failed to install runtime bridges after runtime was shared")
        })?;
        let skill_resolver =
            crate::call_skill::install_unattached(runtime_mut, skill_library.clone());
        let workflow_spawner =
            apxm_driver::runtime::install_workflow_spawner_unattached(runtime_mut, None);
        (skill_resolver, workflow_spawner)
    };
    skill_resolver.attach_runtime(&runtime_arc);
    workflow_spawner.attach_runtime(&runtime_arc);

    // Wrap Runtime in the trait adapter so AppState is decoupled from Arc<Runtime>.
    let runtime: Arc<dyn AgentRuntimeApi> = Arc::new(RuntimeApiAdapter(runtime_arc));

    // Optional outbound lifecycle webhook if configured.
    let webhook_dispatcher = WebhookDispatcher::from_config(&server_config.webhook);

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

    let addr = server_addr(&args, &server_config)?;
    let effective_auth = effective_require_auth(&addr, &server_config.auth);
    if effective_auth && !crate::bind::is_loopback_addr(&addr) {
        info!(%addr, "bearer auth enabled for non-loopback bind (default-deny)");
    }

    let state = AppState {
        runtime,
        host_dispatch: Arc::new(NoOpHostDispatchGateway),
        consent_broker: Arc::new(NoOpConsentBroker),
        host_consent_broker: Arc::new(crate::consent_broker::ConsentBroker::new()),
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
        rollout_registry: rollout_registry.clone(),
        inference_limiter: InferenceLimiter::from_config(&server_config.inference),
        server_config: server_config.clone(),
        bind_addr: addr,
        effective_require_auth: effective_auth,
        safety_state: SafetyState::from_config(&server_config.safety),
        shutdown: ShutdownCoordinator::new(),
        cancel_registry: Arc::new(DashMap::new()),
        goal_runs: GoalRunRegistry::new(),
        capability_grants: crate::capability_grants::CapabilityGrantStore::new(),
        session_registry: crate::conversations::SessionRegistry::new(),
    };

    let shutdown = state.shutdown.clone();
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Derive the announced address from the ACTUAL bound port: the configured
    // port may be 0 (OS-assigned ephemeral) in worktree-parallel stacks.
    let local = listener.local_addr()?;
    info!(addr = %local, "starting apxm-server");
    advertise_listen("apxm-server", local.port());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(
            shutdown,
            rollout_registry,
            server_config.shutdown.drain_timeout_secs,
        ))
        .await?;
    Ok(())
}

/// Shared advertise contract: announce the actually-bound port so an external
/// orchestrator can learn an OS-assigned ephemeral port. When `APXM_RUNTIME_DIR`
/// is set, atomically write `<dir>/<name>.json` describing the listener; always
/// print one `APXM_LISTEN <name> http://127.0.0.1:<port>` line to stdout.
fn advertise_listen(name: &str, port: u16) {
    use std::io::Write;

    if let Ok(dir) = std::env::var("APXM_RUNTIME_DIR")
        && !dir.is_empty()
        && let Err(error) = write_listen_registry(&dir, name, port)
    {
        warn!(%error, "failed to write listen registry file");
    }

    let mut stdout = std::io::stdout();
    let _ = writeln!(stdout, "APXM_LISTEN {name} http://127.0.0.1:{port}");
    let _ = stdout.flush();
}

fn write_listen_registry(dir: &str, name: &str, port: u16) -> std::io::Result<()> {
    let dir_path = std::path::Path::new(dir);
    if !dir_path.exists() {
        std::fs::create_dir_all(dir_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir_path, std::fs::Permissions::from_mode(0o700))?;
        }
    }

    let pid = std::process::id();
    let json =
        format!(r#"{{"pid":{pid},"addr":"127.0.0.1","port":{port},"scheme":"http","ready":true}}"#);
    let tmp = dir_path.join(format!("{name}.json.tmp.{pid}"));
    let final_path = dir_path.join(format!("{name}.json"));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &final_path)?;
    Ok(())
}

/// Build integration catalog roots scanned for `capabilities.toml` action blocks.
pub(crate) fn integration_capability_roots() -> Vec<std::path::PathBuf> {
    use apxm_core::constants::env::{APXM_INTEGRATIONS_ROOT, APXM_WORKSPACE_ROOT};

    let mut roots = Vec::new();
    if let Ok(root) = std::env::var(APXM_INTEGRATIONS_ROOT) {
        let root = std::path::PathBuf::from(root);
        if root.is_dir() && !roots.contains(&root) {
            roots.push(root);
        }
    }
    if let Ok(workspace) = std::env::var(APXM_WORKSPACE_ROOT) {
        let root = std::path::PathBuf::from(workspace).join("integrations");
        if root.is_dir() && !roots.contains(&root) {
            roots.push(root);
        }
    }
    if let Ok(paths) = ApxmPaths::discover() {
        for integration_root in paths.integrations_dirs() {
            if integration_root.is_dir() && !roots.contains(&integration_root) {
                roots.push(integration_root);
            }
        }
    }
    roots
}

/// Build the set of directories scanned for pack `tools.toml` action blocks:
#[deprecated(note = "use integration_capability_roots for connector capabilities")]
pub(crate) fn pack_capability_roots(skill_roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut roots = skill_roots.to_vec();
    if let Ok(root) = std::env::var(apxm_core::constants::env::APXM_LIBS_ROOT) {
        let root = std::path::PathBuf::from(root);
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
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
    let cores = std::thread::available_parallelism().map_or(4, |value| value.get());
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

async fn shutdown_signal(
    shutdown: ShutdownCoordinator,
    rollout_registry: RolloutRegistry,
    drain_timeout_secs: u64,
) {
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
    drain_and_flush_rollouts(shutdown, rollout_registry, drain_timeout_secs).await;
}

async fn drain_and_flush_rollouts(
    shutdown: ShutdownCoordinator,
    rollout_registry: RolloutRegistry,
    drain_timeout_secs: u64,
) {
    info!("shutdown signal received");
    let timeout = std::time::Duration::from_secs(drain_timeout_secs.max(1));
    shutdown.signal_drain();
    shutdown.wait_for_http_drain(timeout).await;
    match tokio::time::timeout(timeout, rollout_registry.flush_all()).await {
        Ok(()) => info!("rollout flush complete"),
        Err(_) => warn!(
            timeout_secs = drain_timeout_secs,
            "rollout flush timed out during graceful shutdown"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contract: `integration_capability_roots` includes workspace integrations
    /// so the server registers bundled catalog capabilities without restart.
    #[test]
    #[allow(unsafe_code)]
    fn integration_capability_roots_includes_workspace_integrations() {
        use std::env;

        let workspace = tempfile::tempdir().expect("temp workspace");
        let integrations = workspace.path().join("integrations");
        std::fs::create_dir_all(&integrations).expect("integrations dir");

        // SAFETY: single-threaded test; no concurrent env readers.
        unsafe { env::set_var(apxm_core::constants::env::APXM_WORKSPACE_ROOT, workspace.path()) };
        let roots = integration_capability_roots();
        unsafe { env::remove_var(apxm_core::constants::env::APXM_WORKSPACE_ROOT) };

        assert!(
            roots.iter().any(|root| root == &integrations),
            "APXM_WORKSPACE_ROOT/integrations must appear in integration capability roots"
        );
    }

    #[tokio::test]
    async fn shutdown_drain_waits_for_tracked_http_work() {
        let shutdown = ShutdownCoordinator::new();
        let guard = shutdown.track_request();
        let started = std::time::Instant::now();
        let drain = tokio::spawn(drain_and_flush_rollouts(
            shutdown.clone(),
            RolloutRegistry::new(),
            2,
        ));

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(shutdown.is_draining());
        assert!(!drain.is_finished());

        drop(guard);
        drain.await.expect("drain task");
        assert!(started.elapsed() >= std::time::Duration::from_millis(30));
        assert_eq!(shutdown.in_flight(), 0);
    }
}
