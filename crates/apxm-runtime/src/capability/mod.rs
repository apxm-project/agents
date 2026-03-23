//! Capability system for tool/function invocation
//!
//! The capability system provides a plugin architecture for external tools and functions.
//! Capabilities can be registered, validated, and invoked through a unified interface.
//!
//! # Architecture
//!
//! - **CapabilityExecutor**: Trait for capability implementations
//! - **CapabilityMetadata**: Schema and metadata for capabilities
//! - **CapabilityRegistry**: Thread-safe storage and lookup
//! - **CapabilitySystem**: Coordinator for invocation with validation
//!
//! # Example
//!
//! ```rust
//! use apxm_runtime::capability::{CapabilitySystem, executor::EchoCapability};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let system = CapabilitySystem::new();
//!
//! // Register built-in capability
//! system.register(Arc::new(EchoCapability::new()))?;
//!
//! // Invoke capability
//! let mut args = std::collections::HashMap::new();
//! args.insert("message".to_string(), apxm_core::types::values::Value::String("Hello".to_string()));
//! let result = system.invoke("echo", args).await?;
//! # Ok(())
//! # }
//! ```

pub mod approval;
pub mod executor;
pub mod flow_registry;
pub mod interceptor;
pub mod metadata;
pub mod registry;

use crate::aam::{Aam, TransitionLabel};
use approval::{ApprovalChannel, ApprovalStore};
use apxm_core::{error::RuntimeError, types::values::Value};
use apxm_sandbox::SandboxRegistry;
use executor::{CapabilityExecutor, exec_result_to_value};
use interceptor::{CapabilityInterceptor, InterceptDecision};
use metadata::CapabilityMetadata;
use parking_lot::RwLock;
use registry::CapabilityRegistry;
use std::{collections::HashMap, sync::Arc, time::Duration};

/// Result type for capability operations
type CapabilityResult<T> = Result<T, RuntimeError>;

/// Main capability system coordinator
///
/// Provides high-level API for capability invocation with:
/// - Automatic input validation against JSON schemas
/// - Timeout enforcement
/// - Error handling and logging
pub struct CapabilitySystem {
    registry: Arc<CapabilityRegistry>,
    default_timeout: Duration,
    aam: Option<Aam>,
    interceptors: Arc<RwLock<Vec<Arc<dyn CapabilityInterceptor>>>>,
    approval_store: Arc<ApprovalStore>,
    approval_channel: Option<Arc<dyn ApprovalChannel>>,
    sandbox_registry: RwLock<Option<Arc<SandboxRegistry>>>,
}

impl CapabilitySystem {
    /// Create a new capability system
    pub fn new() -> Self {
        Self {
            registry: Arc::new(CapabilityRegistry::new()),
            default_timeout: Duration::from_secs(30),
            aam: None,
            interceptors: Arc::new(RwLock::new(Vec::new())),
            approval_store: Arc::new(ApprovalStore::new()),
            approval_channel: None,
            sandbox_registry: RwLock::new(None),
        }
    }

    /// Create with custom default timeout
    pub fn with_timeout(timeout: Duration) -> Self {
        let mut sys = Self::new();
        sys.default_timeout = timeout;
        sys
    }

    /// Create with AAM integration (default timeout)
    pub fn with_aam(aam: Aam) -> Self {
        let mut sys = Self::new();
        sys.aam = Some(aam);
        sys
    }

    /// Set an approval channel for interactive user permission requests.
    ///
    /// When set, denied capabilities are routed through this channel
    /// instead of being immediately rejected.
    pub fn set_approval_channel(&mut self, channel: Arc<dyn ApprovalChannel>) {
        self.approval_channel = Some(channel);
    }

    /// Create with an approval channel attached (builder-style).
    pub fn with_approval_channel(mut self, channel: Arc<dyn ApprovalChannel>) -> Self {
        self.approval_channel = Some(channel);
        self
    }

    /// Get a reference to the approval store.
    pub fn approval_store(&self) -> &ApprovalStore {
        &self.approval_store
    }

    /// Set the sandbox registry for routing capability execution through
    /// sandbox backends.
    ///
    /// When set, capabilities that return an [`ExecRequest`] from
    /// [`to_exec_request()`](executor::CapabilityExecutor::to_exec_request)
    /// will have their execution routed through the sandbox backend
    /// instead of calling `execute()` directly.
    pub fn set_sandbox_registry(&self, registry: Arc<SandboxRegistry>) {
        *self.sandbox_registry.write() = Some(registry);
    }

    /// Register a capability interceptor.
    pub fn register_interceptor(&self, interceptor: Arc<dyn CapabilityInterceptor>) {
        let name = interceptor.name().to_string();
        let mut guard = self.interceptors.write();
        if guard.iter().any(|it| it.name() == name) {
            return;
        }
        guard.push(interceptor);
    }

    /// Check whether an interceptor is already registered.
    pub fn has_interceptor(&self, name: &str) -> bool {
        self.interceptors.read().iter().any(|it| it.name() == name)
    }

    /// Get read-only access to the capability registry
    ///
    /// This allows inspection of registered capabilities without
    /// modification access.
    pub fn registry(&self) -> &CapabilityRegistry {
        &self.registry
    }

    /// List all registered capability names
    ///
    /// This is useful for:
    /// - Validating graph intent before compilation
    /// - Showing available capabilities to users
    /// - Passing to LLM for constrained generation
    pub fn list_capability_names(&self) -> Vec<String> {
        self.registry.list_names()
    }

    /// Register a capability
    ///
    /// # Arguments
    ///
    /// * `capability` - Capability implementation to register
    ///
    /// # Errors
    ///
    /// Returns error if capability is already registered or has invalid schema
    pub fn register(&self, capability: Arc<dyn CapabilityExecutor>) -> CapabilityResult<()> {
        let metadata = capability.metadata().clone();
        self.registry.register(Arc::clone(&capability))?;

        if let Some(aam) = &self.aam {
            let label = TransitionLabel::custom(format!("register_capability:{}", metadata.name));
            aam.register_capability(metadata.name.clone(), (&metadata).into(), label);
        }

        Ok(())
    }

    /// Invoke a capability by name with validation
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the capability to invoke
    /// * `args` - Input arguments as key-value pairs
    ///
    /// # Returns
    ///
    /// Result value from capability execution
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Capability not found
    /// - Input validation fails
    /// - Execution times out
    /// - Capability execution fails
    pub async fn invoke(
        &self,
        name: &str,
        args: HashMap<String, Value>,
    ) -> CapabilityResult<Value> {
        self.invoke_with_timeout(name, args, self.default_timeout)
            .await
    }

    /// Invoke capability with custom timeout
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the capability to invoke
    /// * `args` - Input arguments
    /// * `timeout` - Maximum execution time
    pub async fn invoke_with_timeout(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        timeout: Duration,
    ) -> CapabilityResult<Value> {
        // Get capability
        let capability = self
            .registry
            .get(name)
            .ok_or_else(|| RuntimeError::Capability {
                capability: name.to_string(),
                message: format!(
                    "Capability '{}' not found. Available: {}",
                    name,
                    self.registry.list_names().join(", ")
                ),
            })?;

        // Check approval store for a cached decision first.
        let mut args = args;
        if let Some(cached) = self.approval_store.check(name) {
            match cached {
                InterceptDecision::Allow => { /* proceed */ }
                InterceptDecision::Deny { reason } => {
                    return Err(RuntimeError::Capability {
                        capability: name.to_string(),
                        message: reason,
                    });
                }
                InterceptDecision::EditArgs { args: edited } => {
                    args = edited;
                }
            }
        }

        // Apply pre-invoke interceptors.
        let interceptors = self.interceptors.read().clone();
        for interceptor in &interceptors {
            match interceptor.pre_invoke(name, &args).await {
                InterceptDecision::Allow => {}
                InterceptDecision::Deny { reason } => {
                    // If an approval channel exists, ask the user instead
                    // of immediately rejecting.
                    if let Some(channel) = &self.approval_channel {
                        let args_json =
                            serde_json::to_value(&args).unwrap_or(serde_json::Value::Null);
                        let (decision, scope) =
                            channel.request_approval(name, &args_json, &reason).await;
                        self.approval_store
                            .record(name.to_string(), decision.clone(), scope);
                        match decision {
                            InterceptDecision::Allow => { /* user overrode the deny */ }
                            InterceptDecision::Deny {
                                reason: user_reason,
                            } => {
                                return Err(RuntimeError::Capability {
                                    capability: name.to_string(),
                                    message: user_reason,
                                });
                            }
                            InterceptDecision::EditArgs { args: edited } => {
                                args = edited;
                            }
                        }
                    } else {
                        return Err(RuntimeError::Capability {
                            capability: name.to_string(),
                            message: reason,
                        });
                    }
                }
                InterceptDecision::EditArgs { args: edited } => {
                    args = edited;
                }
            }
        }

        // Validate arguments against schema
        self.validate_args(name, &args).await?;

        tracing::debug!(capability = %name, "Invoking capability");

        // Execute with timeout — route through sandbox if the capability
        // provides an ExecRequest, otherwise execute directly.
        let sandbox_reg = self.sandbox_registry.read().clone();
        let result = tokio::time::timeout(timeout, async {
            // Check if capability wants sandbox execution
            if let Some(exec_req) = capability.to_exec_request(&args) {
                // Route through sandbox backend
                if let Some(ref registry) = sandbox_reg {
                    if let Some(backend) = registry.default_backend() {
                        tracing::debug!(
                            capability = %name,
                            backend = %backend.capabilities().name,
                            "routing capability through sandbox backend"
                        );
                        let session = backend.create_session().await
                            .map_err(|e| RuntimeError::Capability {
                                capability: name.to_string(),
                                message: format!("sandbox session: {e}"),
                            })?;
                        let exec_result = backend.execute(&session, exec_req).await
                            .map_err(|e| RuntimeError::Capability {
                                capability: name.to_string(),
                                message: format!("sandbox execute: {e}"),
                            })?;
                        let _ = backend.destroy_session(session).await;
                        Ok(exec_result_to_value(exec_result))
                    } else {
                        // No available backend, fall back to direct execution
                        tracing::warn!(capability = %name, "no sandbox backend available, executing directly");
                        capability.execute(args).await
                    }
                } else {
                    // No sandbox registry configured, execute directly
                    capability.execute(args).await
                }
            } else {
                // Capability doesn't need sandbox, execute directly
                capability.execute(args).await
            }
        })
            .await
            .map_err(|_| RuntimeError::Timeout { op_id: 0, timeout })?
            .map_err(|e| {
                tracing::error!(capability = %name, error = %e, "Capability execution failed");
                e
            })?;

        // Post-invoke hooks (best effort)
        for interceptor in &interceptors {
            interceptor.post_invoke(name, &result).await;
        }

        tracing::info!(capability = %name, "Capability executed successfully");
        Ok(result)
    }

    /// Validate arguments against capability schema
    async fn validate_args(
        &self,
        name: &str,
        args: &HashMap<String, Value>,
    ) -> CapabilityResult<()> {
        let schema = self
            .registry
            .get_schema(name)
            .ok_or_else(|| RuntimeError::Capability {
                capability: name.to_string(),
                message: "Schema not found".to_string(),
            })?;

        // Convert args to JSON value
        let args_json = serde_json::to_value(args).map_err(|e| RuntimeError::Capability {
            capability: name.to_string(),
            message: format!("Failed to serialize arguments: {}", e),
        })?;

        // Validate against schema
        schema
            .validate(&args_json)
            .map_err(|e| RuntimeError::Capability {
                capability: name.to_string(),
                message: format!("Input validation failed: {}", e),
            })?;

        Ok(())
    }

    /// List all registered capabilities
    pub fn list_capabilities(&self) -> Vec<CapabilityMetadata> {
        self.registry.list_metadata()
    }

    /// Check if capability exists
    pub fn has_capability(&self, name: &str) -> bool {
        self.registry.contains(name)
    }

    /// Get capability metadata
    pub fn get_metadata(&self, name: &str) -> Option<CapabilityMetadata> {
        self.registry.get(name).map(|cap| cap.metadata().clone())
    }

    /// Returns `true` if the named capability is marked read-only.
    pub fn is_read_only(&self, name: &str) -> bool {
        self.registry
            .get(name)
            .map(|cap| cap.metadata().read_only)
            .unwrap_or(false)
    }
}

impl Default for CapabilitySystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use executor::EchoCapability;
    use interceptor::CapabilityInterceptor;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct DenyAll;

    #[async_trait::async_trait]
    impl CapabilityInterceptor for DenyAll {
        fn name(&self) -> &str {
            "deny_all"
        }

        async fn pre_invoke(
            &self,
            _name: &str,
            _args: &HashMap<String, Value>,
        ) -> InterceptDecision {
            InterceptDecision::Deny {
                reason: "blocked".to_string(),
            }
        }
    }

    struct ObservingPost {
        called: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl CapabilityInterceptor for ObservingPost {
        fn name(&self) -> &str {
            "observing_post"
        }

        async fn post_invoke(&self, _name: &str, _result: &Value) {
            self.called.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn test_capability_system_creation() {
        let system = CapabilitySystem::new();
        assert_eq!(system.list_capabilities().len(), 0);
    }

    #[tokio::test]
    async fn test_register_and_invoke() {
        let system = CapabilitySystem::new();
        assert!(system.register(Arc::new(EchoCapability::new())).is_ok());

        let mut args = HashMap::new();
        args.insert("message".to_string(), Value::String("Test".to_string()));

        let result = system.invoke("echo", args).await;
        match result {
            Ok(val) => assert_eq!(val.as_string().map(|s| s.as_str()), Some("Echo: Test")),
            Err(e) => panic!("invoke failed: {}", e),
        }
    }

    #[tokio::test]
    async fn test_invoke_nonexistent_capability() {
        let system = CapabilitySystem::new();

        let result = system.invoke("nonexistent", HashMap::new()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validation_failure() {
        let system = CapabilitySystem::new();
        assert!(system.register(Arc::new(EchoCapability::new())).is_ok());

        // Missing required argument
        let result = system.invoke("echo", HashMap::new()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_has_capability() {
        let system = CapabilitySystem::new();
        assert!(system.register(Arc::new(EchoCapability::new())).is_ok());

        assert!(system.has_capability("echo"));
        assert!(!system.has_capability("nonexistent"));
    }

    #[tokio::test]
    async fn test_get_metadata() {
        let system = CapabilitySystem::new();
        assert!(system.register(Arc::new(EchoCapability::new())).is_ok());

        let metadata = match system.get_metadata("echo") {
            Some(m) => m,
            None => panic!("metadata missing for 'echo'"),
        };
        assert_eq!(metadata.name, "echo");
        assert!(metadata.description.to_lowercase().contains("echo"));
    }

    #[tokio::test]
    async fn test_register_records_in_aam() {
        let aam = Aam::new();
        let system = CapabilitySystem::with_aam(aam.clone());
        assert!(system.register(Arc::new(EchoCapability::new())).is_ok());
        let capabilities = aam.capabilities();
        assert!(capabilities.contains_key("echo"));
    }

    #[tokio::test]
    async fn test_interceptor_can_deny() {
        let system = CapabilitySystem::new();
        assert!(system.register(Arc::new(EchoCapability::new())).is_ok());
        system.register_interceptor(Arc::new(DenyAll));

        let mut args = HashMap::new();
        args.insert("message".to_string(), Value::String("x".to_string()));
        let err = system.invoke("echo", args).await.unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[tokio::test]
    async fn test_interceptor_post_invoke_called() {
        let system = CapabilitySystem::new();
        assert!(system.register(Arc::new(EchoCapability::new())).is_ok());
        let called = Arc::new(AtomicBool::new(false));
        system.register_interceptor(Arc::new(ObservingPost {
            called: Arc::clone(&called),
        }));

        let mut args = HashMap::new();
        args.insert("message".to_string(), Value::String("x".to_string()));
        let _ = system
            .invoke("echo", args)
            .await
            .expect("invoke should succeed");
        assert!(called.load(Ordering::SeqCst));
    }
}
