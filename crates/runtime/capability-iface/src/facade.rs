//! Executor's actual usage surface of `CapabilitySystem`.
//!
//! This is *not* a mirror of `CapabilitySystem`'s full public API (which also
//! has registration, interceptor management, AAM wiring, approval-store
//! access, ...) — it is exactly the methods `apxm-runtime`'s executor module
//! calls on `ExecutionContext.capability_system` from outside the capability
//! module itself (verified against the real call sites in `context.rs`,
//! `handlers/inv_cap.rs`, `handlers/llm/tool_dispatch.rs`, and
//! `executor/hook_driver.rs`).
//!
//! [`ApprovalContext`] mirrors capability's internal `PreInvokeContext` minus
//! its `registry: &CapabilityRegistry` field: the only thing that field was
//! used for (checking one capability's `requires_approval` metadata) is
//! something the concrete `invoke_with_timeout_ctx` implementation can — and,
//! after this seam, does — resolve itself from its own registry, so the
//! caller (executor) no longer needs a `CapabilityRegistry` reference at all.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use apxm_core::error::RuntimeError;
use apxm_core::types::consent::ConsentBroker;
use apxm_core::types::values::Value;
use async_trait::async_trait;

use crate::events::ExecutionEventEmitter;
use crate::metadata::RuntimeCapability;
use crate::sandbox::{IsolationLevel, SandboxRegistry};

/// Result of checking whether a capability invocation can be sandboxed —
/// moved here (from `apxm-runtime`'s `capability` module) because it's the
/// return type of [`CapabilityFacade::sandbox_preflight`], the write-boundary
/// admission check every direct (non-read-only) tool call goes through
/// (`executor::handlers::inv_cap::enforce_write_boundary`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySandboxPreflight {
    /// The capability does not request sandbox routing for these arguments.
    Direct,
    /// The capability produced an execution request and a compatible backend
    /// was selected without running the capability.
    Sandboxed {
        /// Selected backend name.
        backend: String,
        /// Isolation level provided by the selected backend.
        isolation: IsolationLevel,
        /// Warnings reported when the backend can only provide degraded guarantees.
        warnings: Vec<String>,
    },
}

/// Approval-gate context for [`CapabilityFacade::invoke_with_timeout_ctx`].
///
/// Built by the caller (executor) from its own request-scoped state; the
/// facade implementation is responsible for consulting its own capability
/// registry to decide whether the named capability actually requires
/// approval.
pub struct ApprovalContext<'a> {
    pub consent_broker: &'a dyn ConsentBroker,
    pub event_emitter: Option<&'a dyn ExecutionEventEmitter>,
    pub host_id: Option<&'a str>,
    pub agent_code: Option<&'a str>,
    pub grant_id: Option<&'a str>,
    pub permission_timeout: Duration,
}

/// Executor-facing capability dispatch surface.
///
/// `ExecutionContext.capability_system` is `Arc<dyn CapabilityFacade>`;
/// `CapabilitySystem` (in `apxm-runtime`'s `capability` module) implements
/// this trait. Both sides of the executor↔capability touchpoint now depend
/// on this crate for the contract instead of on each other's concrete types.
#[async_trait]
pub trait CapabilityFacade: Send + Sync {
    /// Invoke a capability by name with validation, no approval gate.
    async fn invoke(
        &self,
        name: &str,
        args: HashMap<String, Value>,
    ) -> Result<Value, RuntimeError>;

    /// Invoke a capability with an explicit timeout and optional
    /// approval-gate context.
    async fn invoke_with_timeout_ctx(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        timeout: Duration,
        approval: Option<ApprovalContext<'_>>,
    ) -> Result<Value, RuntimeError>;

    /// Whether a capability with this name is registered.
    fn has_capability(&self, name: &str) -> bool;

    /// Whether the named capability is safe for full parallel execution.
    fn is_read_only(&self, name: &str) -> bool;

    /// Look up one capability's metadata.
    fn get_metadata(&self, name: &str) -> Option<RuntimeCapability>;

    /// List all registered capabilities.
    fn list_capabilities(&self) -> Vec<RuntimeCapability>;

    /// List capabilities whose declared groups intersect the requested set.
    fn list_capabilities_by_groups(&self, groups: &[String]) -> Vec<RuntimeCapability>;

    /// Set the sandbox registry used to route capability execution through
    /// sandbox backends.
    fn set_sandbox_registry(&self, registry: Arc<SandboxRegistry>);

    /// Check whether a capability invocation would route through a
    /// compatible sandbox backend, without executing the capability.
    ///
    /// Load-bearing for the write-boundary admission check every direct
    /// (non-read-only) tool call goes through: a capability that would run
    /// sandboxed is treated as admitted without needing an explicit grant.
    fn sandbox_preflight(
        &self,
        name: &str,
        args: &HashMap<String, Value>,
    ) -> Result<CapabilitySandboxPreflight, RuntimeError>;
}
