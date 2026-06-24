use apxm_backends::llm::backends::{MockLLMBackend, MockResponse};
use apxm_backends::{BackendRegistration, LLMRegistry};
use apxm_core::constants::env as apxm_env;
use apxm_driver::runtime::agents::configure_agent_registry;
use apxm_driver::runtime::sandbox::configure_sandbox_registry;
use apxm_runtime::capability::builtins::{FiredSchedule, OnFire};
use apxm_runtime::{ModelRouterConfig, Runtime, RuntimeConfig};
use tracing::{info, warn};

pub(crate) async fn build_runtime_with_router(
    config: RuntimeConfig,
    schedule_on_fire: Option<OnFire>,
) -> Result<Runtime, apxm_core::error::RuntimeError> {
    let mut runtime = Runtime::new(config).await?;
    let sandbox_registry = configure_sandbox_registry();
    runtime.set_sandbox_registry(std::sync::Arc::clone(&sandbox_registry));
    register_builtin_capabilities(&runtime, schedule_on_fire);
    load_llm_backends(&runtime).await;
    configure_agent_registry(
        runtime.process_table(),
        runtime.capability_system_arc(),
        sandbox_registry,
    )
    .await
    .map_err(|error| apxm_core::error::RuntimeError::State(error.to_string()))?;
    // v0: KIND 2 ACP agents run in-process via the process table's default
    // spawner; the remote runner plane (apxm-runner) is removed.
    runtime.init_model_router(ModelRouterConfig::default())?;
    // Professional-agent middleware chain (dispatcher chokepoint). A generous
    // per-node timeout bounds a hung node without tripping normal multi-agent
    // turns; the token-budget guard is a no-op unless an execution carries a
    // budget. Both are safe to enable globally. (LoopGuard is intentionally
    // left to explicit per-graph config — its repeat fingerprinting can reject
    // legitimate deterministic retries in multi-agent loops.)
    runtime.add_middleware(std::sync::Arc::new(apxm_runtime::TimeoutMiddleware::new(
        Some(std::time::Duration::from_secs(300)),
    )));
    runtime.add_middleware(std::sync::Arc::new(
        apxm_runtime::TokenBudgetMiddleware::new(),
    ));
    runtime.add_middleware(std::sync::Arc::new(
        apxm_runtime::ConversationMemoryMiddleware::new(),
    ));

    // Production permission gate: a generic always-invoked PEP at the
    // capability chokepoint, complementing the in-handler write boundary. It
    // activates the `requires_auth` capability-metadata flag — advisory by
    // default, denying when APXM_REQUIRE_AUTH_STRICT is set. Composed here by the
    // trusted host (capabilities are already registered above); the AIR program
    // can neither add, remove, nor reorder it.
    {
        let strict =
            std::env::var("APXM_REQUIRE_AUTH_STRICT").is_ok_and(|v| !v.is_empty() && v != "0");
        let requires_auth: std::collections::HashSet<String> = runtime
            .capability_system()
            .list_capabilities()
            .into_iter()
            .filter(|meta| meta.requires_auth)
            .map(|meta| meta.name)
            .collect();
        runtime
            .capability_system()
            .register_interceptor(std::sync::Arc::new(
                apxm_runtime::PermissionInterceptor::new(requires_auth, strict),
            ));
    }

    Ok(runtime)
}

/// Register the runtime's builtin tool capabilities so `inv_tool` nodes are
/// admitted by raw `/v1/execute` (which checks the capability system). Without
/// this the system starts empty and every tool node is rejected.
fn register_builtin_capabilities(runtime: &Runtime, schedule_on_fire: Option<OnFire>) {
    use apxm_runtime::capability::builtins::{
        BashCapability, ComposeWorkflowCapability, CountTokensCapability, HttpGetCapability,
        HttpPostCapability, ManageTaskCapability, McpBridgeCapability, ProviderCallCapability,
        ReadCapability, RunWorkflowCapability, ScheduleCapability, ToolsStore, WriteCapability,
    };
    use apxm_runtime::capability::executor::CapabilityExecutor;
    use std::sync::Arc;
    let sys = runtime.capability_system();
    // NB: capabilities must NOT build a reqwest client in their constructor —
    // building one during runtime setup wedged the executor completion path.
    // http_get/http_post use a lazily-initialized shared client. search_web is
    // omitted here (eager client + needs an API key).
    //
    // Read is confined to APXM_READ_BASE (defaulting to the process cwd): any
    // resolved path that escapes the base is rejected.
    let read_base = std::env::var_os("APXM_READ_BASE")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let read_capability: Arc<dyn CapabilityExecutor> = match read_base {
        Some(base) => Arc::new(ReadCapability::new_with_base_directory(base)),
        None => Arc::new(ReadCapability::new()),
    };
    let mut caps: Vec<Arc<dyn CapabilityExecutor>> = vec![
        Arc::new(HttpGetCapability::new()),
        Arc::new(HttpPostCapability::new()),
        read_capability,
        Arc::new(WriteCapability::new()),
        Arc::new(BashCapability::new()),
        Arc::new(ProviderCallCapability::new()),
        Arc::new(McpBridgeCapability::new()),
        Arc::new(CountTokensCapability::new()),
        // Workflow authoring is write-class and confined to the `authoring`
        // group so the conversational agent needs delegated authority.
        Arc::new(ComposeWorkflowCapability::new()),
        Arc::new(RunWorkflowCapability::new()),
    ];

    // Durable agent-management tools (schedule + manage_task). These are always
    // registered (BUILTINS allowlist parity) and back onto a single SQLite file
    // under the state home. The schedule firer fires due wakeups in-process via
    // the park registry; armed schedules and the task tree survive a restart.
    let store_path =
        apxm_core::env::state_home().join(apxm_core::constants::agent_tools::STORE_FILENAME);
    match ToolsStore::open(&store_path) {
        Ok(store) => {
            restore_tasks(runtime.aam(), &store);
            let arm = Arc::new(tokio::sync::Notify::new());
            caps.push(Arc::new(ManageTaskCapability::new(
                runtime.aam().clone(),
                store.clone(),
            )));
            caps.push(Arc::new(ScheduleCapability::new(
                store.clone(),
                arm.clone(),
            )));
            let on_fire = schedule_on_fire.unwrap_or_else(default_schedule_on_fire);
            apxm_runtime::capability::builtins::spawn_firer(store, arm, Some(on_fire));
        }
        Err(error) => {
            warn!(%error, "failed to open agent tools store; schedule/manage_task unavailable");
        }
    }

    let mut n = 0u32;
    for cap in caps {
        let name = cap.metadata().name.clone();
        match sys.register(cap) {
            Ok(()) => n += 1,
            Err(e) => {
                warn!(capability = %name, error = %e, "failed to register builtin capability");
            }
        }
    }
    info!(count = n, "registered builtin tool capabilities");
}

fn default_schedule_on_fire() -> OnFire {
    std::sync::Arc::new(log_schedule_fire)
}

fn log_schedule_fire(fired: FiredSchedule) {
    info!(
        target: "apxm::schedule",
        schedule_id = %fired.id,
        kind = %fired.kind,
        recurring = fired.recurring,
        prompt = fired.prompt.as_deref().unwrap_or(""),
        payload = %fired.payload,
        "schedule fired"
    );
}

/// Rehydrate the runtime's goal tree from the durable task store so tasks
/// created in a previous run are visible to `manage_task` after a restart.
fn restore_tasks(aam: &apxm_runtime::Aam, store: &apxm_runtime::capability::builtins::ToolsStore) {
    use apxm_runtime::{Goal, TransitionLabel};
    let rows = match store.load_tasks() {
        Ok(rows) => rows,
        Err(error) => {
            warn!(%error, "failed to load tasks from store");
            return;
        }
    };
    let mut restored = 0u32;
    for row in rows {
        let Ok(goal) = serde_json::from_str::<Goal>(&row.json) else {
            continue;
        };
        let goal_id = goal.id;
        let label = TransitionLabel::custom("manage_task:restore");
        match goal.parent_id {
            Some(parent_id) => {
                aam.add_child_goal(parent_id, goal, label);
            }
            None => {
                aam.add_goal(goal, label);
            }
        }
        if let Some(policy) = row
            .policy
            .as_deref()
            .and_then(apxm_runtime::capability::builtins::parse_policy)
        {
            aam.set_completion_policy(goal_id, policy);
        }
        restored += 1;
    }
    if restored > 0 {
        info!(count = restored, "restored tasks from durable store");
    }
}

#[allow(dead_code)]
pub(crate) async fn build_runtime_without_router(
    config: RuntimeConfig,
) -> Result<Runtime, apxm_core::error::RuntimeError> {
    let mut runtime = Runtime::new(config).await?;
    let sandbox_registry = configure_sandbox_registry();
    runtime.set_sandbox_registry(std::sync::Arc::clone(&sandbox_registry));
    configure_agent_registry(
        runtime.process_table(),
        runtime.capability_system_arc(),
        sandbox_registry,
    )
    .await
    .map_err(|error| apxm_core::error::RuntimeError::State(error.to_string()))?;
    Ok(runtime)
}

pub(crate) async fn load_llm_backends(runtime: &Runtime) {
    if let Some(latency_ms) = mock_backend_latency_from_env() {
        match register_mock_llm_backend(runtime.llm_registry(), latency_ms) {
            Ok(()) => {
                info!(latency_ms, "registered env-controlled mock LLM backend");
                return;
            }
            Err(error) => {
                warn!(%error, "failed to register env-controlled mock LLM backend");
            }
        }
    }

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

fn mock_backend_latency_from_env() -> Option<u64> {
    let enabled = std::env::var(apxm_env::APXM_MOCK_BACKEND).is_ok_and(|value| {
        let value = value.trim();
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    });
    if !enabled {
        return None;
    }

    Some(
        std::env::var(apxm_env::APXM_MOCK_LATENCY_MS)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(25),
    )
}

fn register_mock_llm_backend(registry: &LLMRegistry, latency_ms: u64) -> Result<(), String> {
    let mock = MockLLMBackend::new()
        .with_latency_ms(latency_ms)
        .default(MockResponse::new("Mock LLM response from apxm-server."));
    registry
        .register("mock", mock)
        .map_err(|error| format!("failed to register mock backend: {error}"))?;
    registry
        .set_default("mock")
        .map_err(|error| format!("failed to set mock backend default: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_mock_llm_backend_sets_default() {
        let runtime = Runtime::new(RuntimeConfig::in_memory())
            .await
            .expect("runtime");

        register_mock_llm_backend(runtime.llm_registry(), 0).expect("mock backend");

        assert!(
            runtime
                .llm_registry()
                .backend_names()
                .iter()
                .any(|name| name == "mock")
        );
    }
}
