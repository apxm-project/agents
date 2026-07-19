//! Typed model inference for the Agent Program runtime.
//!
//! This crate owns the model-effect identity and the typed inference Port
//! Contract seam consumed by `model.call`: exact binding resolution with no
//! ambient/default/alias/first-available selection, a stable request identity,
//! bounded automatic retry that never duplicates a sent non-idempotent request,
//! typed usage, streaming with explicit cancellation, and the closed model
//! outcome set including `model_outcome_unknown`. It carries no live backend and
//! selects no implementation.

pub mod effect;
pub mod identity;
pub mod stream;

pub use effect::{
    execute, AttemptDisposition, ErrorCategory, ModelCallRequest, ModelInferencePort, ModelOutcome,
    RetryPolicy, TypedError, Usage,
};
pub use identity::{
    AdmissionEntry, BindingError, ExactPortBindingRef, ModelBindingAdmission, ModelDeploymentRef,
    ModelTargetRef, ResolvedModelBinding,
};
pub use stream::{stream, CancelToken, ModelStreamPort, StreamChunk, StreamResult};
