//! Exact inference dispatch: authorize driver binding, redeem a lease into
//! adapter memory, invoke one ModelInferencePort, and seal usage lineage.
//!
//! This is the product-neutral driver path. It never ranks targets, never falls
//! back, and never retains lease material after the call returns.

use crate::driver::{DriverBindingError, InferenceDriverBinding};
use crate::effect::{
    AttemptDisposition, ModelCallRequest, ModelExecution, ModelInferencePort, ModelOutcome,
    RetryPolicy, TypedError, Usage, execute_with_attempt, execute_with_attempt_async,
};
use crate::identity::ModelTargetRef;
use crate::lease::{InferenceCredentialLease, LeaseError};
use crate::lineage::{InferenceUsageLineage, LineageError};
use crate::target::{InferenceTargetCommitment, TargetCommitmentError};

/// Closed failure set for exact inference dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferenceDispatchError {
    Driver(DriverBindingError),
    TargetCommitment(TargetCommitmentError),
    Lease(LeaseError),
    Lineage(LineageError),
}

impl std::fmt::Display for InferenceDispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Driver(error) => write!(f, "{error}"),
            Self::TargetCommitment(error) => write!(f, "{error}"),
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

impl From<TargetCommitmentError> for InferenceDispatchError {
    fn from(value: TargetCommitmentError) -> Self {
        Self::TargetCommitment(value)
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

/// Result of production or lease-backed dispatch after one shared immutable
/// target-validation and retry/evidence core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedInferenceDispatchResult {
    pub execution: ModelExecution,
    pub lineage: InferenceUsageLineage,
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

/// Inputs for dispatch through the already admitted production model port.
/// The target commitment is validated immediately before the first attempt;
/// this path does not discover or substitute a binding.
pub struct CommittedInferenceDispatch<'a, P: ModelInferencePort + ?Sized> {
    pub target_commitment: &'a InferenceTargetCommitment,
    pub authored_target: &'a ModelTargetRef,
    pub request: &'a ModelCallRequest,
    pub backend: &'a P,
    pub duration_ms: u64,
    pub policy: RetryPolicy,
}

/// Dispatch through one already admitted production model port using the same
/// retry and lineage core as lease-backed exact dispatch.
pub fn dispatch_committed_inference<P: ModelInferencePort + ?Sized>(
    dispatch: CommittedInferenceDispatch<'_, P>,
) -> Result<CommittedInferenceDispatchResult, InferenceDispatchError> {
    let CommittedInferenceDispatch {
        target_commitment,
        authored_target,
        request,
        backend,
        duration_ms,
        policy,
    } = dispatch;
    target_commitment.matches_resolved(authored_target, request.resolved_binding())?;
    let execution = execute_with_attempt(backend, request, policy);
    let (usage, mut typed_error) = outcome_lineage_inputs(&execution.outcome);
    if execution.outcome_unknown_error.is_some() {
        typed_error.clone_from(&execution.outcome_unknown_error);
    }
    let attempt_index = execution.committed_attempt.unwrap_or(0);
    let lineage = InferenceUsageLineage::seal_with_target_commitment(
        request.effect_id(),
        attempt_index,
        request.request_digest(),
        target_commitment,
        usage,
        duration_ms,
        typed_error,
    )?;
    Ok(CommittedInferenceDispatchResult {
        execution,
        lineage,
        target_commitment: target_commitment.clone(),
    })
}

/// Async production dispatch that keeps the model attempt future owned by the
/// canonical runtime. Dropping the enclosing runtime invocation therefore
/// cancels provider work instead of leaving a synchronous bridge parked on a
/// detached adapter runtime.
pub async fn dispatch_committed_inference_async<P: ModelInferencePort + Sync + ?Sized>(
    dispatch: CommittedInferenceDispatch<'_, P>,
) -> Result<CommittedInferenceDispatchResult, InferenceDispatchError> {
    let CommittedInferenceDispatch {
        target_commitment,
        authored_target,
        request,
        backend,
        duration_ms,
        policy,
    } = dispatch;
    target_commitment.matches_resolved(authored_target, request.resolved_binding())?;
    let execution = execute_with_attempt_async(backend, request, policy).await;
    let (usage, mut typed_error) = outcome_lineage_inputs(&execution.outcome);
    if execution.outcome_unknown_error.is_some() {
        typed_error.clone_from(&execution.outcome_unknown_error);
    }
    let attempt_index = execution.committed_attempt.unwrap_or(0);
    let lineage = InferenceUsageLineage::seal_with_target_commitment(
        request.effect_id(),
        attempt_index,
        request.request_digest(),
        target_commitment,
        usage,
        duration_ms,
        typed_error,
    )?;
    Ok(CommittedInferenceDispatchResult {
        execution,
        lineage,
        target_commitment: target_commitment.clone(),
    })
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
    let committed = dispatch_committed_inference(CommittedInferenceDispatch {
        target_commitment: &binding.target_commitment,
        authored_target,
        request,
        backend: &port,
        duration_ms,
        policy,
    })?;
    Ok(InferenceDispatchResult {
        execution: committed.execution,
        lineage: committed.lineage,
        driver_id: binding.driver_id.clone(),
        inference_profile_ref: binding.inference_profile_ref.clone(),
        target_commitment: committed.target_commitment,
    })
}

fn outcome_lineage_inputs(outcome: &ModelOutcome) -> (Usage, Option<TypedError>) {
    match outcome {
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
    }
}
