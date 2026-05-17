use apxm_backends::BackendRegistration;
use apxm_driver::runtime::sandbox::configure_sandbox_registry;
use apxm_runtime::{ModelRouterConfig, Runtime, RuntimeConfig};
use tracing::{info, warn};

pub(crate) async fn build_runtime_with_router(
    config: RuntimeConfig,
) -> Result<Runtime, apxm_core::error::RuntimeError> {
    let mut runtime = Runtime::new(config).await?;
    runtime.set_sandbox_registry(configure_sandbox_registry());
    load_llm_backends(&runtime).await;
    runtime.init_model_router(ModelRouterConfig::default())?;
    Ok(runtime)
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
