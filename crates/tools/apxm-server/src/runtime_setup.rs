use apxm_backends::BackendRegistration;
use apxm_driver::runtime::agents::configure_agent_registry;
use apxm_driver::runtime::sandbox::configure_sandbox_registry;
use apxm_runtime::{ModelRouterConfig, Runtime, RuntimeConfig};
use tracing::{info, warn};

pub(crate) async fn build_runtime_with_router(
    config: RuntimeConfig,
) -> Result<Runtime, apxm_core::error::RuntimeError> {
    let mut runtime = Runtime::new(config).await?;
    let sandbox_registry = configure_sandbox_registry();
    runtime.set_sandbox_registry(std::sync::Arc::clone(&sandbox_registry));
    register_builtin_capabilities(&runtime);
    load_llm_backends(&runtime).await;
    // Wire the ACP agent spawner so SPAWN_AGENT can launch real subprocess
    // agents (claude, codex, …) through /v1/execute. Best-effort: a failure
    // here only disables agent spawning, it must not abort server startup.
    if let Err(e) = configure_agent_registry(
        runtime.process_table(),
        runtime.capability_system_arc(),
        sandbox_registry,
    )
    .await
    {
        warn!(error = %e, "failed to configure ACP agent spawner; SPAWN_AGENT unavailable");
    }
    runtime.init_model_router(ModelRouterConfig::default())?;
    // Professional-agent middleware chain (dispatcher chokepoint). A generous
    // per-node timeout bounds a hung node without tripping normal multi-agent
    // turns; the token-budget guard is a no-op unless an execution carries a
    // budget. Both are safe to enable globally. (LoopGuard is intentionally
    // left to explicit per-graph config — its repeat fingerprinting can reject
    // legitimate deterministic retries in multi-agent loops.)
    runtime.add_middleware(std::sync::Arc::new(
        apxm_runtime::TimeoutMiddleware::new(Some(std::time::Duration::from_secs(300))),
    ));
    runtime.add_middleware(std::sync::Arc::new(
        apxm_runtime::TokenBudgetMiddleware::new(),
    ));
    runtime.add_middleware(std::sync::Arc::new(
        apxm_runtime::ConversationMemoryMiddleware::new(),
    ));
    Ok(runtime)
}

/// Register the runtime's builtin tool capabilities so `inv_tool` nodes are
/// admitted by raw `/v1/execute` (which checks the capability system). Without
/// this the system starts empty and every tool node is rejected.
fn register_builtin_capabilities(runtime: &Runtime) {
    use apxm_runtime::capability::builtins::{
        BashCapability, CountTokensCapability, HttpGetCapability, HttpPostCapability,
        McpBridgeCapability, ProviderCallCapability, ReadCapability, WriteCapability,
    };
    use apxm_runtime::capability::executor::CapabilityExecutor;
    use std::sync::Arc;
    let sys = runtime.capability_system();
    // NB: capabilities must NOT build a reqwest client in their constructor —
    // building one during runtime setup wedged the executor completion path.
    // http_get/http_post use a lazily-initialized shared client. search_web is
    // omitted here (eager client + needs an API key).
    //
    // Read is confined to APXM_READ_BASE (defaulting to the process cwd): a
    // relative path resolves under it and any resolved path escaping it is
    // rejected, keeping the builtin's default read profile otherwise intact.
    let read_base = std::env::var_os("APXM_READ_BASE")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let read_capability: Arc<dyn CapabilityExecutor> = match read_base {
        Some(base) => Arc::new(ReadCapability::new_with_base_directory(base)),
        None => Arc::new(ReadCapability::new()),
    };
    let caps: Vec<Arc<dyn CapabilityExecutor>> = vec![
        Arc::new(HttpGetCapability::new()),
        Arc::new(HttpPostCapability::new()),
        read_capability,
        Arc::new(WriteCapability::new()),
        Arc::new(BashCapability::new()),
        Arc::new(ProviderCallCapability::new()),
        Arc::new(McpBridgeCapability::new()),
        Arc::new(CountTokensCapability::new()),
    ];
    let mut n = 0u32;
    for cap in caps {
        let name = cap.metadata().name.clone();
        match sys.register(cap) {
            Ok(()) => n += 1,
            Err(e) => warn!(capability = %name, error = %e, "failed to register builtin capability"),
        }
    }
    info!(count = n, "registered builtin tool capabilities");
}

#[allow(dead_code)]
pub(crate) async fn build_runtime_without_router(
    config: RuntimeConfig,
) -> Result<Runtime, apxm_core::error::RuntimeError> {
    let mut runtime = Runtime::new(config).await?;
    runtime.set_sandbox_registry(configure_sandbox_registry());
    Ok(runtime)
}

pub(crate) async fn load_llm_backends(runtime: &Runtime) {
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
