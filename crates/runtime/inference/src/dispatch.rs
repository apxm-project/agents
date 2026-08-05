//! Exact inference dispatch: authorize driver binding, redeem a lease into
//! adapter memory, invoke one ModelInferencePort, and seal usage lineage.
//!
//! This is the product-neutral driver path. It never ranks targets, never falls
//! back, and never retains lease material after the call returns.

use crate::driver::{DriverBindingError, InferenceDriverBinding};
use crate::effect::{
    AttemptDisposition, ModelCallRequest, ModelExecution, ModelInferencePort, ModelOutcome,
    RetryPolicy, TypedError, Usage, execute_with_attempt,
};
use crate::identity::ModelTargetRef;
use crate::lease::{InferenceCredentialLease, LeaseError};
use crate::lineage::{InferenceUsageLineage, LineageError};
use crate::target::InferenceTargetCommitment;

/// Closed failure set for exact inference dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferenceDispatchError {
    Driver(DriverBindingError),
    Lease(LeaseError),
    Lineage(LineageError),
}

impl std::fmt::Display for InferenceDispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Driver(error) => write!(f, "{error}"),
            Self::Lease(error) => write!(f, "{error}"),
            Self::Lineage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for InferenceDispatchError {}

impl From<DriverBindingError> for InferenceDispatchError {
    fn from(value: DriverBindingError) -> Self {
        Self::Driver(value)
    }
}

impl From<LeaseError> for InferenceDispatchError {
    fn from(value: LeaseError) -> Self {
        Self::Lease(value)
    }
}

impl From<LineageError> for InferenceDispatchError {
    fn from(value: LineageError) -> Self {
        Self::Lineage(value)
    }
}

/// Result of one exact driver dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InferenceDispatchResult {
    pub execution: ModelExecution,
    pub lineage: InferenceUsageLineage,
    pub driver_id: String,
    pub inference_profile_ref: String,
    pub target_commitment: InferenceTargetCommitment,
}

/// Lease-aware backend used only inside adapter memory for one attempt.
pub trait LeasedInferenceBackend {
    fn attempt_with_lease(
        &self,
        request: &ModelCallRequest,
        attempt: u32,
        lease_material: &str,
    ) -> AttemptDisposition;

    fn proves_idempotency(&self) -> bool {
        false
    }
}

/// Adapt a leased backend into the existing [`ModelInferencePort`] seam after
/// the lease has been redeemed into a process-local guard.
struct RedeemedPort<'a, B: LeasedInferenceBackend + ?Sized> {
    backend: &'a B,
    material: &'a str,
}

impl<B: LeasedInferenceBackend + ?Sized> ModelInferencePort for RedeemedPort<'_, B> {
    fn attempt(&self, request: &ModelCallRequest, attempt: u32) -> AttemptDisposition {
        self.backend
            .attempt_with_lease(request, attempt, self.material)
    }

    fn proves_idempotency(&self) -> bool {
        self.backend.proves_idempotency()
    }
}

/// Inputs for one exact leased inference dispatch.
pub struct ExactInferenceDispatch<'a, B: LeasedInferenceBackend + ?Sized> {
    pub binding: &'a InferenceDriverBinding,
    pub authored_target: &'a ModelTargetRef,
    pub request: &'a ModelCallRequest,
    pub lease: &'a mut InferenceCredentialLease,
    pub backend: &'a B,
    pub now_unix_ms: u64,
    pub duration_ms: u64,
    pub policy: RetryPolicy,
}

/// Authorize one exact driver binding, redeem one purpose/target-bound lease,
/// dispatch through the leased backend, and seal immutable usage lineage.
pub fn dispatch_exact_inference<B: LeasedInferenceBackend + ?Sized>(
    dispatch: ExactInferenceDispatch<'_, B>,
) -> Result<InferenceDispatchResult, InferenceDispatchError> {
    let ExactInferenceDispatch {
        binding,
        authored_target,
        request,
        lease,
        backend,
        now_unix_ms,
        duration_ms,
        policy,
    } = dispatch;
    binding.authorize(authored_target, request.resolved_binding())?;
    lease.activate(
        authored_target,
        &binding.exact_port_binding_digest,
        now_unix_ms,
    )?;
    let material = lease.expose()?;
    let port = RedeemedPort { backend, material };
    let execution = execute_with_attempt(&port, request, policy);
    let (usage, typed_error) = match &execution.outcome {
        ModelOutcome::CommittedSuccess { usage } => (*usage, None),
        ModelOutcome::TypedFailure { error } => (Usage::default(), Some(error.clone())),
        ModelOutcome::Cancelled => (
            Usage::default(),
            Some(TypedError {
                category: crate::effect::ErrorCategory::Unavailable,
                code: "cancelled".to_string(),
                message: "model effect cancelled".to_string(),
            }),
        ),
        ModelOutcome::ModelOutcomeUnknown { uncertain_usage } => (
            uncertain_usage.unwrap_or_default(),
            Some(TypedError {
                category: crate::effect::ErrorCategory::OutcomeUnknown,
                code: "model_outcome_unknown".to_string(),
                message: "model effect outcome is unknown".to_string(),
            }),
        ),
    };
    let attempt_index = execution.committed_attempt.unwrap_or(0);
    let lineage = InferenceUsageLineage::seal_with_target_commitment(
        request.effect_id(),
        attempt_index,
        request.request_digest(),
        &binding.target_commitment,
        usage,
        duration_ms,
        typed_error,
    )?;
    Ok(InferenceDispatchResult {
        execution,
        lineage,
        driver_id: binding.driver_id.clone(),
        inference_profile_ref: binding.inference_profile_ref.clone(),
        target_commitment: binding.target_commitment.clone(),
    })
}
