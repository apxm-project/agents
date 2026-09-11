//! Kernel-owned runtime effect ports that are shared by every execution profile.
//!
//! These seams are intentionally transport-neutral. The kernel admits the
//! exact implementation and binding; the execution driver only dispatches the
//! already-admitted port. No host or product protocol is defined here.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The exact identity of a reserved Event or a host-owned Capability wait.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventRef {
    /// A reservation retains its generation across continuation persistence.
    Reserved {
        reference: crate::event_api::CanonicalEventRef,
    },
    /// Host Capability identity is minted by the admitted execution itself.
    HostCapability { request_id: String },
}

impl Serialize for EventRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Reserved { reference } => reference.serialize(serializer),
            Self::HostCapability { request_id } => request_id.serialize(serializer),
        }
    }
}

/// Reading a persisted continuation is where the wire form is load-bearing, so
/// the two admitted spellings are stated once, here, and nothing else is read.
///
/// A host Capability wait keeps the bare `capability-request.<digest>` string it
/// has always had, so every continuation a released cohort could actually resume
/// still loads. The other former spelling — a bare AIR operand `value_id` written
/// by an `await.event` on a declared Event — is deliberately no longer readable.
/// That break is accepted rather than bridged:
///
/// * A `value_id` names a compiled operand, not a destination, and carries no
///   generation. There is no reading of it that yields a correct
///   [`CanonicalEventRef`]; generation `1` would be a guess that can bind a wait
///   to the wrong occurrence, which is the exact failure this type exists to
///   make impossible.
/// * No released cohort could resume such a park in any case. Through
///   `chore(release): publish typed program service cohort` the only continuation
///   wake keyed on `event_ref` matched a host Capability request id exactly, so a
///   declared-Event park was already terminal — nothing to preserve compatibility
///   with, only a record that could never have been continued.
///
/// Refusing it at decode therefore turns a silently unresumable record into a
/// loud one, which is the behavior the durable path wants.
impl<'de> Deserialize<'de> for EventRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Reserved(crate::event_api::CanonicalEventRef),
            HostCapability(String),
        }
        match Wire::deserialize(deserializer)? {
            Wire::Reserved(reference) => Self::reserved(reference),
            Wire::HostCapability(request_id) => Self::new(request_id),
        }
        .map_err(serde::de::Error::custom)
    }
}

impl EventRef {
    /// Construct one host Capability wait identity.
    pub fn new(value: impl Into<String>) -> Result<Self, EventRefError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(EventRefError::Empty);
        }
        if !apxm_core::types::host_capability::is_host_capability_request_id(&value) {
            return Err(EventRefError::InvalidHostRequest);
        }
        Ok(Self::HostCapability { request_id: value })
    }

    /// Preserve the complete runtime-reserved target.
    pub fn reserved(reference: crate::event_api::CanonicalEventRef) -> Result<Self, EventRefError> {
        reference
            .validate()
            .map_err(|_| EventRefError::InvalidReservation)?;
        Ok(Self::Reserved { reference })
    }

    /// The full reserved destination, absent for a host Capability wait.
    pub fn reservation(&self) -> Option<&crate::event_api::CanonicalEventRef> {
        match self {
            Self::Reserved { reference } => Some(reference),
            Self::HostCapability { .. } => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Reserved { reference } => &reference.event_id,
            Self::HostCapability { request_id } => request_id,
        }
    }
}

impl std::fmt::Display for EventRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reserved { reference } => {
                write!(formatter, "{}:{}", reference.event_id, reference.generation)
            }
            Self::HostCapability { request_id } => formatter.write_str(request_id),
        }
    }
}

/// An invalid runtime event identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventRefError {
    Empty,
    InvalidHostRequest,
    InvalidReservation,
}

impl std::fmt::Display for EventRefError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("event reference must be non-empty"),
            Self::InvalidHostRequest => formatter.write_str(
                "a bare Event reference is a host Capability request id and nothing else; \
                 a declared Event names a reserved target and generation",
            ),
            Self::InvalidReservation => {
                formatter.write_str("event reservation identity or generation is invalid")
            }
        }
    }
}

impl std::error::Error for EventRefError {}

/// A request to await one external event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventAwait {
    pub node_id: String,
    pub node_execution_id: String,
    pub program_invocation_id: String,
    pub event_ref: EventRef,
    pub contract: EventWaitContract,
}

/// Static declaration requirements remain separate from a runtime destination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventWaitContract {
    HostCapability,
    Declared {
        type_id: String,
        schema_digest: String,
    },
}

/// The typed outcome of an `await.event` effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventOutcome {
    Fulfilled {
        event_ref: EventRef,
        payload: String,
    },
    Parked,
    Expired,
    Cancelled,
    Mismatched {
        delivered_event_ref: EventRef,
    },
    Rejected {
        reason: EventWaitRejection,
    },
}

/// Closed refusal of a wait before its payload can enter execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventWaitRejection {
    UnknownReference,
    OwnerMismatch,
    ContractMismatch,
    AlreadyBound,
    InvalidPayload,
}

/// A terminal Event delivery is distinct from ordinary source JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventDelivery {
    Fulfilled { payload: serde_json::Value },
    Expired,
    Cancelled,
}

/// The admitted port for durable `await.event` effects.
#[async_trait]
pub trait EventPort: Send + Sync {
    async fn await_event(&self, request: EventAwait) -> EventOutcome;

    /// Apply an admitted occurrence. The default rejects so a host cannot inject
    /// a raw continuation value through this trait by accident.
    async fn apply_occurrence(
        &self,
        application: crate::event_api::EventApplication<String>,
    ) -> crate::event_api::EventApplicationResult {
        let _ = application;
        crate::event_api::EventApplicationResult::Rejected
    }
}

/// The typed receiver of a Program composition operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionReceiver {
    Program { program_ref: String },
    Instance { program_instance_ref: String },
}

impl CompositionReceiver {
    /// The exact reference carried by this receiver.
    pub fn reference(&self) -> &str {
        match self {
            Self::Program { program_ref } => program_ref,
            Self::Instance {
                program_instance_ref,
            } => program_instance_ref,
        }
    }

    #[must_use]
    pub fn is_instance(&self) -> bool {
        matches!(self, Self::Instance { .. })
    }
}

/// A request to compose a child Program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionRequest {
    pub node_id: String,
    pub receiver: CompositionReceiver,
}

/// The typed outcome of composing or invoking a child Program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionOutcome {
    Created { child_instance_ref: String },
    Invoked { child_instance_ref: String },
    Failed { message: String },
}

/// The admitted port for Program creation and invocation.
#[async_trait]
pub trait CompositionPort: Send + Sync {
    async fn program_new(&self, request: CompositionRequest) -> CompositionOutcome;
    async fn program_invoke(&self, request: CompositionRequest) -> CompositionOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_reference_retains_canonical_reserved_and_existing_host_wire_forms() {
        let host = format!("capability-request.{}", "a".repeat(64));
        let reference: EventRef = serde_json::from_value(json!(host)).unwrap();
        assert_eq!(reference, EventRef::new(&host).unwrap());
        assert_eq!(serde_json::to_value(reference).unwrap(), json!(host));

        let wire = json!({"event_id":"event.1","generation":3});
        let reference: EventRef = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(reference.reservation().unwrap().generation, 3);
        assert_eq!(serde_json::to_value(reference).unwrap(), wire);
    }

    #[test]
    fn event_reference_rejects_arbitrary_strings_and_incomplete_reservations() {
        for value in [
            json!("event.1"),
            json!("capability-request.invalid"),
            json!({"event_id":"event.1"}),
            json!({"event_id":"event.1","generation":0}),
            json!({"event_id":"event.1","generation":9007199254740992u64}),
            json!({"event_id":"event.1","generation":1,"unexpected":true}),
            json!({"kind":"host_capability","request_id":format!("capability-request.{}", "a".repeat(64))}),
        ] {
            assert!(
                serde_json::from_value::<EventRef>(value.clone()).is_err(),
                "{value}"
            );
        }
        assert!(
            serde_json::from_str::<EventRef>(
                r#"{"event_id":"event.1","generation":1,"generation":2}"#
            )
            .is_err()
        );
    }

    /// The accepted compatibility break, stated as a verdict rather than prose.
    /// A continuation a released cohort could resume still loads; the bare AIR
    /// `value_id` an `await.event` used to persist does not, because no reading
    /// of it produces a generation and none of those parks was ever resumable.
    #[test]
    fn only_the_resumable_persisted_form_survives_the_reference_change() {
        let settled = format!("capability-request.{}", "b".repeat(64));
        assert!(serde_json::from_value::<EventRef>(json!(settled)).is_ok());
        for parked_operand_value_id in ["Event.param.input", "event.submitted", "await.event.0"] {
            let refusal = serde_json::from_value::<EventRef>(json!(parked_operand_value_id))
                .expect_err(parked_operand_value_id)
                .to_string();
            assert!(
                refusal.contains("reserved target and generation"),
                "{refusal}"
            );
        }
    }
}
