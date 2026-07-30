//! Capability system for tool/function invocation
//!
//! The capability system provides a plugin architecture for external tools and functions.
//! Capabilities can be registered, validated, and invoked through a unified interface.
//!
//! # Architecture
//!
//! - **CapabilityExecutor**: Trait for capability implementations
//! - **RuntimeCapability**: Schema and metadata for capabilities
//! - **CapabilityRegistry**: Thread-safe storage and lookup
//! - **CapabilitySystem**: Coordinator for invocation with validation
//!
//! # Example
//!
//! ```rust
//! use apxm_capability::{CapabilitySystem, executor::EchoCapability};
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
pub mod builtins;
pub mod executor;
pub mod flow_registry;
pub mod interceptor;
pub mod metadata;
pub mod registry;
pub mod tool_write_lock;

use approval::ApprovalStore;
use apxm_capability_iface::sandbox::{SandboxRegistry, ValidationResult};
use apxm_capability_iface::{ApprovalContext, CapabilityFacade, CapabilityInvocation};
use apxm_core::{error::RuntimeError, types::values::Value};
use executor::{CapabilityExecutionResult, CapabilityExecutor, exec_result_to_value};
use interceptor::{
    CapabilityInterceptor, InterceptDecision, PreInvokeContext, pre_invoke_policy_ctx,
};
use metadata::RuntimeCapability;
use parking_lot::RwLock;
use registry::CapabilityRegistry;
use std::{collections::HashMap, sync::Arc, time::Duration};

const SANDBOX_DEGRADED_GUARANTEES: &str = "sandbox backend selected with degraded guarantees";

/// Result type for capability operations
type CapabilityResult<T> = Result<T, RuntimeError>;

// `CapabilitySandboxPreflight` moved to `apxm-capability-iface` — it's the
// return type of `CapabilityFacade::sandbox_preflight`. Re-exported so
// `crate::CapabilitySandboxPreflight` keeps working.
pub use apxm_capability_iface::CapabilitySandboxPreflight;

/// Main capability system coordinator
///
/// Provides high-level API for capability invocation with:
/// - Automatic input validation against JSON schemas
/// - Timeout enforcement
/// - Error handling and logging
pub struct CapabilitySystem {
    registry: Arc<CapabilityRegistry>,
    default_timeout: Duration,
    interceptors: Arc<RwLock<Vec<Arc<dyn CapabilityInterceptor>>>>,
    approval_store: Arc<ApprovalStore>,
    sandbox_registry: Arc<RwLock<Option<Arc<SandboxRegistry>>>>,
}

impl CapabilitySystem {
    /// Create a new capability system
    pub fn new() -> Self {
        Self {
            registry: Arc::new(CapabilityRegistry::new()),
            default_timeout: Duration::from_secs(30),
            interceptors: Arc::new(RwLock::new(Vec::new())),
            approval_store: Arc::new(ApprovalStore::new()),
            sandbox_registry: Arc::new(RwLock::new(None)),
        }
    }

    /// Create with custom default timeout
    pub fn with_timeout(timeout: Duration) -> Self {
        let mut sys = Self::new();
        sys.default_timeout = timeout;
        sys
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
        self.registry.register(Arc::clone(&capability))?;
        Ok(())
    }

    /// Register a capability, replacing any existing one of the same name.
    /// Hosts can re-register a context-scoped tool surface on every
    /// Invocation without a duplicate error.
    pub fn register_or_replace(
        &self,
        capability: Arc<dyn CapabilityExecutor>,
    ) -> CapabilityResult<()> {
        self.registry.register_or_replace(Arc::clone(&capability))?;
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
        self.invoke_with_timeout_ctx_raw(name, args, timeout, None, None)
            .await
    }

    /// Apply cached approvals, execution-scoped consent, and registered
    /// interceptors before a capability or artifact-local worker receives its
    /// arguments. Calls that require approval but have no consent context fail
    /// closed rather than reaching the execution path.
    async fn admit_invocation_ctx_raw(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        requires_approval: bool,
        pre_ctx: Option<&PreInvokeContext<'_>>,
    ) -> CapabilityResult<HashMap<String, Value>> {
        if requires_approval && pre_ctx.is_none() {
            return Err(RuntimeError::Capability {
                capability: name.to_string(),
                message: format!(
                    "capability '{name}' requires approval but no consent context was provided"
                ),
            });
        }

        let mut args = args;
        if let Some(cached) = self.approval_store.check(name) {
            match cached {
                InterceptDecision::Allow => {}
                InterceptDecision::Deny { reason } => {
                    return Err(RuntimeError::Capability {
                        capability: name.to_string(),
                        message: reason,
                    });
                }
                InterceptDecision::EditArgs { args: edited } => args = edited,
            }
        }

        if let Some(ctx) = pre_ctx {
            match pre_invoke_policy_ctx(ctx, name, &args, requires_approval).await {
                InterceptDecision::Allow => {}
                InterceptDecision::Deny { reason } => {
                    return Err(RuntimeError::Capability {
                        capability: name.to_string(),
                        message: reason,
                    });
                }
                InterceptDecision::EditArgs { args: edited } => args = edited,
            }
        }

        let interceptors = self.interceptors.read().clone();
        for interceptor in &interceptors {
            match interceptor.pre_invoke(name, &args).await {
                InterceptDecision::Allow => {}
                InterceptDecision::Deny { reason } => {
                    return Err(RuntimeError::Capability {
                        capability: name.to_string(),
                        message: reason,
                    });
                }
                InterceptDecision::EditArgs { args: edited } => args = edited,
            }
        }

        Ok(args)
    }

    /// Invoke capability with custom timeout and optional approval-gate context.
    ///
    /// `pub(crate)` — the `CapabilityFacade::invoke_with_timeout_ctx` trait
    /// method (below) is the executor-facing entry point. It takes an
    /// `ApprovalContext` (no `&CapabilityRegistry` field) and builds the full
    /// `PreInvokeContext` this method needs itself, from `self.registry`, so
    /// callers outside this module never need a registry reference.
    pub(crate) async fn invoke_with_timeout_ctx_raw(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        timeout: Duration,
        pre_ctx: Option<&PreInvokeContext<'_>>,
        invocation: Option<&CapabilityInvocation>,
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

        let args = self
            .admit_invocation_ctx_raw(name, args, capability.metadata().requires_approval, pre_ctx)
            .await?;
        if let Some(invocation) = invocation {
            invocation
                .validate_for(name, &args)
                .map_err(|error| RuntimeError::Capability {
                    capability: name.to_string(),
                    message: format!("invalid canonical Capability invocation: {error}"),
                })?;
        }
        let interceptors = self.interceptors.read().clone();

        // Validate arguments against schema
        self.validate_args(name, &args)?;

        tracing::debug!(capability = %name, "Invoking capability");

        // Execute with timeout — route through sandbox if the capability
        // provides an ExecRequest, otherwise execute directly.
        let sandbox_reg = self.sandbox_registry.read().clone();
        let execution = tokio::time::timeout(timeout, async {
            // Check if capability wants sandbox execution
            if let Some(exec_req) = capability.to_exec_request(&args) {
                // Route through sandbox backend
                let Some(registry) = sandbox_reg else {
                    // Capability requires sandbox but no registry configured
                    return Err(RuntimeError::Capability {
                        capability: name.to_string(),
                        message: "Capability requires sandbox execution but sandbox registry is not configured".to_string(),
                    });
                };
                if registry.is_empty() {
                    // Capability requires sandbox but registry is empty
                    return Err(RuntimeError::Capability {
                        capability: name.to_string(),
                        message: "Capability requires sandbox execution but no sandbox backend is available".to_string(),
                    });
                }

                let selection = registry
                    .select_for_request(&exec_req)
                    .map_err(|e| RuntimeError::Capability {
                        capability: name.to_string(),
                        message: format!("sandbox select: {e}"),
                    })?;
                let backend = selection.backend;
                if let ValidationResult::Degraded { warnings } = &selection.validation {
                    tracing::warn!(
                        capability = %name,
                        backend = %backend.capabilities().name,
                        warnings = ?warnings,
                        "sandbox backend selected with degraded guarantees"
                    );
                    return Err(RuntimeError::Capability {
                        capability: name.to_string(),
                        message: format!(
                            "{SANDBOX_DEGRADED_GUARANTEES}: {}",
                            warnings.join("; ")
                        ),
                    });
                }
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
                let exec_result = match backend.execute(&session, exec_req).await {
                    Ok(result) => result,
                    Err(error) => {
                        let _ = backend.destroy_session(session).await;
                        return Err(RuntimeError::Capability {
                            capability: name.to_string(),
                            message: format!("sandbox execute: {error}"),
                        });
                    }
                };
                let _ = backend.destroy_session(session).await;
                return Ok(CapabilityExecutionResult::new(exec_result_to_value(exec_result)));
            }

            // Capability doesn't need sandbox, execute directly.
            match invocation {
                Some(invocation) => {
                    capability.execute_with_effect_receipt(args, invocation).await
                }
                None => capability.execute(args).await.map(CapabilityExecutionResult::new),
            }
        })
            .await
            .map_err(|_| RuntimeError::Timeout { op_id: 0, timeout })?
            .map_err(|e| {
                tracing::error!(capability = %name, error = %e, "Capability execution failed");
                e
            })?;

        if let Some(receipt) = execution.effect_receipt.as_ref() {
            receipt
                .validate()
                .map_err(|message| RuntimeError::Capability {
                    capability: name.to_string(),
                    message: format!("invalid durable capability effect receipt: {message}"),
                })?;
            let invocation = invocation.ok_or_else(|| RuntimeError::Capability {
                capability: name.to_string(),
                message: "durable capability effect receipt is missing trusted invocation identity"
                    .to_string(),
            })?;
            if receipt.execution_id != invocation.execution_id()
                || receipt.node_id != invocation.node_id()
                || receipt.invocation_id
                    != invocation
                        .canonical_request()
                        .correlation()
                        .program_invocation_ref
                        .target
                || receipt.request_digest
                    != invocation.canonical_request().request_digest_sha256_hex()
            {
                return Err(RuntimeError::Capability {
                    capability: name.to_string(),
                    message:
                        "durable capability effect receipt does not bind the active invocation"
                            .to_string(),
                });
            }
            if let Some(emitter) = pre_ctx.and_then(|context| context.event_emitter) {
                emitter.emit_capability_effect_receipt(receipt);
            }
        }

        let result = execution.value;

        // Post-invoke hooks (best effort)
        for interceptor in &interceptors {
            interceptor.post_invoke(name, &result).await;
        }

        tracing::info!(capability = %name, "Capability executed successfully");
        Ok(result)
    }

    /// Validate arguments against capability schema
    fn validate_args(&self, name: &str, args: &HashMap<String, Value>) -> CapabilityResult<()> {
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
    pub fn list_capabilities(&self) -> Vec<RuntimeCapability> {
        self.registry.list_metadata()
    }

    /// List capabilities whose declared groups intersect the requested set.
    pub fn list_capabilities_by_groups(&self, groups: &[String]) -> Vec<RuntimeCapability> {
        if groups.is_empty() {
            return Vec::new();
        }

        self.registry
            .list_metadata()
            .into_iter()
            .filter(|meta| meta.groups.iter().any(|group| groups.contains(group)))
            .collect()
    }

    /// Check if capability exists
    pub fn has_capability(&self, name: &str) -> bool {
        self.registry.contains(name)
    }

    /// Get capability metadata
    pub fn get_metadata(&self, name: &str) -> Option<RuntimeCapability> {
        self.registry.get(name).map(|cap| cap.metadata().clone())
    }

    /// Returns `true` if the named capability is marked read-only.
    pub fn is_read_only(&self, name: &str) -> bool {
        self.registry
            .get(name)
            .is_some_and(|cap| cap.metadata().read_only)
    }

    /// Check whether a capability invocation would route through a compatible
    /// sandbox backend, without executing the capability.
    pub fn sandbox_preflight(
        &self,
        name: &str,
        args: &HashMap<String, Value>,
    ) -> CapabilityResult<CapabilitySandboxPreflight> {
        let capability = self
            .registry
            .get(name)
            .ok_or_else(|| RuntimeError::Capability {
                capability: name.to_string(),
                message: format!("Capability '{name}' not found"),
            })?;

        let Some(exec_req) = capability.to_exec_request(args) else {
            return Ok(CapabilitySandboxPreflight::Direct);
        };

        let sandbox_reg = self.sandbox_registry.read().clone();
        let registry = sandbox_reg.ok_or_else(|| RuntimeError::Capability {
            capability: name.to_string(),
            message: "Capability requires sandbox execution but sandbox registry is not configured"
                .to_string(),
        })?;
        if registry.is_empty() {
            return Err(RuntimeError::Capability {
                capability: name.to_string(),
                message:
                    "Capability requires sandbox execution but no sandbox backend is available"
                        .to_string(),
            });
        }

        let selection =
            registry
                .select_for_request(&exec_req)
                .map_err(|error| RuntimeError::Capability {
                    capability: name.to_string(),
                    message: format!("sandbox select: {error}"),
                })?;
        let capabilities = selection.backend.capabilities();
        let warnings = match selection.validation {
            ValidationResult::Ok => Vec::new(),
            ValidationResult::Degraded { warnings } => {
                return Err(RuntimeError::Capability {
                    capability: name.to_string(),
                    message: format!("{SANDBOX_DEGRADED_GUARANTEES}: {}", warnings.join("; ")),
                });
            }
            // `select_for_request` never returns an Unsupported selection (it is
            // filtered into the rejected set), but fail closed if that ever
            // changes rather than reporting a confined run that isn't.
            ValidationResult::Unsupported { reason } => {
                return Err(RuntimeError::Capability {
                    capability: name.to_string(),
                    message: format!("sandbox unsupported: {reason}"),
                });
            }
        };
        Ok(CapabilitySandboxPreflight::Sandboxed {
            backend: capabilities.name,
            isolation: capabilities.isolation_level,
            warnings,
        })
    }
}

impl Default for CapabilitySystem {
    fn default() -> Self {
        Self::new()
    }
}

/// Executor's actual `CapabilitySystem` usage surface, expressed as a trait
/// so `ExecutionContext.capability_system` can be `Arc<dyn CapabilityFacade>`
/// instead of the concrete type — the fourth and final step in breaking
/// apxm-runtime's capability/executor/scheduler use-graph cycle (see
/// `apxm-capability-iface`'s crate docs).
#[async_trait::async_trait]
impl CapabilityFacade for CapabilitySystem {
    async fn admit_with_ctx(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        requires_approval: bool,
        approval: ApprovalContext<'_>,
    ) -> CapabilityResult<HashMap<String, Value>> {
        let pre_ctx = PreInvokeContext {
            registry: &self.registry,
            call_id: approval.call_id,
            tool_call_correlation: approval.tool_call_correlation,
            consent_broker: approval.consent_broker,
            event_emitter: approval.event_emitter,
            host_id: approval.host_id,
            agent_code: approval.agent_code,
            grant_id: approval.grant_id,
            permission_timeout: approval.permission_timeout,
        };
        self.admit_invocation_ctx_raw(name, args, requires_approval, Some(&pre_ctx))
            .await
    }

    async fn invoke_with_timeout_ctx(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        timeout: Duration,
        approval: ApprovalContext<'_>,
    ) -> CapabilityResult<Value> {
        let pre_ctx = PreInvokeContext {
            registry: &self.registry,
            call_id: approval.call_id,
            tool_call_correlation: approval.tool_call_correlation,
            consent_broker: approval.consent_broker,
            event_emitter: approval.event_emitter,
            host_id: approval.host_id,
            agent_code: approval.agent_code,
            grant_id: approval.grant_id,
            permission_timeout: approval.permission_timeout,
        };
        self.invoke_with_timeout_ctx_raw(name, args, timeout, Some(&pre_ctx), None)
            .await
    }

    async fn invoke_with_timeout_ctx_and_invocation(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        timeout: Duration,
        approval: ApprovalContext<'_>,
        invocation: &CapabilityInvocation,
    ) -> CapabilityResult<Value> {
        let pre_ctx = PreInvokeContext {
            registry: &self.registry,
            call_id: approval.call_id,
            tool_call_correlation: approval.tool_call_correlation,
            consent_broker: approval.consent_broker,
            event_emitter: approval.event_emitter,
            host_id: approval.host_id,
            agent_code: approval.agent_code,
            grant_id: approval.grant_id,
            permission_timeout: approval.permission_timeout,
        };
        self.invoke_with_timeout_ctx_raw(name, args, timeout, Some(&pre_ctx), Some(invocation))
            .await
    }

    fn has_capability(&self, name: &str) -> bool {
        CapabilitySystem::has_capability(self, name)
    }

    fn is_read_only(&self, name: &str) -> bool {
        CapabilitySystem::is_read_only(self, name)
    }

    fn get_metadata(&self, name: &str) -> Option<RuntimeCapability> {
        CapabilitySystem::get_metadata(self, name)
    }

    fn list_capabilities(&self) -> Vec<RuntimeCapability> {
        CapabilitySystem::list_capabilities(self)
    }

    fn list_capabilities_by_groups(&self, groups: &[String]) -> Vec<RuntimeCapability> {
        CapabilitySystem::list_capabilities_by_groups(self, groups)
    }

    fn set_sandbox_registry(&self, registry: Arc<SandboxRegistry>) {
        CapabilitySystem::set_sandbox_registry(self, registry);
    }

    fn sandbox_preflight(
        &self,
        name: &str,
        args: &HashMap<String, Value>,
    ) -> CapabilityResult<CapabilitySandboxPreflight> {
        CapabilitySystem::sandbox_preflight(self, name, args)
    }
}
