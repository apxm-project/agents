//! TypeScript tool worker bridge — Node subprocess NDJSON worker.

pub mod constants;
pub mod protocol;
pub mod registry;
pub mod worker;

pub use constants::{
    CAPABILITY_NAME, MANIFEST_TEMPFILE_PREFIX, MANIFEST_TEMPFILE_SUFFIX, NODE_BIN, TRACE_TARGET,
    WORKER_SCRIPT,
};
pub use protocol::{
    CallRequest, CallResponse, ErrorEnvelope, PROTOCOL_VERSION, WorkerRequest, WorkerResponse,
};
pub use registry::{ToolDescriptor, TypeScriptHandlerRegistry};
pub use worker::TypeScriptHandlerWorker;

use apxm_core::error::RuntimeError;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

pub struct TypeScriptHandlerBridge {
    registry: TypeScriptHandlerRegistry,
    worker: OnceCell<Arc<TypeScriptHandlerWorker>>,
    sandbox: Option<Arc<dyn crate::sandbox::SandboxBackend>>,
    sandbox_required: bool,
}

impl TypeScriptHandlerBridge {
    pub fn new(registry: TypeScriptHandlerRegistry) -> Self {
        Self {
            registry,
            worker: OnceCell::new(),
            sandbox: None,
            sandbox_required: false,
        }
    }

    pub fn with_sandbox(
        mut self,
        sandbox: Option<Arc<dyn crate::sandbox::SandboxBackend>>,
        required: bool,
    ) -> Self {
        self.sandbox = sandbox;
        self.sandbox_required = required;
        self
    }

    pub fn from_tools_json(path: &std::path::Path) -> Result<Self, RuntimeError> {
        let registry = TypeScriptHandlerRegistry::from_file(path)?;
        Ok(Self::new(registry))
    }

    pub fn has_tool(&self, capability_name: &str) -> bool {
        self.registry.contains(capability_name)
    }

    pub fn has_handler(&self, handler_id: &str) -> bool {
        self.registry.resolve_handler_id(handler_id).is_some()
    }

    pub async fn call(
        &self,
        capability_name: &str,
        args: serde_json::Value,
        deadline: Duration,
    ) -> Result<serde_json::Value, RuntimeError> {
        self.call_with_call_id(capability_name, args, deadline, None)
            .await
    }

    pub async fn call_with_call_id(
        &self,
        capability_name: &str,
        args: serde_json::Value,
        deadline: Duration,
        call_id: Option<&str>,
    ) -> Result<serde_json::Value, RuntimeError> {
        let descriptor =
            self.registry
                .resolve(capability_name)
                .ok_or_else(|| RuntimeError::Capability {
                    capability: capability_name.into(),
                    message: format!(
                        "No TypeScript tool registered for capability '{}'",
                        capability_name
                    ),
                })?;

        let handler_id = descriptor.handler_id.clone();
        let worker = self.worker().await?;
        worker
            .call_with_call_id(&handler_id, args, deadline, call_id)
            .await
    }

    pub async fn call_hook(
        &self,
        handler_id: &str,
        payload: serde_json::Value,
        deadline: Duration,
    ) -> Result<serde_json::Value, RuntimeError> {
        let worker = self.worker().await?;
        worker.call(handler_id, payload, deadline).await
    }

    pub async fn call_hook_with_host<F, Fut>(
        &self,
        handler_id: &str,
        payload: serde_json::Value,
        deadline: Duration,
        host: F,
    ) -> Result<serde_json::Value, RuntimeError>
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = Result<serde_json::Value, String>>,
    {
        let worker = self.worker().await?;
        worker
            .call_with_host(handler_id, payload, deadline, host)
            .await
    }

    pub fn registry(&self) -> &TypeScriptHandlerRegistry {
        &self.registry
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &ToolDescriptor> {
        self.registry.descriptors()
    }

    async fn worker(&self) -> Result<Arc<TypeScriptHandlerWorker>, RuntimeError> {
        self.worker
            .get_or_try_init(|| async {
                let manifest = self.registry.manifest_json()?;
                let w = TypeScriptHandlerWorker::spawn_with_env(
                    &manifest,
                    &[],
                    self.sandbox.as_ref(),
                    self.sandbox_required,
                )
                .await?;
                Ok::<_, RuntimeError>(Arc::new(w))
            })
            .await
            .map(Arc::clone)
    }
}
