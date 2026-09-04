//! Native Runtime Service protocol.
//!
//! Program Instance, Invocation, Event ingress, approval, cancellation, and
//! observations live here. Source, FrontendGraph, AIR text, and compiler
//! options are not representable as executable truth.
//!
//! Protocol/1's request union is closed: it grows only by an owner decision
//! recorded as an ADR, never to smuggle a surface through. ADR-0025 added
//! `capability_fulfill` and `capability_cancel` so the embedding host can
//! settle a host-fulfilled Capability request on the channel it received the
//! request on. Admission binding remains a separately negotiated
//! execution-admission envelope; it is not smuggled through the read-only
//! Runtime/2 handshake.

use apxm_core::types::host_capability::is_host_capability_request_id;
use apxm_kernel::event_api::{
    CanonicalEventRef, EventApplication, EventApplicationResult, InvocationBoundary,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub mod execution_contracts;

pub use execution_contracts::{
    AttemptId, AuthoredPermission, Commitment, ContentReadResult, ContentRef,
    ContractValidationError, CorrelationId, EXECUTION_OBSERVATION_CONTRACT,
    EXECUTION_READ_CONTRACT, EventObservationRef, EvidenceFactKind, EvidenceRecord, EvidenceRef,
    ExecutionCursor, ExecutionObservation, ExecutionPage, ExecutionReadRequest,
    ExecutionReadResult, GrantRef, HostCapabilityObservation, HostCapabilityOutcomeKind,
    NODE_EXECUTION_INSPECTION_CONTRACT, NodeExecutionId, NodeExecutionInspection,
    NodeExecutionStatus, ObservationId, ObservationKind, ObservationTiming, OutputRef,
    OutputVisibility, PrincipalRef, ProgramInstanceId, ProgramInvocationId,
    ProgramInvocationInspection, ProgramInvocationStatus, ProgramRef, ReadContext, ReadPurpose,
    RegionOccurrenceId, RequestId, SESSION_OUTPUT_REF_CONTRACT, ScopeRef, SessionOutputRef,
};

/// Authoritative lifecycle state for one reserved EventRef.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Pending,
    Fulfilled,
    Expired,
    Cancelled,
}

/// Authorized inspection of one EventRef reservation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventInspection {
    pub event_ref: CanonicalEventRef,
    pub type_id: String,
    pub status: EventStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<String>,
}

/// Only declared Runtime protocol version. Unknown versions fail closed.
pub const RUNTIME_PROTOCOL_VERSION: &str = "apxm.runtime.protocol/1";

/// Separately negotiated mutation envelope for Runtime-owned admission.
/// This is intentionally not the read-only Protocol/2 surface below.
pub const RUNTIME_EXECUTION_ADMISSION_VERSION: &str = "apxm.runtime.execution-admission/1";
pub const EXECUTION_ADMISSION_CONTRACT: &str = "apxm.runtime.execution-admission";

/// Additive read/observation protocol. Protocol/1 remains frozen and is not
/// silently widened; clients opt into this separately negotiated surface.
pub const RUNTIME_PROTOCOL_V2_VERSION: &str = "apxm.runtime.protocol/2";
pub const EXECUTION_READ_SCHEMA_DIGEST: &str =
    "sha256:e168df958f22f2f7c488aa6b4f5fa39d26a1b21822eeb502e9e6ccf7d13f651b";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFeature {
    #[serde(rename = "observation.subscribe")]
    ObservationSubscribe,
    #[serde(rename = "program_invocation.inspect")]
    ProgramInvocationInspect,
    #[serde(rename = "content.read")]
    ContentRead,
    #[serde(rename = "output.read")]
    OutputRead,
    #[serde(rename = "evidence.read")]
    EvidenceRead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFailureCode {
    Unavailable,
    InvalidRequest,
    Unauthorized,
    NotFound,
    RetentionGap,
    UnsupportedFeature,
    ProtocolSkew,
    InternalError,
}

/// Opaque possession claim minted by the Runtime Service.
///
/// This is a typed capability claim, not a claim that the transport
/// authenticated the caller. A transport that has no caller-authentication
/// field must not be described as authenticated; it can still require the
/// exact unguessable claim returned at instance or Event reservation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeOwnerClaim {
    /// Opaque service-minted claim. Only exact equality authorizes a state
    /// transition at this protocol boundary.
    pub value: String,
}

impl RuntimeOwnerClaim {
    /// Mint an unguessable claim using the operating system CSPRNG through
    /// UUID v4.
    #[must_use]
    pub fn mint() -> Self {
        Self {
            value: format!("owner-{}", Uuid::new_v4()),
        }
    }

    /// Validate the closed wire grammar without treating it as caller auth.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let Some(uuid) = self.value.strip_prefix("owner-") else {
            return Err(ProtocolError::InvalidOwnerClaim);
        };
        let parsed = Uuid::parse_str(uuid).map_err(|_| ProtocolError::InvalidOwnerClaim)?;
        (parsed.get_version_num() == 4)
            .then_some(())
            .ok_or(ProtocolError::InvalidOwnerClaim)
    }
}

/// Client-to-service handshake.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHandshake {
    /// Exact protocol version the client speaks.
    pub protocol_version: String,
}

/// Runtime-owned admission inputs advertised after instance creation. The
/// carrier bytes remain image/host composition data; these fields let a
/// caller construct a matching template without copying APXM constants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAdmissionProfileDescriptor {
    pub profile_ref: String,
    pub port_bindings_digest: String,
    pub resource_ceiling_digest: String,
}

impl RuntimeHandshake {
    /// Accept only the frozen version. No downgrade or legacy unions.
    pub fn admit(&self) -> Result<(), ProtocolError> {
        if self.protocol_version == RUNTIME_PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(ProtocolError::IncompatibleVersion)
        }
    }
}

/// Handshake for the additive observation/read surface. It is deliberately a
/// separate type so a Protocol/1 peer cannot accidentally receive new methods.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHandshakeV2 {
    pub protocol_version: String,
    pub contract: String,
    pub schema_digest: String,
    pub supported_features: Vec<RuntimeFeature>,
}

impl RuntimeHandshakeV2 {
    #[must_use]
    pub fn server() -> Self {
        Self {
            protocol_version: RUNTIME_PROTOCOL_V2_VERSION.to_owned(),
            contract: EXECUTION_READ_CONTRACT.to_owned(),
            schema_digest: EXECUTION_READ_SCHEMA_DIGEST.to_owned(),
            supported_features: vec![
                RuntimeFeature::ObservationSubscribe,
                RuntimeFeature::ProgramInvocationInspect,
                RuntimeFeature::ContentRead,
                RuntimeFeature::OutputRead,
                RuntimeFeature::EvidenceRead,
            ],
        }
    }

    pub fn admit(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != RUNTIME_PROTOCOL_V2_VERSION {
            return Err(ProtocolError::IncompatibleVersion);
        }
        if self.contract != EXECUTION_READ_CONTRACT
            || self.schema_digest != EXECUTION_READ_SCHEMA_DIGEST
        {
            return Err(ProtocolError::SchemaMismatch);
        }
        let mut seen = std::collections::HashSet::new();
        if self
            .supported_features
            .iter()
            .any(|feature| !seen.insert(feature))
            || self.supported_features.is_empty()
        {
            return Err(ProtocolError::InvalidHandshake);
        }
        Ok(())
    }

    pub fn negotiate(&self, peer: &Self) -> Result<Vec<RuntimeFeature>, ProtocolError> {
        self.admit()?;
        peer.admit()?;
        let negotiated = self
            .supported_features
            .iter()
            .copied()
            .filter(|feature| peer.supported_features.contains(feature))
            .collect::<Vec<_>>();
        if negotiated.is_empty() {
            return Err(ProtocolError::UnsupportedFeature);
        }
        Ok(negotiated)
    }
}

/// Closed client request set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeRequest {
    /// Create a Program Instance from a committed artifact digest.
    ProgramInstanceCreate {
        /// Caller correlation id.
        request_id: String,
        /// Digest-bound executable artifact. Never source.
        artifact_digest: String,
    },
    /// Start an Invocation on an admitted instance.
    ProgramInvocationStart {
        /// Caller correlation id.
        request_id: String,
        /// Program Instance id.
        program_instance_id: String,
        /// Exact claim returned when this instance was created.
        owner_claim: RuntimeOwnerClaim,
        /// Typed Invocation input JSON.
        input: serde_json::Value,
    },
    /// Reserve an EventRef through the root Event API.
    EventReserve {
        /// Caller correlation id.
        request_id: String,
        /// Authored Event type id. Not itself an EventRef.
        type_id: String,
    },
    /// Apply an admitted occurrence to an EventRef.
    EventFulfill {
        /// Caller correlation id.
        request_id: String,
        /// Exact claim returned when this EventRef was reserved.
        owner_claim: RuntimeOwnerClaim,
        /// Canonical application record.
        application: EventApplication<serde_json::Value>,
    },
    /// List pending EventRefs owned by an exact reservation claim.
    EventList {
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
    },
    /// Inspect an EventRef.
    EventInspect {
        /// Caller correlation id.
        request_id: String,
        /// Exact claim returned when this EventRef was reserved.
        owner_claim: RuntimeOwnerClaim,
        /// Target EventRef.
        event_ref: CanonicalEventRef,
    },
    /// Expire a pending EventRef through the root Event API.
    EventExpire {
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
        event_ref: CanonicalEventRef,
    },
    /// Cancel a pending EventRef through the root Event API.
    EventCancel {
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
        event_ref: CanonicalEventRef,
    },
    /// Settle one outstanding host-fulfilled Capability request.
    ///
    /// The runtime published the request as a `capability_requested`
    /// observation and parked the node; this is the answer. `cancelled` is not
    /// a settlement a host sends — `capability_cancel` withdraws a request —
    /// and `output` is stated only for `ok`. Retrying the same request identity
    /// with identical settlement content returns its authoritative result;
    /// changing any settlement content is an invalid request.
    CapabilityFulfill {
        /// Caller correlation id.
        request_id: String,
        /// Exact claim returned when the Program Instance was created.
        owner_claim: RuntimeOwnerClaim,
        /// The request identity the `capability_requested` observation carried.
        capability_request_id: String,
        /// How the host settled it.
        outcome: HostCapabilityOutcomeKind,
        /// The exact canonical JSON output bytes, for `ok` only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        /// The host's own durable record of the effect.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        receipt_ref: Option<String>,
        /// Why a settlement that is not `ok` decided what it decided.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Withdraw one outstanding host-fulfilled Capability request.
    ///
    /// The node settles as an uncertain effect: a request the host withdrew
    /// may or may not have reached the system behind it. An identical retry is
    /// idempotent and a retry that conflicts with a prior settlement is
    /// invalid.
    CapabilityCancel {
        /// Caller correlation id.
        request_id: String,
        /// Exact claim returned when the Program Instance was created.
        owner_claim: RuntimeOwnerClaim,
        /// The request identity the `capability_requested` observation carried.
        capability_request_id: String,
        /// Why the host withdrew it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Cancel a Program Invocation.
    ProgramInvocationCancel {
        /// Caller correlation id.
        request_id: String,
        /// Exact claim returned when this instance was created.
        owner_claim: RuntimeOwnerClaim,
        /// Invocation to cancel.
        program_invocation_id: String,
    },
}

/// Handshake for the separately negotiated execution-admission mutation
/// envelope. A Protocol/1 peer cannot decode or receive this method.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeExecutionAdmissionHandshake {
    pub protocol_version: String,
    pub contract: String,
}

impl RuntimeExecutionAdmissionHandshake {
    #[must_use]
    pub fn server() -> Self {
        Self {
            protocol_version: RUNTIME_EXECUTION_ADMISSION_VERSION.to_owned(),
            contract: EXECUTION_ADMISSION_CONTRACT.to_owned(),
        }
    }

    pub fn admit(&self) -> Result<(), ProtocolError> {
        if self.protocol_version != RUNTIME_EXECUTION_ADMISSION_VERSION {
            return Err(ProtocolError::IncompatibleVersion);
        }
        if self.contract != EXECUTION_ADMISSION_CONTRACT {
            return Err(ProtocolError::SchemaMismatch);
        }
        Ok(())
    }
}

/// Mutation request that requires the execution-admission handshake.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeExecutionAdmissionRequest {
    ProgramInstanceBindAdmission {
        request_id: String,
        program_instance_id: String,
        owner_claim: RuntimeOwnerClaim,
        admission_profile_ref: String,
    },
}

/// Additive Protocol/2 read and observation requests. Execution mutation and
/// Event ingress remain Protocol/1 concerns; these methods cannot be decoded
/// by a Protocol/1 peer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeRequestV2 {
    #[serde(rename = "program_invocation.inspect")]
    ProgramInvocationInspect {
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
        #[serde(skip_serializing_if = "Option::is_none")]
        node_execution_id: Option<NodeExecutionId>,
    },
    #[serde(rename = "observation.subscribe")]
    ObservationSubscribe {
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
        after_cursor: Option<ExecutionCursor>,
        limit: u32,
    },
    #[serde(rename = "content.read")]
    ContentRead {
        context: ReadContext,
        content_ref: ContentRef,
    },
    #[serde(rename = "output.read")]
    OutputRead {
        context: ReadContext,
        output_ref: OutputRef,
    },
    #[serde(rename = "evidence.read")]
    EvidenceRead {
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
        after_cursor: Option<ExecutionCursor>,
        limit: u32,
    },
}

impl RuntimeRequestV2 {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        self.as_execution_read_request().validate()
    }

    #[must_use]
    pub fn as_execution_read_request(&self) -> ExecutionReadRequest {
        match self {
            Self::ProgramInvocationInspect {
                context,
                program_invocation_id,
                node_execution_id,
            } => ExecutionReadRequest::ProgramInvocationInspect {
                context: context.clone(),
                program_invocation_id: program_invocation_id.clone(),
                node_execution_id: node_execution_id.clone(),
            },
            Self::ObservationSubscribe {
                context,
                program_invocation_id,
                after_cursor,
                limit,
            } => ExecutionReadRequest::ObservationSubscribe {
                context: context.clone(),
                program_invocation_id: program_invocation_id.clone(),
                after_cursor: after_cursor.clone(),
                limit: *limit,
            },
            Self::ContentRead {
                context,
                content_ref,
            } => ExecutionReadRequest::ContentRead {
                context: context.clone(),
                content_ref: content_ref.clone(),
            },
            Self::OutputRead {
                context,
                output_ref,
            } => ExecutionReadRequest::OutputRead {
                context: context.clone(),
                output_ref: output_ref.clone(),
            },
            Self::EvidenceRead {
                context,
                program_invocation_id,
                after_cursor,
                limit,
            } => ExecutionReadRequest::EvidenceRead {
                context: context.clone(),
                program_invocation_id: program_invocation_id.clone(),
                after_cursor: after_cursor.clone(),
                limit: *limit,
            },
        }
    }

    #[must_use]
    pub fn feature(&self) -> RuntimeFeature {
        match self {
            Self::ProgramInvocationInspect { .. } => RuntimeFeature::ProgramInvocationInspect,
            Self::ObservationSubscribe { .. } => RuntimeFeature::ObservationSubscribe,
            Self::ContentRead { .. } => RuntimeFeature::ContentRead,
            Self::OutputRead { .. } => RuntimeFeature::OutputRead,
            Self::EvidenceRead { .. } => RuntimeFeature::EvidenceRead,
        }
    }

    #[must_use]
    pub fn request_id(&self) -> RequestId {
        match self {
            Self::ProgramInvocationInspect { context, .. }
            | Self::ObservationSubscribe { context, .. }
            | Self::ContentRead { context, .. }
            | Self::OutputRead { context, .. }
            | Self::EvidenceRead { context, .. } => context.request_id.clone(),
        }
    }
}

/// Closed service result set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeResult {
    /// Program Instance created against an artifact digest.
    ProgramInstanceCreated {
        /// Matching request id.
        request_id: String,
        /// Instance id.
        program_instance_id: String,
        /// Possession claim bound to this instance by the service.
        owner_claim: RuntimeOwnerClaim,
        /// Artifact digest that was admitted.
        artifact_digest: String,
        /// Runtime-owned profile available for the explicit bind step.
        #[serde(skip_serializing_if = "Option::is_none")]
        admission_profile: Option<RuntimeAdmissionProfileDescriptor>,
    },
    /// Exact Invocation Admission is now bound to the instance.
    ProgramInstanceAdmissionBound {
        request_id: String,
        program_instance_id: String,
        artifact_digest: String,
    },
    /// Invocation started or already waiting.
    ProgramInvocationStarted {
        /// Matching request id.
        request_id: String,
        /// Invocation id.
        program_invocation_id: String,
    },
    /// Event reserved with generation lineage.
    EventReserved {
        /// Matching request id.
        request_id: String,
        /// Possession claim bound to this reservation by the service.
        owner_claim: RuntimeOwnerClaim,
        /// Runtime-minted EventRef.
        event_ref: CanonicalEventRef,
    },
    /// Event application result from the root Event API.
    EventApplied {
        /// Matching request id.
        request_id: String,
        /// Application result.
        result: EventApplicationResult,
    },
    /// Authorized EventRef inspection.
    EventInspected {
        request_id: String,
        inspection: EventInspection,
    },
    /// Authorized pending EventRef listing.
    EventListed {
        request_id: String,
        events: Vec<EventInspection>,
    },
    /// Result of an explicit EventRef lifecycle mutation.
    EventLifecycleChanged {
        request_id: String,
        inspection: EventInspection,
    },
    /// One host-fulfilled Capability request reached a terminal settlement.
    CapabilitySettled {
        /// Matching request id.
        request_id: String,
        /// The request that settled.
        capability_request_id: String,
        /// How it settled.
        outcome: HostCapabilityOutcomeKind,
    },
    /// Invocation cancelled.
    Cancelled {
        /// Matching request id.
        request_id: String,
    },
    /// Typed protocol failure.
    Failed {
        /// Matching request id.
        request_id: String,
        /// Stable failure code.
        code: String,
    },
}

/// Additive Protocol/2 result set for read and observation methods.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeResultV2 {
    ObservationPage {
        request_id: RequestId,
        page: ExecutionPage<ExecutionObservation>,
    },
    ProgramInvocationInspection {
        request_id: RequestId,
        inspection: ProgramInvocationInspection,
    },
    NodeExecutionInspection {
        request_id: RequestId,
        inspection: NodeExecutionInspection,
    },
    Content {
        request_id: RequestId,
        content: ContentReadResult,
    },
    Output {
        request_id: RequestId,
        output: ContentReadResult,
    },
    EvidencePage {
        request_id: RequestId,
        page: ExecutionPage<EvidenceRecord>,
    },
    Failed {
        request_id: RequestId,
        code: RuntimeFailureCode,
    },
}

/// Protocol admission errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    /// Undeclared protocol version.
    IncompatibleVersion,
    /// Claim is absent or not the closed UUID-v4 wire form.
    InvalidOwnerClaim,
    /// A claim does not match the service-owned state.
    OwnerMismatch,
    /// An EventRef was not minted by this service or has a wrong generation.
    UnknownReservation,
    /// Request tried to submit source or compiler inputs as truth.
    SourceAsExecutable,
    /// Public method named a forbidden internal Event operation.
    ForbiddenEventMethod,
    /// The service-minted Event generation counter is exhausted.
    GenerationExhausted,
    /// Protocol/2 handshake schema identity does not match.
    SchemaMismatch,
    /// Protocol/2 handshake fields are internally inconsistent.
    InvalidHandshake,
    /// No common Protocol/2 read feature was negotiated.
    UnsupportedFeature,
    /// Protocol/2 request failed contract validation.
    InvalidRequest,
}

/// Methods the public protocol must never expose.
#[must_use]
pub fn public_event_method_forbidden(name: &str) -> bool {
    matches!(
        name,
        "event.bind_wait" | "event.consume_wake" | "resume_event"
    )
}

/// Whether a host fulfillment has the closed Runtime/1 wire shape.
///
/// Cancellation is a distinct request method. A fulfillment must therefore
/// name a runtime-minted request, exclude `cancelled`, and carry output only
/// when the outcome is `ok`.
#[must_use]
pub fn capability_fulfillment_is_well_formed(
    capability_request_id: &str,
    outcome: HostCapabilityOutcomeKind,
    output: Option<&str>,
) -> bool {
    is_host_capability_request_id(capability_request_id)
        && outcome != HostCapabilityOutcomeKind::Cancelled
        && (outcome == HostCapabilityOutcomeKind::Ok) == output.is_some()
}

/// In-memory Runtime Service peer used by protocol vectors.
#[derive(Clone)]
struct InMemoryEventApplication {
    event_ref: CanonicalEventRef,
    occurrence_id: String,
    payload: serde_json::Value,
}

impl InMemoryEventApplication {
    /// Compare every identity coordinate that makes an application replay-safe.
    fn matches(&self, application: &EventApplication<serde_json::Value>) -> bool {
        self.event_ref == application.event_ref
            && self.occurrence_id == application.occurrence.occurrence_id
            && self.payload == application.occurrence.payload
    }
}

#[derive(Default)]
pub struct InMemoryRuntimePeer {
    next_generation: u64,
    instances: HashMap<String, (RuntimeOwnerClaim, String)>,
    invocations: HashMap<String, InMemoryInvocation>,
    invocation_for_instance: HashMap<String, String>,
    admissions: HashMap<String, String>,
    reservations: HashMap<CanonicalEventRef, (RuntimeOwnerClaim, String)>,
    event_inspections: HashMap<CanonicalEventRef, EventInspection>,
    applications: HashMap<String, InMemoryEventApplication>,
}

struct InMemoryInvocation {
    request_id: String,
    owner_claim: RuntimeOwnerClaim,
    result: RuntimeResult,
}

impl InMemoryRuntimePeer {
    #[must_use]
    pub fn v2_handshake() -> RuntimeHandshakeV2 {
        RuntimeHandshakeV2::server()
    }

    /// Decode and negotiate the additive read surface without pretending to
    /// implement storage. Valid requests return a typed unavailable result
    /// until the runtime observation/read handlers land.
    pub fn handle_v2(
        &mut self,
        handshake: &RuntimeHandshakeV2,
        request: RuntimeRequestV2,
    ) -> Result<RuntimeResultV2, ProtocolError> {
        let negotiated = handshake.negotiate(&Self::v2_handshake())?;
        if !negotiated.contains(&request.feature()) {
            return Ok(RuntimeResultV2::Failed {
                request_id: request.request_id(),
                code: RuntimeFailureCode::UnsupportedFeature,
            });
        }
        request
            .validate()
            .map_err(|_| ProtocolError::InvalidRequest)?;
        Ok(RuntimeResultV2::Failed {
            request_id: request.request_id(),
            code: RuntimeFailureCode::Unavailable,
        })
    }

    fn change_event_status(
        &mut self,
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
        event_ref: CanonicalEventRef,
        status: EventStatus,
    ) -> Result<RuntimeResult, ProtocolError> {
        owner_claim.validate()?;
        event_ref
            .validate()
            .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
        let Some((expected_claim, _)) = self.reservations.get(&event_ref) else {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "unknown_reservation".to_owned(),
            });
        };
        if expected_claim != &owner_claim {
            return Err(ProtocolError::OwnerMismatch);
        }
        let inspection = self
            .event_inspections
            .get_mut(&event_ref)
            .expect("inspection");
        if inspection.status == EventStatus::Pending {
            inspection.status = status;
        }
        Ok(RuntimeResult::EventLifecycleChanged {
            request_id,
            inspection: inspection.clone(),
        })
    }

    /// Handle one admitted request. Artifact digest is the only executable truth.
    pub fn handle(
        &mut self,
        handshake: &RuntimeHandshake,
        request: RuntimeRequest,
    ) -> Result<RuntimeResult, ProtocolError> {
        handshake.admit()?;
        match request {
            RuntimeRequest::ProgramInstanceCreate {
                request_id,
                artifact_digest,
            } => {
                if !is_strict_digest(&artifact_digest) {
                    return Err(ProtocolError::SourceAsExecutable);
                }
                let id = format!("pi-{}", Uuid::new_v4());
                let owner_claim = RuntimeOwnerClaim::mint();
                self.instances
                    .insert(id.clone(), (owner_claim.clone(), artifact_digest.clone()));
                Ok(RuntimeResult::ProgramInstanceCreated {
                    request_id,
                    program_instance_id: id,
                    owner_claim,
                    artifact_digest,
                    admission_profile: None,
                })
            }
            RuntimeRequest::ProgramInvocationStart {
                request_id,
                program_instance_id,
                owner_claim,
                input: _,
            } => {
                owner_claim.validate()?;
                let Some((expected_claim, _)) = self.instances.get(&program_instance_id) else {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "unknown_instance".to_owned(),
                    });
                };
                if expected_claim != &owner_claim {
                    return Err(ProtocolError::OwnerMismatch);
                }
                if !self.admissions.contains_key(&program_instance_id) {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "missing_invocation_admission".to_owned(),
                    });
                }
                if let Some(invocation_id) = self.invocation_for_instance.get(&program_instance_id)
                {
                    let prior = self
                        .invocations
                        .get(invocation_id)
                        .expect("instance invocation index remains consistent");
                    if prior.request_id == request_id {
                        return Ok(prior.result.clone());
                    }
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "invocation_already_started".to_owned(),
                    });
                }
                let invocation_id = format!("{program_instance_id}:inv-{}", Uuid::new_v4());
                let result = RuntimeResult::ProgramInvocationStarted {
                    request_id: request_id.clone(),
                    program_invocation_id: invocation_id.clone(),
                };
                self.invocation_for_instance
                    .insert(program_instance_id, invocation_id.clone());
                self.invocations.insert(
                    invocation_id,
                    InMemoryInvocation {
                        request_id,
                        owner_claim,
                        result: result.clone(),
                    },
                );
                Ok(result)
            }
            RuntimeRequest::EventReserve {
                request_id,
                type_id,
            } => {
                if type_id.trim().is_empty() {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "empty_type".to_owned(),
                    });
                }
                self.next_generation = self
                    .next_generation
                    .checked_add(1)
                    .ok_or(ProtocolError::GenerationExhausted)?;
                let owner_claim = RuntimeOwnerClaim::mint();
                let event_ref = CanonicalEventRef {
                    event_id: format!("evt-{}", Uuid::new_v4()),
                    generation: self.next_generation,
                };
                self.reservations
                    .insert(event_ref.clone(), (owner_claim.clone(), type_id));
                self.event_inspections.insert(
                    event_ref.clone(),
                    EventInspection {
                        event_ref: event_ref.clone(),
                        type_id: self
                            .reservations
                            .get(&event_ref)
                            .map(|(_, type_id)| type_id.clone())
                            .expect("reservation just inserted"),
                        status: EventStatus::Pending,
                        occurrence_id: None,
                    },
                );
                Ok(RuntimeResult::EventReserved {
                    request_id,
                    owner_claim,
                    event_ref,
                })
            }
            RuntimeRequest::EventFulfill {
                request_id,
                owner_claim,
                application,
            } => {
                owner_claim.validate()?;
                application
                    .validate_identities()
                    .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
                let key = application.idempotency_key.clone();
                let Some((reservation_claim, _)) = self.reservations.get(&application.event_ref)
                else {
                    return Ok(RuntimeResult::EventApplied {
                        request_id,
                        result: EventApplicationResult::Rejected,
                    });
                };
                if reservation_claim != &owner_claim {
                    return Err(ProtocolError::OwnerMismatch);
                }
                if let Some(prior) = self.applications.get(&key) {
                    if !prior.matches(&application) {
                        return Ok(RuntimeResult::EventApplied {
                            request_id,
                            result: EventApplicationResult::Conflict,
                        });
                    }
                    return Ok(RuntimeResult::EventApplied {
                        request_id,
                        result: EventApplicationResult::Fulfilled,
                    });
                }
                if let Some(inspection) = self.event_inspections.get(&application.event_ref)
                    && inspection.status != EventStatus::Pending
                {
                    let result = match inspection.status {
                        EventStatus::Fulfilled => EventApplicationResult::Fulfilled,
                        EventStatus::Expired => EventApplicationResult::Expired,
                        EventStatus::Cancelled => EventApplicationResult::Cancelled,
                        EventStatus::Pending => unreachable!(),
                    };
                    return Ok(RuntimeResult::EventApplied { request_id, result });
                }
                self.applications.insert(
                    key,
                    InMemoryEventApplication {
                        event_ref: application.event_ref.clone(),
                        occurrence_id: application.occurrence.occurrence_id.clone(),
                        payload: application.occurrence.payload.clone(),
                    },
                );
                if let Some(inspection) = self.event_inspections.get_mut(&application.event_ref) {
                    inspection.status = EventStatus::Fulfilled;
                    inspection.occurrence_id = Some(application.occurrence.occurrence_id);
                }
                Ok(RuntimeResult::EventApplied {
                    request_id,
                    result: EventApplicationResult::Fulfilled,
                })
            }
            RuntimeRequest::EventList {
                request_id,
                owner_claim,
            } => {
                owner_claim.validate()?;
                let events = self
                    .event_inspections
                    .iter()
                    .filter(|(event_ref, inspection)| {
                        self.reservations
                            .get(*event_ref)
                            .is_some_and(|(claim, _)| claim == &owner_claim)
                            && inspection.status == EventStatus::Pending
                    })
                    .map(|(_, inspection)| inspection.clone())
                    .collect();
                Ok(RuntimeResult::EventListed { request_id, events })
            }
            RuntimeRequest::EventInspect {
                request_id,
                owner_claim,
                event_ref,
            } => {
                owner_claim.validate()?;
                event_ref
                    .validate()
                    .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
                let Some((expected_claim, _)) = self.reservations.get(&event_ref) else {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "unknown_reservation".to_owned(),
                    });
                };
                if expected_claim != &owner_claim {
                    return Err(ProtocolError::OwnerMismatch);
                }
                Ok(RuntimeResult::EventInspected {
                    request_id,
                    inspection: self
                        .event_inspections
                        .get(&event_ref)
                        .cloned()
                        .expect("inspection follows reservation"),
                })
            }
            RuntimeRequest::EventExpire {
                request_id,
                owner_claim,
                event_ref,
            } => self.change_event_status(request_id, owner_claim, event_ref, EventStatus::Expired),
            RuntimeRequest::EventCancel {
                request_id,
                owner_claim,
                event_ref,
            } => {
                self.change_event_status(request_id, owner_claim, event_ref, EventStatus::Cancelled)
            }
            RuntimeRequest::ProgramInvocationCancel {
                request_id,
                owner_claim,
                program_invocation_id,
            } => {
                owner_claim.validate()?;
                let Some(invocation) = self.invocations.get(&program_invocation_id) else {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "unknown_invocation".to_owned(),
                    });
                };
                if invocation.owner_claim != owner_claim {
                    return Err(ProtocolError::OwnerMismatch);
                }
                Ok(RuntimeResult::Cancelled { request_id })
            }
            // The vector peer states the shape of a settlement, not the park
            // it settles: it runs no driver, so it has no parked continuation
            // to wake. It admits a well-formed request and refuses one that is
            // not, which is exactly what a protocol vector proves.
            RuntimeRequest::CapabilityFulfill {
                request_id,
                owner_claim,
                capability_request_id,
                outcome,
                output,
                ..
            } => {
                owner_claim.validate()?;
                if !capability_fulfillment_is_well_formed(
                    &capability_request_id,
                    outcome,
                    output.as_deref(),
                ) {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "invalid_request".to_owned(),
                    });
                }
                Ok(RuntimeResult::CapabilitySettled {
                    request_id,
                    capability_request_id,
                    outcome,
                })
            }
            RuntimeRequest::CapabilityCancel {
                request_id,
                owner_claim,
                capability_request_id,
                ..
            } => {
                owner_claim.validate()?;
                if !is_host_capability_request_id(&capability_request_id) {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "invalid_request".to_owned(),
                    });
                }
                Ok(RuntimeResult::CapabilitySettled {
                    request_id,
                    capability_request_id,
                    outcome: HostCapabilityOutcomeKind::Cancelled,
                })
            }
        }
    }

    /// Handle the separately negotiated execution-admission envelope.
    pub fn handle_execution_admission(
        &mut self,
        handshake: &RuntimeExecutionAdmissionHandshake,
        request: RuntimeExecutionAdmissionRequest,
    ) -> Result<RuntimeResult, ProtocolError> {
        handshake.admit()?;
        match request {
            RuntimeExecutionAdmissionRequest::ProgramInstanceBindAdmission {
                request_id,
                program_instance_id,
                owner_claim,
                admission_profile_ref,
            } => {
                owner_claim.validate()?;
                let Some((expected_claim, artifact_digest)) =
                    self.instances.get(&program_instance_id)
                else {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "unknown_instance".to_owned(),
                    });
                };
                if expected_claim != &owner_claim {
                    return Err(ProtocolError::OwnerMismatch);
                }
                if !apxm_core::grammar::is_identifier(&admission_profile_ref) {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "invalid_invocation_admission".to_owned(),
                    });
                }
                if let Some(existing) = self.admissions.get(&program_instance_id)
                    && existing != &admission_profile_ref
                {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "admission_conflict".to_owned(),
                    });
                }
                self.admissions
                    .insert(program_instance_id.clone(), admission_profile_ref);
                Ok(RuntimeResult::ProgramInstanceAdmissionBound {
                    request_id,
                    program_instance_id,
                    artifact_digest: artifact_digest.clone(),
                })
            }
        }
    }
}

fn is_strict_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Map structural yield versus Event wait for protocol projection tests.
#[must_use]
pub fn invocation_boundary_for_await_event() -> InvocationBoundary {
    InvocationBoundary::EventWait
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_kernel::event_api::{EventOccurrence, EventWaitBinding};

    fn handshake() -> RuntimeHandshake {
        RuntimeHandshake {
            protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
        }
    }

    #[test]
    fn unknown_version_fails_closed() {
        let hs = RuntimeHandshake {
            protocol_version: "apxm.runtime.protocol/9".to_owned(),
        };
        assert_eq!(hs.admit(), Err(ProtocolError::IncompatibleVersion));
    }

    #[test]
    fn instance_create_rejects_empty_digest() {
        let mut peer = InMemoryRuntimePeer::default();
        let err = peer
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "r".to_owned(),
                    artifact_digest: " ".to_owned(),
                },
            )
            .unwrap_err();
        assert_eq!(err, ProtocolError::SourceAsExecutable);
    }

    #[test]
    fn fulfill_identical_retry_is_idempotent() {
        let mut peer = InMemoryRuntimePeer::default();
        let reserved = peer
            .handle(
                &handshake(),
                RuntimeRequest::EventReserve {
                    request_id: "r".to_owned(),
                    type_id: "UserInput".to_owned(),
                },
            )
            .unwrap();
        let RuntimeResult::EventReserved {
            event_ref,
            owner_claim,
            ..
        } = reserved
        else {
            panic!("reserve");
        };
        let application = EventApplication {
            event_ref,
            occurrence: EventOccurrence {
                occurrence_id: "occ".to_owned(),
                source_kind: "human.terminal".to_owned(),
                mapping_digest: "m".to_owned(),
                source_record: "s".to_owned(),
                payload: serde_json::json!({"text": "hi"}),
            },
            idempotency_key: "idem".to_owned(),
        };
        for _ in 0..2 {
            let result = peer
                .handle(
                    &handshake(),
                    RuntimeRequest::EventFulfill {
                        request_id: "f".to_owned(),
                        owner_claim: owner_claim.clone(),
                        application: application.clone(),
                    },
                )
                .unwrap();
            assert!(matches!(
                result,
                RuntimeResult::EventApplied {
                    result: EventApplicationResult::Fulfilled,
                    ..
                }
            ));
        }
    }

    #[test]
    fn conflicting_retry_is_conflict() {
        let mut peer = InMemoryRuntimePeer::default();
        let reserved = peer
            .handle(
                &handshake(),
                RuntimeRequest::EventReserve {
                    request_id: "r".to_owned(),
                    type_id: "UserInput".to_owned(),
                },
            )
            .unwrap();
        let RuntimeResult::EventReserved {
            event_ref,
            owner_claim,
            ..
        } = reserved
        else {
            panic!("reserve");
        };
        let mut application = EventApplication {
            event_ref,
            occurrence: EventOccurrence {
                occurrence_id: "occ".to_owned(),
                source_kind: "broker.queue".to_owned(),
                mapping_digest: "m".to_owned(),
                source_record: "s".to_owned(),
                payload: serde_json::json!(1),
            },
            idempotency_key: "idem".to_owned(),
        };
        peer.handle(
            &handshake(),
            RuntimeRequest::EventFulfill {
                request_id: "f1".to_owned(),
                owner_claim: owner_claim.clone(),
                application: application.clone(),
            },
        )
        .unwrap();
        let mut changed_occurrence = application.clone();
        changed_occurrence.occurrence.occurrence_id = "occ.other".to_owned();
        let result = peer
            .handle(
                &handshake(),
                RuntimeRequest::EventFulfill {
                    request_id: "f-occurrence".to_owned(),
                    owner_claim: owner_claim.clone(),
                    application: changed_occurrence,
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            RuntimeResult::EventApplied {
                result: EventApplicationResult::Conflict,
                ..
            }
        ));
        application.occurrence.payload = serde_json::json!(2);
        let result = peer
            .handle(
                &handshake(),
                RuntimeRequest::EventFulfill {
                    request_id: "f2".to_owned(),
                    owner_claim,
                    application,
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            RuntimeResult::EventApplied {
                result: EventApplicationResult::Conflict,
                ..
            }
        ));
    }

    #[test]
    fn conflicting_event_ref_retry_is_conflict() {
        let mut peer = InMemoryRuntimePeer::default();
        let reserve = |peer: &mut InMemoryRuntimePeer, request_id: &str| {
            peer.handle(
                &handshake(),
                RuntimeRequest::EventReserve {
                    request_id: request_id.to_owned(),
                    type_id: "UserInput".to_owned(),
                },
            )
            .unwrap()
        };
        let RuntimeResult::EventReserved {
            event_ref: first_event_ref,
            owner_claim: first_owner_claim,
            ..
        } = reserve(&mut peer, "r1")
        else {
            panic!("first reserve");
        };
        let RuntimeResult::EventReserved {
            event_ref: second_event_ref,
            owner_claim: second_owner_claim,
            ..
        } = reserve(&mut peer, "r2")
        else {
            panic!("second reserve");
        };
        let application = EventApplication {
            event_ref: first_event_ref,
            occurrence: EventOccurrence {
                occurrence_id: "occ".to_owned(),
                source_kind: "broker.queue".to_owned(),
                mapping_digest: "m".to_owned(),
                source_record: "s".to_owned(),
                payload: serde_json::json!(1),
            },
            idempotency_key: "idem".to_owned(),
        };
        peer.handle(
            &handshake(),
            RuntimeRequest::EventFulfill {
                request_id: "f1".to_owned(),
                owner_claim: first_owner_claim,
                application: application.clone(),
            },
        )
        .unwrap();

        let result = peer
            .handle(
                &handshake(),
                RuntimeRequest::EventFulfill {
                    request_id: "f2".to_owned(),
                    owner_claim: second_owner_claim,
                    application: EventApplication {
                        event_ref: second_event_ref,
                        ..application
                    },
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            RuntimeResult::EventApplied {
                result: EventApplicationResult::Conflict,
                ..
            }
        ));
    }

    #[test]
    fn forbidden_internal_event_methods_are_not_public() {
        assert!(public_event_method_forbidden("resume_event"));
        assert!(public_event_method_forbidden("event.bind_wait"));
        assert!(public_event_method_forbidden("event.consume_wake"));
        assert!(!public_event_method_forbidden("event.fulfill"));
    }

    #[test]
    fn capability_fulfillment_validation_excludes_cancellation() {
        let capability_request_id =
            apxm_core::types::host_capability::host_capability_request_id("invocation.1", "node.1");
        assert!(capability_fulfillment_is_well_formed(
            &capability_request_id,
            HostCapabilityOutcomeKind::Ok,
            Some("{}")
        ));
        assert!(!capability_fulfillment_is_well_formed(
            &capability_request_id,
            HostCapabilityOutcomeKind::Cancelled,
            None
        ));
        assert!(!capability_fulfillment_is_well_formed(
            &capability_request_id,
            HostCapabilityOutcomeKind::Ok,
            None
        ));
    }

    #[test]
    fn await_event_maps_to_event_wait_not_yield() {
        assert_eq!(
            invocation_boundary_for_await_event(),
            InvocationBoundary::EventWait
        );
        let binding = EventWaitBinding {
            program_invocation_id: "inv".to_owned(),
            event_ref: CanonicalEventRef {
                event_id: "evt".to_owned(),
                generation: 1,
            },
        };
        assert_eq!(binding.program_invocation_id, "inv");
    }

    #[test]
    fn mismatched_digest_does_not_start_an_invocation() {
        let mut peer = InMemoryRuntimePeer::default();
        let result = peer
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "s".to_owned(),
                    program_instance_id: "missing".to_owned(),
                    owner_claim: RuntimeOwnerClaim::mint(),
                    input: serde_json::json!({}),
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            RuntimeResult::Failed {
                code,
                ..
            } if code == "unknown_instance"
        ));
    }

    #[test]
    fn invocation_start_is_idempotent_and_owner_bound() {
        let mut peer = InMemoryRuntimePeer::default();
        let created = peer
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "c".to_owned(),
                    artifact_digest: format!("sha256:{}", "a".repeat(64)),
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        } = created
        else {
            panic!("create");
        };
        let start =
            |peer: &mut InMemoryRuntimePeer, owner_claim: RuntimeOwnerClaim, request_id: &str| {
                peer.handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationStart {
                        request_id: request_id.to_owned(),
                        program_instance_id: program_instance_id.clone(),
                        owner_claim,
                        input: serde_json::json!({"same": true}),
                    },
                )
            };
        let first = start(&mut peer, owner_claim.clone(), "s").unwrap();
        let retry = start(&mut peer, owner_claim.clone(), "s").unwrap();
        assert_eq!(first, retry);
        let other = start(&mut peer, RuntimeOwnerClaim::mint(), "other");
        assert!(matches!(other, Err(ProtocolError::OwnerMismatch)));
    }

    #[test]
    fn instance_admission_handoff_is_typed_owner_and_artifact_bound() {
        let mut peer = InMemoryRuntimePeer::default();
        let artifact_digest = format!("sha256:{}", "a".repeat(64));
        let created = peer
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "c".to_owned(),
                    artifact_digest: artifact_digest.clone(),
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        } = created
        else {
            panic!("create");
        };
        let bind = RuntimeExecutionAdmissionRequest::ProgramInstanceBindAdmission {
            request_id: "bind".to_owned(),
            program_instance_id: program_instance_id.clone(),
            owner_claim: owner_claim.clone(),
            admission_profile_ref: "apxm.admission-profile.test".to_owned(),
        };
        assert!(matches!(
            peer.handle_execution_admission(&RuntimeExecutionAdmissionHandshake::server(), bind)
                .unwrap(),
            RuntimeResult::ProgramInstanceAdmissionBound { .. }
        ));
        assert!(matches!(
            peer.handle_execution_admission(
                &RuntimeExecutionAdmissionHandshake::server(),
                RuntimeExecutionAdmissionRequest::ProgramInstanceBindAdmission {
                    request_id: "bind-retry".to_owned(),
                    program_instance_id: program_instance_id.clone(),
                    owner_claim: owner_claim.clone(),
                    admission_profile_ref: "apxm.admission-profile.test".to_owned(),
                },
            )
            .unwrap(),
            RuntimeResult::ProgramInstanceAdmissionBound { .. }
        ));
        assert!(matches!(
            peer.handle_execution_admission(
                &RuntimeExecutionAdmissionHandshake::server(),
                RuntimeExecutionAdmissionRequest::ProgramInstanceBindAdmission {
                    request_id: "bind-conflict".to_owned(),
                    program_instance_id,
                    owner_claim,
                    admission_profile_ref: "apxm.admission-profile.other".to_owned(),
                },
            )
            .unwrap(),
            RuntimeResult::Failed { code, .. } if code == "admission_conflict"
        ));
    }

    #[test]
    fn forged_event_ref_has_no_reservation_lineage() {
        let mut peer = InMemoryRuntimePeer::default();
        let result = peer
            .handle(
                &handshake(),
                RuntimeRequest::EventFulfill {
                    request_id: "f".to_owned(),
                    owner_claim: RuntimeOwnerClaim::mint(),
                    application: EventApplication {
                        event_ref: CanonicalEventRef {
                            event_id: "evt-forged".to_owned(),
                            generation: 7,
                        },
                        occurrence: EventOccurrence {
                            occurrence_id: "occ".to_owned(),
                            source_kind: "human.terminal".to_owned(),
                            mapping_digest: "m".to_owned(),
                            source_record: "s".to_owned(),
                            payload: serde_json::json!(null),
                        },
                        idempotency_key: "idem".to_owned(),
                    },
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            RuntimeResult::EventApplied {
                result: EventApplicationResult::Rejected,
                ..
            }
        ));
    }

    fn read_context(purpose: ReadPurpose) -> ReadContext {
        ReadContext {
            request_id: RequestId::new("request.1").expect("request id"),
            scope_ref: ScopeRef::new("scope.1").expect("scope ref"),
            principal_ref: PrincipalRef::new("principal.1").expect("principal ref"),
            grant_ref: GrantRef::new("grant.1").expect("grant ref"),
            correlation_id: Some(CorrelationId::new("correlation.1").expect("correlation id")),
            purpose,
        }
    }

    #[test]
    fn protocol_v2_is_separate_and_protocol_v1_stays_frozen() {
        assert_eq!(
            RuntimeHandshake {
                protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
            }
            .admit(),
            Ok(())
        );
        assert_eq!(RuntimeHandshakeV2::server().admit(), Ok(()));
        let mut skewed = RuntimeHandshakeV2::server();
        skewed.schema_digest = "sha256:wrong".to_owned();
        assert_eq!(skewed.admit(), Err(ProtocolError::SchemaMismatch));
    }

    #[test]
    fn execution_admission_is_not_decodable_as_protocol_or_read() {
        let request = RuntimeExecutionAdmissionRequest::ProgramInstanceBindAdmission {
            request_id: "bind".to_owned(),
            program_instance_id: "pi-1".to_owned(),
            owner_claim: RuntimeOwnerClaim::mint(),
            admission_profile_ref: "apxm.admission-profile.test".to_owned(),
        };
        let wire = serde_json::to_value(&request).expect("execution admission wire");
        assert!(serde_json::from_value::<RuntimeRequest>(wire.clone()).is_err());
        assert!(serde_json::from_value::<RuntimeRequestV2>(wire).is_err());
        assert_eq!(RuntimeExecutionAdmissionHandshake::server().admit(), Ok(()));
    }

    #[test]
    fn protocol_v2_read_wire_is_closed_and_validated() {
        let request = RuntimeRequestV2::ObservationSubscribe {
            context: read_context(ReadPurpose::Observation),
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("invocation"),
            after_cursor: Some(ExecutionCursor::new(4, "scope.cursor").expect("cursor")),
            limit: 50,
        };
        request.validate().expect("valid protocol v2 request");
        let value = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(value["method"], "observation.subscribe");
        let decoded: RuntimeRequestV2 = serde_json::from_value(value).expect("decode request");
        assert_eq!(decoded, request);

        let mut unknown = serde_json::to_value(request).expect("serialize request");
        unknown["method"] = serde_json::json!("retired_operation");
        assert!(serde_json::from_value::<RuntimeRequestV2>(unknown).is_err());
    }

    #[test]
    fn protocol_v2_rejects_mismatched_read_purpose_and_page_limit() {
        let request = RuntimeRequestV2::EvidenceRead {
            context: read_context(ReadPurpose::Content),
            program_invocation_id: ProgramInvocationId::new("invocation.1").expect("invocation"),
            after_cursor: None,
            limit: 1001,
        };
        assert_eq!(
            request.validate(),
            Err(ContractValidationError::InvalidReadPurpose)
        );
    }

    #[test]
    fn protocol_v2_output_read_negotiates_as_its_own_feature() {
        let request = RuntimeRequestV2::OutputRead {
            context: read_context(ReadPurpose::Output),
            output_ref: OutputRef::new("output.1").expect("output ref"),
        };
        request.validate().expect("valid output request");
        assert_eq!(request.feature(), RuntimeFeature::OutputRead);
        let value = serde_json::to_value(&request).expect("encode output request");
        assert_eq!(value["method"], "output.read");
        let decoded: RuntimeRequestV2 = serde_json::from_value(value).expect("decode");
        assert_eq!(decoded, request);
    }

    #[test]
    fn protocol_v2_peer_negotiates_and_fails_closed_until_storage_exists() {
        let mut peer = InMemoryRuntimePeer::default();
        let request = RuntimeRequestV2::ContentRead {
            context: read_context(ReadPurpose::Content),
            content_ref: ContentRef::new("content.1").expect("content"),
        };
        let result = peer
            .handle_v2(&RuntimeHandshakeV2::server(), request)
            .expect("negotiated request");
        assert!(matches!(
            result,
            RuntimeResultV2::Failed {
                code: RuntimeFailureCode::Unavailable,
                ..
            }
        ));

        let mut no_content = RuntimeHandshakeV2::server();
        no_content
            .supported_features
            .retain(|feature| *feature != RuntimeFeature::ContentRead);
        let request = RuntimeRequestV2::ContentRead {
            context: read_context(ReadPurpose::Content),
            content_ref: ContentRef::new("content.1").expect("content"),
        };
        assert!(matches!(
            peer.handle_v2(&no_content, request),
            Ok(RuntimeResultV2::Failed {
                code: RuntimeFailureCode::UnsupportedFeature,
                ..
            })
        ));
    }

    #[test]
    fn protocol_v2_failure_codes_are_closed() {
        let result = RuntimeResultV2::Failed {
            request_id: RequestId::new("request.1").expect("request"),
            code: RuntimeFailureCode::Unavailable,
        };
        let mut wire = serde_json::to_value(result).expect("encode failure");
        assert_eq!(wire["code"], "unavailable");
        wire["code"] = serde_json::json!("made_up");
        assert!(serde_json::from_value::<RuntimeResultV2>(wire).is_err());
    }
}
