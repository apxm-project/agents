//! Agent Program runtime kernel.
//!
//! This crate owns the generic Program Instance runtime lifecycle: product-neutral
//! Execution Admission verification, single-flight fail-busy instances, typed Hook
//! callbacks over the scoped Agent Facade lowered to the private `HookEffect<C, T>`
//! ABI, a typed port bundle constructed from immutable Exact Port Bindings
//! (validated at construction only — no registry, discovery, first-party bypass,
//! or rebind), the atomic Execution Commit port as the only commit boundary, the
//! prepared-effect reducer that decides what a durable effect outcome means, a
//! Confinement port, non-authoritative post-commit telemetry, and lifecycle
//! reconstruction from durable evidence. Adapters implement the ports it defines;
//! the Composition Root seals admissions and supplies exact implementations.

pub mod admission;
pub mod bundle;
pub mod capability;
pub mod commit;
pub mod confinement;
pub mod effect;
pub mod event_api;
pub mod events;
pub mod external_agent;
pub mod hook;
pub mod instance;
pub mod reconcile;
pub mod runtime_ports;

pub use admission::{
    AdmissionError, AdmittedCapabilityPermission, AdmittedConfinement, AdmittedModelTarget,
    AdmittedPortBinding, CAPABILITY_PORT_SCHEMA, CONFINEMENT_PORT_SCHEMA, CheckpointAdvancer,
    DURABLE_EVENT_PORT_SCHEMA, EXECUTION_ADMISSION_SCHEMA, EXECUTION_COMMIT_PORT_SCHEMA,
    EXTERNAL_AGENT_PORT_SCHEMA, ExecutionAdmission, INVOCATION_ADMISSION_SCHEMA,
    InvocationAdmission, InvocationAdmissionClaim, InvocationAdmissionError, IssuerKey,
    IssuerKeyring, IssuerSigningKey, MAX_NONCE_BYTES, MODEL_INFERENCE_PORT_SCHEMA, NonceLedger,
    PROGRAM_COMPOSITION_PORT_SCHEMA, RequirementReconciliationError, ResourceCeilings,
    RuntimeAdmission, RuntimeAdmissionError, SignatureEnvelope, SignatureRejection,
    VerifiedExecutionAdmission, VerifiedInvocationAdmission, admitted_capability_permissions,
    digest_char, digest_serializable, minimal_port_bindings, parse_execution_admission,
    reconcile_artifact_requirements, resolve_exact_bindings, unsigned_admission_skeleton,
    verify_execution_admission, verify_invocation_admission,
};
pub use bundle::{
    BundleError, ExactPortBinding, PortBundle, PortBundleSpec, PortImplementation, PortSlot,
};
pub use capability::{CapabilityOutcome, CapabilityPort, CapabilityRequest};
pub use commit::{
    ATOMIC_WRITE_SET, AtomicWriteSet, CommitRequestError, CommittedContinuation,
    ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult, ExecutionCommitTuple,
    PrecommitEvidenceRef, PreparedSessionOutputRef, ProgramInstanceRef, ProgramInvocationRef,
    SESSION_OUTPUT_REF_CONTRACT, SessionOutputPreparation, SessionOutputVisibility,
    canonical_json_bytes, canonical_json_value, continuation_digest,
    runtime_evidence_and_observation_digest, session_output_refs_digest,
};
pub use confinement::{
    ConfinementAttestation, ConfinementError, ConfinementPort, ConfinementRequest, ConfinementType,
};
pub use effect::{
    EffectError, EffectId, EffectRecord, EffectState, EffectTransition, PreparedEffect,
};
pub use event_api::{
    CanonicalEventRef, EVENT_CONTRACT, EventApplication, EventApplicationResult,
    EventContractError, EventHttpMethod, EventOccurrence, EventTypeRef, EventWaitBinding,
    InvocationBoundary,
};
pub use events::{EventSink, NullEventSink, TelemetryNote, diagnostic_may_override_evidence};
pub use external_agent::{
    AcpPromptOutcome, AcpPromptRequest, ExternalAgentCapabilityPort, PromptEffectState,
    assemble_evidence,
};
pub use hook::{AgentFacade, Hook, HookEffect, HookReturn, apply_hooks};
pub use instance::{
    CapabilityInvocation, CapabilityReport, InstanceError, Invocation, InvocationReport,
    ProgramInstance,
};
pub use reconcile::{LifecycleView, reconstruct};
pub use runtime_ports::{
    CompositionOutcome, CompositionPort, CompositionReceiver, CompositionRequest, EventAwait,
    EventOutcome, EventPort, EventRef, EventRefError,
};
