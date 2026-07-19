//! Typed model inference effect: request identity, the inference Port Contract
//! seam, usage, and the closed set of model outcomes.
//!
//! Every model effect has a stable request/effect identity. Retries reuse that
//! identity and never mint a new one. A request is built from an admitted
//! resolution for its authored target, so a request cannot carry a binding for a
//! different target — there is no substitution.

use serde::{Deserialize, Serialize};

use crate::identity::{BindingError, ModelBindingAdmission, ModelTargetRef, ResolvedModelBinding};

/// The closed typed-error categories shared with the contract common envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Validation,
    Admission,
    Authority,
    Configuration,
    Unavailable,
    Conflict,
    OutcomeUnknown,
    Internal,
}

/// A typed model-effect error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedError {
    pub category: ErrorCategory,
    pub code: String,
    pub message: String,
}

/// Typed usage evidence for one model effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// The closed set of model outcomes, aligned with the runtime-evidence
/// `model_outcome` closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelOutcome {
    CommittedSuccess { usage: Usage },
    TypedFailure { error: TypedError },
    Cancelled,
    /// An uncertain external effect. Usage/cost is uncertain and the request is
    /// never silently retried or treated as success.
    ModelOutcomeUnknown { uncertain_usage: Option<Usage> },
}

/// A stable model-effect request identity plus its resolved binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCallRequest {
    pub effect_id: String,
    pub request_digest: String,
    pub resolved_binding: ResolvedModelBinding,
}

impl ModelCallRequest {
    /// Build a request for the authored target by resolving exactly one admitted
    /// binding. The resolved binding always matches the authored target, so a
    /// request cannot substitute a different model.
    ///
    /// # Errors
    ///
    /// Returns a [`BindingError`] when the target has no single admitted binding.
    pub fn authorize(
        effect_id: impl Into<String>,
        request_digest: impl Into<String>,
        authored_target: &ModelTargetRef,
        admission: &ModelBindingAdmission,
    ) -> Result<Self, BindingError> {
        let resolved_binding = admission.resolve(authored_target)?;
        Ok(Self {
            effect_id: effect_id.into(),
            request_digest: request_digest.into(),
            resolved_binding,
        })
    }

    /// The one authored target this request binds.
    #[must_use]
    pub fn target(&self) -> &ModelTargetRef {
        &self.resolved_binding.model_target_ref
    }
}

/// The disposition of one send attempt as reported by an inference backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptDisposition {
    /// The request committed successfully with typed usage.
    Success(Usage),
    /// The attempt failed before the request left the client; it is safe to
    /// retry the same request identity.
    FailedBeforeSend(TypedError),
    /// The attempt failed after the request was sent; the outcome is uncertain
    /// unless the backend proves idempotency/reconciliation.
    FailedAfterSend(TypedError),
    /// The attempt was cancelled.
    Cancelled,
}

/// The typed model inference Port Contract seam. A backend reports per-attempt
/// dispositions and whether it can prove idempotent reconciliation after a send.
pub trait ModelInferencePort {
    /// Perform one attempt for the given request. Attempts share the request's
    /// stable identity.
    fn attempt(&self, request: &ModelCallRequest, attempt: u32) -> AttemptDisposition;

    /// Whether the backend proves idempotency/reconciliation, permitting an
    /// automatic retry after a send.
    fn proves_idempotency(&self) -> bool {
        false
    }
}

/// Bounded automatic-retry policy.
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    pub max_attempts: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 3 }
    }
}

/// Drive a model effect to a committed outcome.
///
/// Automatic retry happens only before a send, or after a send when the backend
/// proves idempotency/reconciliation. A post-send failure on a non-idempotent
/// backend never retries and never duplicates the request: it commits a typed
/// [`ModelOutcome::ModelOutcomeUnknown`].
#[must_use]
pub fn execute<P: ModelInferencePort + ?Sized>(
    port: &P,
    request: &ModelCallRequest,
    policy: RetryPolicy,
) -> ModelOutcome {
    let max = policy.max_attempts.max(1);
    for attempt in 0..max {
        let last = attempt + 1 == max;
        match port.attempt(request, attempt) {
            AttemptDisposition::Success(usage) => {
                return ModelOutcome::CommittedSuccess { usage };
            }
            AttemptDisposition::Cancelled => return ModelOutcome::Cancelled,
            AttemptDisposition::FailedBeforeSend(error) => {
                if last {
                    return ModelOutcome::TypedFailure { error };
                }
                // Safe to retry: nothing was sent.
            }
            AttemptDisposition::FailedAfterSend(error) => {
                if port.proves_idempotency() {
                    if last {
                        return ModelOutcome::TypedFailure { error };
                    }
                    // Reconcilable: retry the same identity.
                } else {
                    // Uncertain: never duplicate the request.
                    return ModelOutcome::ModelOutcomeUnknown {
                        uncertain_usage: None,
                    };
                }
            }
        }
    }
    // Unreachable for max >= 1, but fail closed rather than assume success.
    ModelOutcome::ModelOutcomeUnknown {
        uncertain_usage: None,
    }
}
