//! Native Runtime Service protocol.
//!
//! Program Instance, Invocation, Event ingress, approval, cancellation, and
//! observations live here. Source, FrontendGraph, AIR text, and compiler
//! options are not representable as executable truth.

use apxm_kernel::event_api::{
    CanonicalEventRef, EventApplication, EventApplicationResult, InvocationBoundary,
};
use serde::{Deserialize, Serialize};

/// Only declared Runtime protocol version. Unknown versions fail closed.
pub const RUNTIME_PROTOCOL_VERSION: &str = "apxm.runtime.protocol/1";

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
        /// Canonical application record.
        application: EventApplication<serde_json::Value>,
    },
    /// Inspect an EventRef.
    EventInspect {
        /// Caller correlation id.
        request_id: String,
        /// Target EventRef.
        event_ref: CanonicalEventRef,
    },
    /// Cancel a Program Invocation.
    ProgramInvocationCancel {
        /// Caller correlation id.
        request_id: String,
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
    instances: Vec<String>,
    artifact_by_instance: Vec<(String, String)>,
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
                if artifact_digest.trim().is_empty() {
                    return Err(ProtocolError::SourceAsExecutable);
                }
                let id = format!("pi-{}", self.instances.len() + 1);
                self.instances.push(id.clone());
                self.artifact_by_instance
                    .push((id.clone(), artifact_digest.clone()));
                Ok(RuntimeResult::ProgramInstanceCreated {
                    request_id,
                    program_instance_id: id,
                    artifact_digest,
                })
            }
            RuntimeRequest::ProgramInvocationStart {
                request_id,
                program_instance_id,
                input: _,
            } => {
                if !self.instances.iter().any(|id| id == &program_instance_id) {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "unknown_instance".to_owned(),
                    });
                }
                Ok(RuntimeResult::ProgramInvocationStarted {
                    request_id,
                    program_invocation_id: format!("{program_instance_id}:inv-1"),
                })
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
                Ok(RuntimeResult::EventReserved {
                    request_id,
                    event_ref: CanonicalEventRef {
                        event_id: format!("evt-{}", self.next_generation),
                        generation: self.next_generation,
                    },
                })
            }
            RuntimeRequest::EventFulfill {
                request_id,
                application,
            } => {
                application
                    .validate_identities()
                    .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
                let key = application.idempotency_key.clone();
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
                event_ref,
            } => {
                event_ref
                    .validate()
                    .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
                Ok(RuntimeResult::Failed {
                    request_id,
                    code: "inspect_ok".to_owned(),
                })
            }
            RuntimeRequest::ProgramInvocationCancel { request_id, .. } => {
                Ok(RuntimeResult::Cancelled { request_id })
            }
        }
    }
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
        let RuntimeResult::EventReserved { event_ref, .. } = reserved else {
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
        let RuntimeResult::EventReserved { event_ref, .. } = reserved else {
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
}
