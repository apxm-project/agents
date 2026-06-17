//! Public helpers for integration tests in `tests/`.

use std::sync::Arc;
use std::time::SystemTime;

use apxm_backends::llm::backends::MockLLMBackend;
use apxm_driver::{RunEventsConfig, ServerConfig};
use apxm_runtime::{ModelRouterConfig, Runtime, RuntimeConfig};
use axum::Router;
use dashmap::DashMap;

use crate::build_app;
use crate::executions::ExecutionStore;
use crate::runs::RunEventBus;
use crate::skills::SkillLibrary;
use crate::state::AppState;

const FIXTURE_WORKFLOW_BACKEND: &str = "mock-workflow";

/// Test harness with router + observer-bus access for integration tests.
pub struct TestHarness {
    pub router: Router,
    run_event_bus: RunEventBus,
}

impl TestHarness {
    pub fn first_execution_id(&self) -> Option<String> {
        self.run_event_bus
            .list_execution_ids()
            .into_iter()
            .next()
    }
}

/// Build a test router with session API routes mounted (contract tests).
pub async fn contract_app_with_mock(mock: MockLLMBackend) -> Router {
    let mut runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    runtime
        .llm_registry()
        .register(FIXTURE_WORKFLOW_BACKEND, mock)
        .expect("register mock backend");
    runtime
        .llm_registry()
        .set_default(FIXTURE_WORKFLOW_BACKEND)
        .expect("set mock backend default");
    runtime
        .init_model_router(ModelRouterConfig::default())
        .expect("init model router");
    let state = test_state_with_runtime(Arc::new(runtime), ServerConfig::default()).await;
    build_app(state)
}

/// Build a test harness with custom server config and mock LLM backend.
pub async fn test_harness_with_config_and_mock(
    server_config: ServerConfig,
    mock: MockLLMBackend,
) -> TestHarness {
    let mut runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    runtime
        .llm_registry()
        .register(FIXTURE_WORKFLOW_BACKEND, mock)
        .expect("register mock backend");
    runtime
        .llm_registry()
        .set_default(FIXTURE_WORKFLOW_BACKEND)
        .expect("set mock backend default");
    runtime
        .init_model_router(ModelRouterConfig::default())
        .expect("init model router");
    let state = test_state_with_runtime(Arc::new(runtime), server_config).await;
    let run_event_bus = state.run_event_bus.clone();
    TestHarness {
        router: build_app(state),
        run_event_bus,
    }
}

/// Build a test router with default server config and an optional mock LLM backend.
pub async fn test_app_with_mock(mock: MockLLMBackend) -> Router {
    test_harness_with_config_and_mock(ServerConfig::default(), mock)
        .await
        .router
}

/// Build a test router with custom server config and mock LLM backend.
pub async fn test_app_with_config_and_mock(
    server_config: ServerConfig,
    mock: MockLLMBackend,
) -> Router {
    test_harness_with_config_and_mock(server_config, mock)
        .await
        .router
}

/// Shared ASK AIR fixture that drives the mock LLM backend.
pub fn single_ask_air() -> String {
    r#"module {
  func.func @chat() -> !ais.token attributes {ais.entry} {
    %reply = ais.ask "say hello" : !ais.token
    func.return %reply : !ais.token
  }
}
"#
    .to_string()
}

/// Default mock backend name used by [`test_app_with_mock`].
pub fn fixture_workflow_backend() -> &'static str {
    FIXTURE_WORKFLOW_BACKEND
}

async fn test_state_with_runtime(runtime: Arc<Runtime>, server_config: ServerConfig) -> AppState {
    let mut runtime = runtime;
    let skill_library = SkillLibrary::new(Vec::new());
    install_test_runtime_bridges(&mut runtime, skill_library.clone());
    let rollout_home = std::env::temp_dir().join(format!(
        "apxm-test-rollout-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&rollout_home).expect("rollout home");
    AppState {
        runtime,
        agent_registry: Arc::new(DashMap::new()),
        task_manager: crate::tasks::TaskQueueManager::new(),
        checkpoint_store: crate::checkpoints::CheckpointStore::new(),
        start_time: SystemTime::now(),
        a2a_tasks: Arc::new(DashMap::new()),
        skill_library,
        execution_store: ExecutionStore::with_index_max_entries(256),
        run_event_bus: crate::runs::RunEventBus::with_config(&server_config.run_events),
        webhook_dispatcher: None,
        rollout_paths: Arc::new(apxm_rollout::RolloutPaths::new(rollout_home)),
        rollout_index: Arc::new(tokio::sync::Mutex::new(
            apxm_rollout::IndexDb::open_in_memory().expect("rollout index"),
        )),
        rollout_registry: crate::rollout::RolloutRegistry::with_config(&server_config.rollout),
        inference_limiter: crate::state::InferenceLimiter::from_config(&server_config.inference),
        server_config,
        cancel_registry: Arc::new(DashMap::new()),
        goal_runs: crate::goal_runs::GoalRunRegistry::new(),
        session_registry: crate::conversations::SessionRegistry::new(),
    }
}

fn install_test_runtime_bridges(runtime: &mut Arc<Runtime>, skill_library: SkillLibrary) {
    let (skill_resolver, workflow_spawner) = {
        let runtime_mut = Arc::get_mut(runtime)
            .expect("test runtime bridges must be installed before runtime is shared");
        let skill_resolver = crate::call_skill::install_unattached(runtime_mut, skill_library);
        let workflow_spawner =
            apxm_driver::runtime::install_workflow_spawner_unattached(runtime_mut, None);
        (skill_resolver, workflow_spawner)
    };
    skill_resolver.attach_runtime(runtime);
    workflow_spawner.attach_runtime(runtime);
}

/// Small observer-bus buffer for backpressure fixtures (SC-001).
pub fn backpressure_run_events_config() -> RunEventsConfig {
    RunEventsConfig {
        stream_buffer: 8,
        retained_events: 256,
        ..RunEventsConfig::default()
    }
}
