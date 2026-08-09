//! Typed model inference for the Agent Program runtime.
//!
//! This crate owns the model-effect identity and the typed inference Port
//! Contract seam consumed by `model.call`: exact binding resolution with no
//! ambient/default/alias/first-available selection, a stable request identity,
//! bounded automatic retry that never duplicates a sent non-idempotent request,
//! typed usage, streaming with explicit cancellation, and the closed model
//! outcome set including `model_outcome_unknown`.
//!
//! G4 additions (P-010 / P-012 / P-013):
//! - exact inference driver/profile/binding contracts with explicit unavailable;
//! - short-lived target- and purpose-bound credential leases in adapter memory;
//! - immutable usage lineage with timing and typed errors;
//! - correlated diagnostic signals that never replace owner evidence; and
//! - digest join of released vLLM conformance vectors without stubbing backend
//!   authority.

pub mod backend_join;
pub mod diagnostics;
pub mod dispatch;
pub mod driver;
pub mod effect;
pub mod identity;
pub mod lease;
pub mod lineage;
pub mod stream;
pub mod target;

pub use backend_join::{
    JoinError, PINNED_VLLM_OWNER_REVISION, PINNED_VLLM_PORT_CONTRACT_DIGEST,
    PINNED_VLLM_RELEASE_ID, PINNED_VLLM_RELEASE_MANIFEST_DIGEST, PINNED_VLLM_VECTOR_DIGESTS,
    VLLM_CONFORMANCE_JOIN_SCHEMA, VllmConformanceJoin, VllmJoinStatus, VllmReleaseAttestation,
    digest_bytes,
};
pub use diagnostics::{
    BoundedMetricLabels, CorrelateDiagnosticsRequest, DIAGNOSTIC_CORRELATION_SCHEMA,
    DiagnosticAgreement, DiagnosticCorrelation, DiagnosticError, authoritative_usage,
    correlate_diagnostics,
};
pub use dispatch::{
    CommittedInferenceDispatch, CommittedInferenceDispatchResult, ExactInferenceDispatch,
    InferenceDispatchError, InferenceDispatchResult, LeasedInferenceBackend,
    dispatch_committed_inference, dispatch_exact_inference,
};
pub use driver::{
    DriverAvailability, DriverBindingError, INFERENCE_DRIVER_BINDING_SCHEMA, InferenceDriverBinding,
};
pub use effect::{
    AttemptDisposition, ErrorCategory, IdempotencyKey, ModelCallPreparation, ModelCallRequest,
    ModelCallRequestError, ModelCallRequestMetadata, ModelCallRequestMetadataPort,
    ModelContextEnvelopeRef, ModelExecution, ModelInferencePort, ModelOutcome, ModelStreamMode,
    NodeExecutionId, RetryPolicy, TypedError, Usage, execute, execute_with_attempt,
};
pub use identity::{
    BindingError, ExactModelTargetRef, ExactPortBindingRef, ModelBindingAdmission,
    ModelDeploymentRef, ModelTargetRef, ResolvedModelBinding,
};
pub use lease::{
    INFERENCE_CREDENTIAL_LEASE_SCHEMA, InferenceCredentialLease, InferenceCredentialLeaseIdentity,
    LeaseError, LeasePurpose, redact_diagnostic_value,
};
pub use lineage::{
    INFERENCE_USAGE_LINEAGE_SCHEMA, InferenceLineageTarget, InferenceUsageLineage, LineageError,
    lineage_error_category,
};
pub use stream::{
    CancelToken, ModelContentRef, ModelStreamEvent, ModelStreamPort, ModelStreamStep, StreamResult,
    stream,
};
pub use target::{
    INFERENCE_TARGET_COMMITMENT_SCHEMA, InferenceTargetCommitment, TargetCommitState,
    TargetCommitmentError,
};
