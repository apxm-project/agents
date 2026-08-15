//! Canonical Event type family owned by the runtime kernel.
//!
//! Event type, EventRef, source occurrence, target application, source
//! contract, and wait binding are distinct identities. Source kind is
//! provenance on an occurrence; the reducer must not branch on it.

use serde::{Deserialize, Serialize};

/// Frozen Event contract identity. Unknown names fail closed.
pub const EVENT_CONTRACT: &str = "apxm.event";

/// Authored Event type / source requirement. Never used as a wait identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventTypeRef {
    /// Namespaced Event type id from Program source.
    pub type_id: String,
}

/// Runtime-minted destination with generation and reservation lineage.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalEventRef {
    /// Opaque reserved identity. Not a declaration name and not a source id.
    pub event_id: String,
    /// Ownership generation. Wrong generation fails closed.
    pub generation: u64,
}

/// Source-side observation before target application.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventOccurrence<T> {
    /// Idempotent occurrence identity, distinct from the EventRef.
    pub occurrence_id: String,
    /// Namespaced source kind descriptor. Never a reducer discriminant.
    pub source_kind: String,
    /// Digest of the admitted source-contract mapping.
    pub mapping_digest: String,
    /// Source-stable record or sequence identity.
    pub source_record: String,
    /// Typed payload after source-contract reduction.
    pub payload: T,
}

/// Authorized application of one occurrence onto one EventRef.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventApplication<T> {
    /// Target EventRef, including generation.
    pub event_ref: CanonicalEventRef,
    /// Occurrence being applied. Fan-out uses one application per target.
    pub occurrence: EventOccurrence<T>,
    /// Caller idempotency key for this application.
    pub idempotency_key: String,
}

/// Wait binding recorded when `await.event` parks an Invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventWaitBinding {
    /// Invocation that remains `WaitingEvent` until exact wake.
    pub program_invocation_id: String,
    /// EventRef the wait consumes. Declaration ids are rejected.
    pub event_ref: CanonicalEventRef,
}

/// Closed application result. Pre-admission drops are not Event terminals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventApplicationResult {
    /// First terminal wins; later identical retries replay this result.
    Fulfilled,
    /// Conflicting payload or generation against a committed terminal.
    Conflict,
    /// Target expired before fulfillment.
    Expired,
    /// Target cancelled before fulfillment.
    Cancelled,
    /// Schema, owner, callsite, generation, or authority mismatch.
    Rejected,
}

/// Frozen Event HTTP route names. Framing lives in a later edge crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventHttpMethod {
    /// POST reserve.
    Reserve,
    /// GET inspect.
    Inspect,
    /// POST fulfill.
    Fulfill,
    /// POST expire.
    Expire,
    /// POST cancel.
    Cancel,
}

impl EventHttpMethod {
    /// HTTP path fragment for the method.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Reserve => "/v1/events/reserve",
            Self::Inspect => "/v1/events/inspect",
            Self::Fulfill => "/v1/events/fulfill",
            Self::Expire => "/v1/events/expire",
            Self::Cancel => "/v1/events/cancel",
        }
    }
}

/// Yield versus Event-wait lifecycle mapping frozen for protocol projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvocationBoundary {
    /// Structural yield commits output and `CommittedYield`.
    StructuralYield,
    /// Event wait parks the same Invocation in `WaitingEvent`.
    EventWait,
}

impl CanonicalEventRef {
    /// Reject empty ids. Reservation lineage is proven by later phases.
    pub fn validate(&self) -> Result<(), EventContractError> {
        if self.event_id.trim().is_empty() {
            return Err(EventContractError::EmptyRef);
        }
        Ok(())
    }
}

impl<T> EventApplication<T> {
    /// Reject empty identities that would collapse occurrence and target.
    pub fn validate_identities(&self) -> Result<(), EventContractError> {
        self.event_ref.validate()?;
        if self.occurrence.occurrence_id.trim().is_empty() || self.idempotency_key.trim().is_empty()
        {
            return Err(EventContractError::IncompleteApplication);
        }
        if self.occurrence.occurrence_id == self.event_ref.event_id {
            return Err(EventContractError::CollapsedIdentity);
        }
        Ok(())
    }
}

/// Contract-level Event identity errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventContractError {
    /// EventRef identity is empty.
    EmptyRef,
    /// Application is missing occurrence or idempotency identity.
    IncompleteApplication,
    /// Occurrence id was reused as the EventRef.
    CollapsedIdentity,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occurrence(payload: &str) -> EventOccurrence<String> {
        EventOccurrence {
            occurrence_id: "occ-1".to_owned(),
            source_kind: "human.terminal".to_owned(),
            mapping_digest: "map".to_owned(),
            source_record: "rec-1".to_owned(),
            payload: payload.to_owned(),
        }
    }

    #[test]
    fn occurrence_and_event_ref_stay_distinct() {
        let application = EventApplication {
            event_ref: CanonicalEventRef {
                event_id: "evt-1".to_owned(),
                generation: 1,
            },
            occurrence: occurrence("hello"),
            idempotency_key: "idem-1".to_owned(),
        };
        application.validate_identities().unwrap();
    }

    #[test]
    fn collapsed_occurrence_and_ref_fail_closed() {
        let application = EventApplication {
            event_ref: CanonicalEventRef {
                event_id: "same".to_owned(),
                generation: 1,
            },
            occurrence: EventOccurrence {
                occurrence_id: "same".to_owned(),
                source_kind: "broker.queue".to_owned(),
                mapping_digest: "map".to_owned(),
                source_record: "rec".to_owned(),
                payload: "x".to_owned(),
            },
            idempotency_key: "idem".to_owned(),
        };
        assert_eq!(
            application.validate_identities(),
            Err(EventContractError::CollapsedIdentity)
        );
    }

    #[test]
    fn source_kind_is_not_a_result_variant() {
        let kinds = [
            "human.terminal",
            "http.cloudevents",
            "broker.queue",
            "schedule.timer",
            "hook.capability",
            "iot.sensor",
        ];
        for kind in kinds {
            let mut occ = occurrence("p");
            occ.source_kind = kind.to_owned();
            let application = EventApplication {
                event_ref: CanonicalEventRef {
                    event_id: "evt".to_owned(),
                    generation: 1,
                },
                occurrence: occ,
                idempotency_key: "k".to_owned(),
            };
            application.validate_identities().unwrap();
        }
    }
}
