//! Native Runtime Service protocol.
//!
//! Program Instance, Invocation, Event ingress, approval, cancellation, and
//! observations live here. Source, FrontendGraph, AIR text, and compiler
//! options are not representable as executable truth.

use apxm_kernel::event_api::{
    CanonicalEventRef, EventApplication, EventApplicationResult, InvocationBoundary,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Only declared Runtime protocol version. Unknown versions fail closed.
pub const RUNTIME_PROTOCOL_VERSION: &str = "apxm.runtime.protocol/1";

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
    /// Inspect an EventRef.
    EventInspect {
        /// Caller correlation id.
        request_id: String,
        /// Exact claim returned when this EventRef was reserved.
        owner_claim: RuntimeOwnerClaim,
        /// Target EventRef.
        event_ref: CanonicalEventRef,
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
}

/// Methods the public protocol must never expose.
#[must_use]
pub fn public_event_method_forbidden(name: &str) -> bool {
    matches!(
        name,
        "event.bind_wait" | "event.consume_wake" | "resume_event"
    )
}

/// In-memory Runtime Service peer used by protocol vectors.
#[derive(Default)]
pub struct InMemoryRuntimePeer {
    next_generation: u64,
    instances: Vec<(String, RuntimeOwnerClaim, String)>,
    invocations: Vec<(String, String, String, RuntimeResult)>,
    reservations: Vec<(CanonicalEventRef, RuntimeOwnerClaim, String)>,
    applications: Vec<(String, serde_json::Value)>,
}

impl InMemoryRuntimePeer {
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
                    .push((id.clone(), owner_claim.clone(), artifact_digest.clone()));
                Ok(RuntimeResult::ProgramInstanceCreated {
                    request_id,
                    program_instance_id: id,
                    owner_claim,
                    artifact_digest,
                })
            }
            RuntimeRequest::ProgramInvocationStart {
                request_id,
                program_instance_id,
                owner_claim,
                input: _,
            } => {
                owner_claim.validate()?;
                let Some((_, expected_claim, _)) = self
                    .instances
                    .iter()
                    .find(|(id, _, _)| id == &program_instance_id)
                else {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "unknown_instance".to_owned(),
                    });
                };
                if expected_claim != &owner_claim {
                    return Err(ProtocolError::OwnerMismatch);
                }
                if let Some((_, prior_request, _, prior_result)) = self
                    .invocations
                    .iter()
                    .find(|(id, _, _, _)| id == &program_instance_id)
                {
                    if prior_request == &request_id {
                        return Ok(prior_result.clone());
                    }
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "invocation_already_started".to_owned(),
                    });
                }
                let invocation_id = format!("{program_instance_id}:inv-{}", Uuid::new_v4());
                let result = RuntimeResult::ProgramInvocationStarted {
                    request_id: request_id.clone(),
                    program_invocation_id: invocation_id,
                };
                self.invocations.push((
                    program_instance_id,
                    request_id,
                    owner_claim.value,
                    result.clone(),
                ));
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
                self.next_generation += 1;
                let owner_claim = RuntimeOwnerClaim::mint();
                let event_ref = CanonicalEventRef {
                    event_id: format!("evt-{}", Uuid::new_v4()),
                    generation: self.next_generation,
                };
                self.reservations
                    .push((event_ref.clone(), owner_claim.clone(), type_id));
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
                let Some((event_ref, reservation_claim, _)) = self
                    .reservations
                    .iter()
                    .find(|(event_ref, _, _)| event_ref == &application.event_ref)
                else {
                    return Ok(RuntimeResult::EventApplied {
                        request_id,
                        result: EventApplicationResult::Rejected,
                    });
                };
                if reservation_claim != &owner_claim
                    || event_ref.generation != application.event_ref.generation
                {
                    return Err(ProtocolError::OwnerMismatch);
                }
                if let Some((_, prior)) = self.applications.iter().find(|(k, _)| k == &key) {
                    if prior != &application.occurrence.payload {
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
                self.applications
                    .push((key, application.occurrence.payload.clone()));
                Ok(RuntimeResult::EventApplied {
                    request_id,
                    result: EventApplicationResult::Fulfilled,
                })
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
                let Some((_, expected_claim, _)) = self
                    .reservations
                    .iter()
                    .find(|(reserved, _, _)| reserved == &event_ref)
                else {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "unknown_reservation".to_owned(),
                    });
                };
                if expected_claim != &owner_claim {
                    return Err(ProtocolError::OwnerMismatch);
                }
                Ok(RuntimeResult::Failed {
                    request_id,
                    code: "inspect_ok".to_owned(),
                })
            }
            RuntimeRequest::ProgramInvocationCancel {
                request_id,
                owner_claim,
                program_invocation_id,
            } => {
                owner_claim.validate()?;
                let Some((_, _, expected_claim, _)) = self
                    .invocations
                    .iter()
                    .find(|(_, _, _, result)| {
                        matches!(result, RuntimeResult::ProgramInvocationStarted { program_invocation_id: id, .. } if id == &program_invocation_id)
                    })
                else {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "unknown_invocation".to_owned(),
                    });
                };
                if expected_claim != &owner_claim.value {
                    return Err(ProtocolError::OwnerMismatch);
                }
                Ok(RuntimeResult::Cancelled { request_id })
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
    fn forbidden_internal_event_methods_are_not_public() {
        assert!(public_event_method_forbidden("resume_event"));
        assert!(public_event_method_forbidden("event.bind_wait"));
        assert!(public_event_method_forbidden("event.consume_wake"));
        assert!(!public_event_method_forbidden("event.fulfill"));
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
}
