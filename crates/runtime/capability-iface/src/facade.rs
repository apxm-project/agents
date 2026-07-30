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
use apxm_core::types::host::{HostEffectCommit, HostEffectRequest};
use apxm_core::types::values::Value;
use apxm_program::CapabilityRequest;
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

/// Why a canonical Capability dispatch could not enter the executor boundary.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CapabilityInvocationError {
    #[error("Capability invocation field {field} is empty")]
    EmptyField { field: &'static str },
    #[error("Capability invocation request is invalid: {0}")]
    InvalidCanonicalRequest(String),
    #[error("Capability invocation request names {request}; executor selected {selected}")]
    CapabilityMismatch { request: String, selected: String },
    #[error("Capability invocation arguments differ from the canonical request identity")]
    ArgumentsMismatch,
    #[error("Capability invocation arguments could not be encoded as canonical JSON")]
    ArgumentsEncoding,
}

/// Immutable identity for one capability dispatch performed by an execution.
///
/// Operational receipt coordinates remain separate from the canonical Agents
/// request. Host and other durable-effect implementations consume
/// [`Self::canonical_request`] for argument, authority, invocation,
/// NodeExecution, effect-id, and request-digest facts; they never infer those
/// facts from `execution_id`, `graph_id`, or `node_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInvocation {
    execution_id: String,
    graph_id: String,
    node_id: u64,
    dispatch_path: CapabilityEffectDispatchPath,
    canonical_request: CapabilityRequest,
    /// Trusted projections of the grants that admitted this invocation.
    ///
    /// Executors use this only to constrain resource visibility and output;
    /// callers cannot supply it through capability arguments.
    grant_contexts: Vec<RuntimeCapabilityGrant>,
}

impl CapabilityInvocation {
    /// Construct an executor dispatch around one already-prepared canonical
    /// Capability request.
    ///
    /// # Errors
    ///
    /// Returns an error when operational receipt coordinates are empty or the
    /// canonical request fails its owner invariants.
    pub fn new(
        execution_id: impl Into<String>,
        graph_id: impl Into<String>,
        node_id: u64,
        dispatch_path: CapabilityEffectDispatchPath,
        canonical_request: CapabilityRequest,
        grant_contexts: Vec<RuntimeCapabilityGrant>,
    ) -> Result<Self, CapabilityInvocationError> {
        let execution_id = execution_id.into();
        let graph_id = graph_id.into();
        for (field, value) in [
            ("execution_id", execution_id.as_str()),
            ("graph_id", graph_id.as_str()),
        ] {
            if value.is_empty() {
                return Err(CapabilityInvocationError::EmptyField { field });
            }
        }
        canonical_request.validate().map_err(|error| {
            CapabilityInvocationError::InvalidCanonicalRequest(error.to_string())
        })?;
        Ok(Self {
            execution_id,
            graph_id,
            node_id,
            dispatch_path,
            canonical_request,
            grant_contexts,
        })
    }

    #[must_use]
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }

    #[must_use]
    pub fn graph_id(&self) -> &str {
        &self.graph_id
    }

    #[must_use]
    pub fn node_id(&self) -> u64 {
        self.node_id
    }

    #[must_use]
    pub fn dispatch_path(&self) -> CapabilityEffectDispatchPath {
        self.dispatch_path
    }

    #[must_use]
    pub fn canonical_request(&self) -> &CapabilityRequest {
        &self.canonical_request
    }

    #[must_use]
    pub fn grant_contexts(&self) -> &[RuntimeCapabilityGrant] {
        &self.grant_contexts
    }

    /// Check that the selected implementation and post-admission argument
    /// value still match the immutable canonical request.
    ///
    /// # Errors
    ///
    /// Returns an error for any request mutation, implementation mismatch, or
    /// argument edit after the owner digest was prepared.
    pub fn validate_for(
        &self,
        selected_capability: &str,
        args: &HashMap<String, Value>,
    ) -> Result<(), CapabilityInvocationError> {
        self.canonical_request.validate().map_err(|error| {
            CapabilityInvocationError::InvalidCanonicalRequest(error.to_string())
        })?;
        if self.canonical_request.capability_ref() != selected_capability {
            return Err(CapabilityInvocationError::CapabilityMismatch {
                request: self.canonical_request.capability_ref().to_string(),
                selected: selected_capability.to_string(),
            });
        }
        let actual =
            serde_json::to_value(args).map_err(|_| CapabilityInvocationError::ArgumentsEncoding)?;
        let expected = self
            .canonical_request
            .arguments()
            .value()
            .map_err(|_| CapabilityInvocationError::ArgumentsEncoding)?;
        if actual != expected {
            return Err(CapabilityInvocationError::ArgumentsMismatch);
        }
        Ok(())
    }
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
pub struct HostEffectRequestEvidence {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: u64,
    pub invocation_id: String,
    pub call_id: String,
    pub capability_id: String,
    pub agent_identity_ref: String,
    pub host_op: String,
    pub capability_binding: String,
    pub implementation_ref: String,
    pub request_digest: String,
    pub idempotency_key: String,
    pub expected_host_key_id: String,
    pub grant_refs: Vec<String>,
    pub approval_refs: Vec<String>,
}

impl From<&HostEffectRequest> for HostEffectRequestEvidence {
    fn from(prepare: &HostEffectRequest) -> Self {
        Self {
            execution_id: prepare.execution_id.clone(),
            graph_id: prepare.graph_id.clone(),
            node_id: prepare.node_id,
            invocation_id: prepare.invocation_id.clone(),
            call_id: prepare.call_id.clone(),
            capability_id: prepare.capability_id.clone(),
            agent_identity_ref: prepare.agent_identity_ref.clone(),
            host_op: prepare.host_op.clone(),
            capability_binding: prepare.capability_binding.clone(),
            implementation_ref: prepare.implementation_ref.clone(),
            request_digest: prepare.request_digest.clone(),
            idempotency_key: prepare.idempotency_key.clone(),
            expected_host_key_id: prepare.expected_host_key_id.clone(),
            grant_refs: prepare.grant_refs.clone(),
            approval_refs: prepare.approval_refs.clone(),
        }
    }
}

/// Durable replay evidence for one committed capability effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEffectReplayEvidence {
    pub receipt: CapabilityEffectReceiptPayload,
    pub host_prepare: Option<HostEffectRequestEvidence>,
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
        invocation: &CapabilityInvocation,
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

#[cfg(test)]
mod tests {
    use apxm_program::{CapabilityInvocationAuthority, CapabilityRequest};
    use serde_json::json;

    use super::*;

    fn canonical_request() -> CapabilityRequest {
        CapabilityRequest::prepare(
            "cap.search",
            "SearchArguments",
            json!({"query": "release checklist"}),
            "invocation.1",
            "node-execution.invocation.1.cap.search.4",
            CapabilityInvocationAuthority::new(
                "principal.user.1",
                "agent.gao.1",
                "grant.search.1",
                ["approval.search.1".to_string()],
            )
            .expect("valid authority"),
        )
        .expect("valid request")
    }

    fn arguments() -> HashMap<String, Value> {
        HashMap::from([(
            "query".to_string(),
            Value::String("release checklist".to_string()),
        )])
    }

    #[test]
    fn executor_boundary_accepts_only_the_exact_canonical_request() {
        let invocation = CapabilityInvocation::new(
            "execution.1",
            "graph.1",
            4,
            CapabilityEffectDispatchPath::InvCap,
            canonical_request(),
            Vec::new(),
        )
        .expect("valid invocation");

        invocation
            .validate_for("cap.search", &arguments())
            .expect("selected capability and arguments match");
        assert_eq!(
            invocation.canonical_request().arguments().type_ref(),
            "SearchArguments"
        );
        assert_eq!(
            invocation
                .canonical_request()
                .correlation()
                .program_invocation_ref
                .target,
            "invocation.1"
        );
        assert_eq!(
            invocation
                .canonical_request()
                .correlation()
                .node_execution_id,
            "node-execution.invocation.1.cap.search.4"
        );
    }

    #[test]
    fn executor_boundary_rejects_selected_capability_or_argument_mutation() {
        let invocation = CapabilityInvocation::new(
            "execution.1",
            "graph.1",
            4,
            CapabilityEffectDispatchPath::InvCap,
            canonical_request(),
            Vec::new(),
        )
        .expect("valid invocation");

        assert!(matches!(
            invocation.validate_for("cap.write", &arguments()),
            Err(CapabilityInvocationError::CapabilityMismatch { .. })
        ));
        let mut changed = arguments();
        changed.insert("query".to_string(), Value::String("changed".to_string()));
        assert_eq!(
            invocation.validate_for("cap.search", &changed),
            Err(CapabilityInvocationError::ArgumentsMismatch)
        );
    }

    #[test]
    fn request_decode_rejects_every_identity_coordinate_mutation() {
        let value = serde_json::to_value(canonical_request()).expect("serialize request");
        let mutations = [
            ("/capability_ref", json!("cap.write")),
            ("/arguments/type_ref", json!("OtherArguments")),
            (
                "/arguments/canonical_json",
                json!("{\"query\":\"changed\"}"),
            ),
            (
                "/arguments/digest",
                json!("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ),
            (
                "/correlation/program_invocation_ref/ref",
                json!("invocation.2"),
            ),
            (
                "/correlation/node_execution_id",
                json!("node-execution.invocation.1.cap.search.5"),
            ),
            (
                "/authority/acting_principal_ref/ref",
                json!("principal.user.2"),
            ),
            ("/authority/agent_identity_ref/ref", json!("agent.gao.2")),
            (
                "/authority/capability_grant_ref/ref",
                json!("grant.search.2"),
            ),
            ("/authority/approval_refs/0/ref", json!("approval.search.2")),
            (
                "/effect/effect_id",
                json!(
                    "capability-effect.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                ),
            ),
            (
                "/effect/request_digest",
                json!("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ),
        ];

        for (pointer, mutation) in mutations {
            let mut changed = value.clone();
            *changed
                .pointer_mut(pointer)
                .expect("mutation path exists in canonical request") = mutation;
            assert!(
                serde_json::from_value::<CapabilityRequest>(changed).is_err(),
                "mutation at {pointer} must fail closed"
            );
        }
    }
}
