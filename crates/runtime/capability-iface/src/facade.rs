//! Executor's actual usage surface of `CapabilitySystem`.
//!
//! This is not `CapabilitySystem`'s full public API (which also
//! has registration, interceptor management, AAM wiring, approval-store
//! access, ...) — it is exactly the methods `apxm-runtime`'s executor module
//! calls on `ExecutionContext.capability_system` from outside the capability
//! module itself.
//!
//! [`ApprovalContext`] carries capability pre-invoke fields minus its
//! `registry: &CapabilityRegistry` field: the concrete
//! `invoke_with_timeout_ctx` implementation resolves the named capability's
//! policy from its own registry, so the caller cannot omit canonical admission.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use apxm_core::error::RuntimeError;
use apxm_core::events::payload::{
    CapabilityEffectDispatchPath, CapabilityEffectReceiptPayload, ToolCallCorrelation,
};
use apxm_core::types::RuntimeCapabilityGrant;
use apxm_core::types::consent::ConsentBroker;
use apxm_core::types::host::{HostEffectCommit, HostEffectPrepare};
use apxm_core::types::values::Value;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::events::ExecutionEventEmitter;
use crate::metadata::RuntimeCapability;
use crate::sandbox::{IsolationLevel, SandboxRegistry};

/// Result of checking whether a capability invocation can be sandboxed.
///
/// Runtime admission uses this as confinement evidence for the write boundary;
/// the sandbox backend itself does not decide permission.
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
///
/// An approval-gated call is admitted only when the caller supplies both
/// `agent_code` (the acting agent) and `grant_id` (the grant the effect is
/// admitted under). The gate denies a gated call that carries neither; it
/// never substitutes a value for an absent identity field.
pub struct ApprovalContext<'a> {
    pub call_id: &'a str,
    pub tool_call_correlation: Option<&'a ToolCallCorrelation>,
    pub consent_broker: &'a dyn ConsentBroker,
    pub event_emitter: Option<&'a dyn ExecutionEventEmitter>,
    pub host_id: Option<&'a str>,
    pub agent_code: Option<&'a str>,
    pub grant_id: Option<&'a str>,
    pub permission_timeout: Duration,
}

/// Immutable identity for one capability dispatch performed by an execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInvocation {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: u64,
    pub invocation_id: String,
    pub dispatch_path: CapabilityEffectDispatchPath,
    /// Opaque identifiers retained in durable traces and capability receipts.
    pub grant_refs: Vec<String>,
    /// Trusted projections of the grants that admitted this invocation.
    ///
    /// Executors use this only to constrain resource visibility and output;
    /// callers cannot supply it through capability arguments.
    pub grant_contexts: Vec<RuntimeCapabilityGrant>,
}

/// Shared digest helper for committed capability-effect idempotency keys.
pub fn capability_effect_idempotency_key_digest(idempotency_key: &str) -> String {
    format!(
        "blake3:{}",
        blake3::hash(idempotency_key.as_bytes()).to_hex()
    )
}

/// Content-free preparation evidence made available to the replayer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEffectPrepareEvidence {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: u64,
    pub invocation_id: String,
    pub call_id: String,
    pub capability_id: String,
    pub host_op: String,
    pub capability_binding: String,
    pub implementation_ref: String,
    pub request_digest: String,
    pub idempotency_key: String,
    pub grant_refs: Vec<String>,
    pub approval_refs: Vec<String>,
}

impl From<&HostEffectPrepare> for HostEffectPrepareEvidence {
    fn from(prepare: &HostEffectPrepare) -> Self {
        Self {
            execution_id: prepare.execution_id.clone(),
            graph_id: prepare.graph_id.clone(),
            node_id: prepare.node_id,
            invocation_id: prepare.invocation_id.clone(),
            call_id: prepare.call_id.clone(),
            capability_id: prepare.capability_id.clone(),
            host_op: prepare.host_op.clone(),
            capability_binding: prepare.capability_binding.clone(),
            implementation_ref: prepare.implementation_ref.clone(),
            request_digest: prepare.request_digest.clone(),
            idempotency_key: prepare.idempotency_key.clone(),
            grant_refs: prepare.grant_refs.clone(),
            approval_refs: prepare.approval_refs.clone(),
        }
    }
}

/// Durable replay evidence for one committed capability effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEffectReplayEvidence {
    pub receipt: CapabilityEffectReceiptPayload,
    pub host_prepare: Option<HostEffectPrepareEvidence>,
    pub host_commit: Option<HostEffectCommit>,
    pub host_pubkey_hex: Option<String>,
}

/// Inline evidence envelope supplied by the durable receipt authority for a
/// partial replay attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEffectReplayEvidenceEnvelope {
    pub records: Vec<CapabilityEffectReplayEvidence>,
}

/// Executor-facing capability dispatch surface.
///
/// `ExecutionContext.capability_system` is `Arc<dyn CapabilityFacade>`;
/// `CapabilitySystem` (in `apxm-runtime`'s `capability` module) implements
/// this trait. Both sides of the executor↔capability touchpoint now depend
/// on this crate for the contract instead of on each other's concrete types.
#[async_trait]
pub trait CapabilityFacade: Send + Sync {
    /// Apply capability policy, consent, approval-store, and interceptor
    /// admission without executing a process-wide registered capability.
    /// Artifact-scoped script tools use this before dispatching to their worker.
    async fn admit_with_ctx(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        requires_approval: bool,
        approval: ApprovalContext<'_>,
    ) -> Result<HashMap<String, Value>, RuntimeError>;

    /// Invoke a capability with an explicit timeout and mandatory policy and
    /// consent admission context.
    async fn invoke_with_timeout_ctx(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        timeout: Duration,
        approval: ApprovalContext<'_>,
    ) -> Result<Value, RuntimeError>;

    /// Invoke a capability with the durable identity assigned by the executor.
    ///
    /// Implementors that do not produce durable capability-effect receipts can
    /// use the mandatory approval path unchanged. The concrete runtime facade
    /// overrides this to bind receipts to the supplied invocation.
    async fn invoke_with_timeout_ctx_and_invocation(
        &self,
        name: &str,
        args: HashMap<String, Value>,
        timeout: Duration,
        approval: ApprovalContext<'_>,
        invocation: Option<&CapabilityInvocation>,
    ) -> Result<Value, RuntimeError> {
        let _ = invocation;
        self.invoke_with_timeout_ctx(name, args, timeout, approval)
            .await
    }

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
    /// Load-bearing for the runtime write-boundary admission check every direct
    /// (non-read-only) tool call goes through. A capability that will run in a
    /// compatible sandbox can be admitted without an explicit grant because the
    /// runtime has verified confinement first; the sandbox backend is still not
    /// the policy decision engine.
    fn sandbox_preflight(
        &self,
        name: &str,
        args: &HashMap<String, Value>,
    ) -> Result<CapabilitySandboxPreflight, RuntimeError>;
}
