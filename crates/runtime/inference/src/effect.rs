//! Typed model inference effect: request identity, the inference Port Contract
//! seam, usage, and the closed set of model outcomes.
//!
//! Every model effect has a stable request/effect identity. Retries reuse that
//! identity and never mint a new one. A request is built from an admitted
//! resolution for its authored target, so a request cannot carry a binding for a
//! different target — there is no substitution.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
#[serde(deny_unknown_fields)]
pub struct TypedError {
    pub category: ErrorCategory,
    pub code: String,
    pub message: String,
}

/// Typed usage evidence for one model effect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// The closed set of model outcomes, aligned with the runtime-evidence
/// `model_outcome` closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelOutcome {
    CommittedSuccess {
        usage: Usage,
    },
    TypedFailure {
        error: TypedError,
    },
    Cancelled,
    /// An uncertain external effect. Usage/cost is uncertain and the request is
    /// never silently retried or treated as success.
    ModelOutcomeUnknown {
        uncertain_usage: Option<Usage>,
    },
}

/// The terminal result of one model effect together with the runtime attempt
/// that committed its native usage, when one committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelExecution {
    pub outcome: ModelOutcome,
    pub committed_attempt: Option<u32>,
    /// The typed application value returned by the committing attempt. A
    /// successful attempt always supplies this value; non-success outcomes do
    /// not manufacture one.
    pub output: Option<Value>,
}

/// Runtime-generated identity and exact admission facts for one model effect.
///
/// The runtime builds this value before requesting host-owned metadata. The
/// materializer may supply a sealed context reference, idempotency data, and a
/// delivery mode, but it cannot replace the effect, NodeExecution, request
/// digest, target, deployment, binding, or deployment composition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCallPreparation {
    effect_id: String,
    node_execution_id: NodeExecutionId,
    request_digest: String,
    context_digest: String,
    authored_request: Value,
    resolved_binding: ResolvedModelBinding,
}

impl ModelCallPreparation {
    /// Validate the admission for the authored target and prepare the immutable
    /// runtime-owned coordinates for one inference request.
    ///
    /// # Errors
    ///
    /// Returns [`BindingError`] when admission does not authorize exactly one
    /// valid binding for the authored target.
    pub fn authorize(
        effect_id: impl Into<String>,
        node_execution_id: impl Into<String>,
        request_digest: impl Into<String>,
        context_digest: impl Into<String>,
        authored_request: Value,
        authored_target: &ModelTargetRef,
        admission: &ModelBindingAdmission,
    ) -> Result<Self, BindingError> {
        Ok(Self {
            effect_id: effect_id.into(),
            node_execution_id: NodeExecutionId(node_execution_id.into()),
            request_digest: request_digest.into(),
            context_digest: context_digest.into(),
            authored_request,
            resolved_binding: admission.validate(authored_target)?,
        })
    }

    /// The stable effect identity generated before an external send.
    #[must_use]
    pub fn effect_id(&self) -> &str {
        &self.effect_id
    }

    /// The one compiled NodeExecution this request belongs to.
    #[must_use]
    pub fn node_execution_id(&self) -> &NodeExecutionId {
        &self.node_execution_id
    }

    /// The stable digest over the exact request identity.
    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }

    /// Digest of the exact persistent application Context incorporated into
    /// this request identity.
    #[must_use]
    pub fn context_digest(&self) -> &str {
        &self.context_digest
    }

    /// The exact authored request SSA value materialized by execution.
    #[must_use]
    pub fn authored_request(&self) -> &Value {
        &self.authored_request
    }

    /// The admission-validated model target, deployment, binding, and
    /// composition coordinates.
    #[must_use]
    pub fn resolved_binding(&self) -> &ResolvedModelBinding {
        &self.resolved_binding
    }
}

/// One exact runtime NodeExecution identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeExecutionId(pub String);

impl NodeExecutionId {
    /// Borrow the wire value without losing its domain type at call sites.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A reference to an already-sealed `apxm.model-context-envelope` instance.
///
/// The inference request carries only this identity and digest; it never copies
/// or assembles model-visible context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelContextEnvelopeRef {
    pub context_id: String,
    pub sealed_digest: String,
}

/// A stable idempotency identity for one externally visible model effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdempotencyKey {
    pub key_id: String,
    pub scope_ref: String,
}

/// The closed delivery mode supported by the inference request contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStreamMode {
    Buffered,
    Streamed,
}

/// Host-owned metadata that completes an already-admitted model request.
///
/// This does not include a target or binding, so an implementation cannot use
/// it to select, substitute, or rebind a model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCallRequestMetadata {
    pub model_context_envelope_ref: ModelContextEnvelopeRef,
    pub idempotency: IdempotencyKey,
    pub stream_mode: ModelStreamMode,
}

/// The injected upstream seam that supplies metadata for each exact model
/// effect. It is not an HTTP client or a model resolver; the Composition Root
/// provides an implementation that returns already-sealed, already-admitted
/// values.
pub trait ModelCallRequestMetadataPort: Send + Sync {
    /// Materialize metadata for the runtime-owned preparation. An unavailable
    /// or invalid value fails the effect before it reaches the inference
    /// adapter.
    fn materialize(
        &self,
        preparation: &ModelCallPreparation,
    ) -> Result<ModelCallRequestMetadata, TypedError>;
}

/// A complete, validated inference request. Its fields are private so callers
/// cannot bypass the exact-binding, sealed-context, and idempotency checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCallRequest {
    effect_id: String,
    node_execution_id: NodeExecutionId,
    request_digest: String,
    context_digest: String,
    authored_request: Value,
    resolved_binding: ResolvedModelBinding,
    model_context_envelope_ref: ModelContextEnvelopeRef,
    idempotency: IdempotencyKey,
    stream_mode: ModelStreamMode,
}

/// Why a fully materialized model request cannot be admitted. Every variant
/// rejects dispatch before the adapter is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelCallRequestError {
    EmptyField(&'static str),
    InvalidDigest(&'static str),
}

impl std::fmt::Display for ModelCallRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "model request field {field} is empty"),
            Self::InvalidDigest(field) => {
                write!(f, "model request field {field} is not a sha256 digest")
            }
        }
    }
}

impl std::error::Error for ModelCallRequestError {}

impl ModelCallRequest {
    /// Complete one exact model-call request from runtime-owned preparation and
    /// host-supplied metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ModelCallRequestError`] when any required identity is empty or
    /// any supplied digest is not content addressed. It never fills missing
    /// values from defaults, aliases, or ambient configuration.
    pub fn prepare(
        preparation: ModelCallPreparation,
        metadata: ModelCallRequestMetadata,
    ) -> Result<Self, ModelCallRequestError> {
        for (field, value) in [
            ("effect_id", preparation.effect_id.as_str()),
            ("node_execution_id", preparation.node_execution_id.as_str()),
            ("idempotency.key_id", metadata.idempotency.key_id.as_str()),
            (
                "idempotency.scope_ref",
                metadata.idempotency.scope_ref.as_str(),
            ),
            (
                "model_context_envelope_ref.context_id",
                metadata.model_context_envelope_ref.context_id.as_str(),
            ),
        ] {
            if value.trim().is_empty() {
                return Err(ModelCallRequestError::EmptyField(field));
            }
        }
        for (field, digest) in [
            ("request_digest", preparation.request_digest.as_str()),
            ("context_digest", preparation.context_digest.as_str()),
            (
                "model_context_envelope_ref.sealed_digest",
                metadata.model_context_envelope_ref.sealed_digest.as_str(),
            ),
        ] {
            if !apxm_program::grammar::is_digest(digest) {
                return Err(ModelCallRequestError::InvalidDigest(field));
            }
        }

        Ok(Self {
            effect_id: preparation.effect_id,
            node_execution_id: preparation.node_execution_id,
            request_digest: preparation.request_digest,
            context_digest: preparation.context_digest,
            authored_request: preparation.authored_request,
            resolved_binding: preparation.resolved_binding,
            model_context_envelope_ref: metadata.model_context_envelope_ref,
            idempotency: metadata.idempotency,
            stream_mode: metadata.stream_mode,
        })
    }

    /// The stable effect identity prepared before external transmission.
    #[must_use]
    pub fn effect_id(&self) -> &str {
        &self.effect_id
    }

    /// The exact NodeExecution this request represents.
    #[must_use]
    pub fn node_execution_id(&self) -> &NodeExecutionId {
        &self.node_execution_id
    }

    /// The request digest shared by the idempotency and runtime evidence paths.
    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }

    /// Digest of the persistent application Context used for this exact call.
    #[must_use]
    pub fn context_digest(&self) -> &str {
        &self.context_digest
    }

    /// The exact authored request value delivered to the inference adapter.
    #[must_use]
    pub fn authored_request(&self) -> &Value {
        &self.authored_request
    }

    /// The exact target/deployment/Port Binding admitted for this effect.
    #[must_use]
    pub fn resolved_binding(&self) -> &ResolvedModelBinding {
        &self.resolved_binding
    }

    /// The reference to the already-sealed model context for this NodeExecution.
    #[must_use]
    pub fn model_context_envelope_ref(&self) -> &ModelContextEnvelopeRef {
        &self.model_context_envelope_ref
    }

    /// The stable idempotency scope and key supplied upstream.
    #[must_use]
    pub fn idempotency(&self) -> &IdempotencyKey {
        &self.idempotency
    }

    /// The explicitly selected buffered or streamed delivery mode.
    #[must_use]
    pub fn stream_mode(&self) -> ModelStreamMode {
        self.stream_mode
    }

    /// The one authored target this request binds.
    #[must_use]
    pub fn target(&self) -> &ModelTargetRef {
        &self.resolved_binding.model_target.reference
    }
}

/// The disposition of one send attempt as reported by an inference backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptDisposition {
    /// The request committed successfully with typed usage and application
    /// output. Output is mandatory at this boundary so downstream typed control
    /// flow never substitutes a sentinel value for a missing model response.
    Success { usage: Usage, output: Value },
    /// The backend delivered a reconciled terminal typed failure. The request
    /// was sent, but its terminal outcome is known and must not be retried or
    /// degraded to outcome-unknown.
    DeliveredTypedFailure(TypedError),
    /// The attempt failed before the request left the client; it is safe to
    /// retry the same request identity.
    FailedBeforeSend(TypedError),
    /// The attempt failed after the request was sent; the outcome is uncertain
    /// unless the backend proves idempotency/reconciliation.
    FailedAfterSend(TypedError),
    /// The attempt was cancelled.
    Cancelled,
}

/// The future returned by the cancellation-aware model port seam.
///
/// The legacy [`ModelInferencePort::attempt`] method remains available for
/// synchronous callers, but canonical runtime execution must use this future
/// so dropping an invocation drops the provider future too.
pub type ModelAttemptFuture<'a> = Pin<Box<dyn Future<Output = AttemptDisposition> + Send + 'a>>;

/// The typed model inference Port Contract seam. A backend reports per-attempt
/// dispositions and whether it can prove idempotent reconciliation after a send.
pub trait ModelInferencePort {
    /// Perform one attempt for the given request. Attempts share the request's
    /// stable identity.
    fn attempt(&self, request: &ModelCallRequest, attempt: u32) -> AttemptDisposition;

    /// Perform one attempt without introducing a blocking bridge.
    ///
    /// The default preserves compatibility for synchronous test and embedded
    /// ports. A native async adapter overrides this method and awaits its
    /// provider directly; the owning runtime can then cancel the future at its
    /// wall deadline without leaving detached provider work behind.
    fn attempt_async<'a>(
        &'a self,
        request: &'a ModelCallRequest,
        attempt: u32,
    ) -> ModelAttemptFuture<'a>
    where
        Self: Sync,
    {
        Box::pin(async move { self.attempt(request, attempt) })
    }

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
    execute_with_attempt(port, request, policy).outcome
}

/// Drive a model effect and retain the successful attempt coordinate for
/// runtime-owned evidence and post-commit usage measurement.
#[must_use]
pub fn execute_with_attempt<P: ModelInferencePort + ?Sized>(
    port: &P,
    request: &ModelCallRequest,
    policy: RetryPolicy,
) -> ModelExecution {
    let max = policy.max_attempts.max(1);
    for attempt in 0..max {
        let last = attempt + 1 == max;
        match port.attempt(request, attempt) {
            AttemptDisposition::Success { usage, output } => {
                return ModelExecution {
                    outcome: ModelOutcome::CommittedSuccess { usage },
                    committed_attempt: Some(attempt),
                    output: Some(output),
                };
            }
            AttemptDisposition::DeliveredTypedFailure(error) => {
                return ModelExecution {
                    outcome: ModelOutcome::TypedFailure { error },
                    committed_attempt: None,
                    output: None,
                };
            }
            AttemptDisposition::Cancelled => {
                return ModelExecution {
                    outcome: ModelOutcome::Cancelled,
                    committed_attempt: None,
                    output: None,
                };
            }
            AttemptDisposition::FailedBeforeSend(error) => {
                if last {
                    return ModelExecution {
                        outcome: ModelOutcome::TypedFailure { error },
                        committed_attempt: None,
                        output: None,
                    };
                }
                // Safe to retry: nothing was sent.
            }
            AttemptDisposition::FailedAfterSend(error) => {
                if port.proves_idempotency() {
                    if last {
                        return ModelExecution {
                            outcome: ModelOutcome::TypedFailure { error },
                            committed_attempt: None,
                            output: None,
                        };
                    }
                    // Reconcilable: retry the same identity.
                } else {
                    // Uncertain: never duplicate the request.
                    return ModelExecution {
                        outcome: ModelOutcome::ModelOutcomeUnknown {
                            uncertain_usage: None,
                        },
                        committed_attempt: None,
                        output: None,
                    };
                }
            }
        }
    }
    // Unreachable for max >= 1, but fail closed rather than assume success.
    ModelExecution {
        outcome: ModelOutcome::ModelOutcomeUnknown {
            uncertain_usage: None,
        },
        committed_attempt: None,
        output: None,
    }
}

/// Async counterpart to [`execute`]. The attempt future is owned directly by
/// the caller, so cancellation propagates through retries into the provider
/// adapter rather than parking a caller thread on a detached runtime task.
#[must_use]
pub async fn execute_async<P: ModelInferencePort + Sync + ?Sized>(
    port: &P,
    request: &ModelCallRequest,
    policy: RetryPolicy,
) -> ModelOutcome {
    execute_with_attempt_async(port, request, policy)
        .await
        .outcome
}

/// Async counterpart to [`execute_with_attempt`].
#[must_use]
pub async fn execute_with_attempt_async<P: ModelInferencePort + Sync + ?Sized>(
    port: &P,
    request: &ModelCallRequest,
    policy: RetryPolicy,
) -> ModelExecution {
    let max = policy.max_attempts.max(1);
    for attempt in 0..max {
        let last = attempt + 1 == max;
        match port.attempt_async(request, attempt).await {
            AttemptDisposition::Success { usage, output } => {
                return ModelExecution {
                    outcome: ModelOutcome::CommittedSuccess { usage },
                    committed_attempt: Some(attempt),
                    output: Some(output),
                };
            }
            AttemptDisposition::DeliveredTypedFailure(error) => {
                return ModelExecution {
                    outcome: ModelOutcome::TypedFailure { error },
                    committed_attempt: None,
                    output: None,
                };
            }
            AttemptDisposition::Cancelled => {
                return ModelExecution {
                    outcome: ModelOutcome::Cancelled,
                    committed_attempt: None,
                    output: None,
                };
            }
            AttemptDisposition::FailedBeforeSend(error) => {
                if last {
                    return ModelExecution {
                        outcome: ModelOutcome::TypedFailure { error },
                        committed_attempt: None,
                        output: None,
                    };
                }
            }
            AttemptDisposition::FailedAfterSend(error) => {
                if port.proves_idempotency() {
                    if last {
                        return ModelExecution {
                            outcome: ModelOutcome::TypedFailure { error },
                            committed_attempt: None,
                            output: None,
                        };
                    }
                } else {
                    return ModelExecution {
                        outcome: ModelOutcome::ModelOutcomeUnknown {
                            uncertain_usage: None,
                        },
                        committed_attempt: None,
                        output: None,
                    };
                }
            }
        }
    }
    ModelExecution {
        outcome: ModelOutcome::ModelOutcomeUnknown {
            uncertain_usage: None,
        },
        committed_attempt: None,
        output: None,
    }
}
