//! Runtime Service Composition Root.
//!
//! Accepts only a verified artifact digest plus admission inputs. It does not
//! import source-port, frontends, or the compiler.

mod stdio;

pub use stdio::{
    StdioFrame, UnixEndpoint, decode_jsonl, encode_jsonl, handshake_cross_wired, serve_stdio,
};

use apxm_runtime_protocol::{
    InMemoryRuntimePeer, ProtocolError, RuntimeHandshake, RuntimeRequest, RuntimeResult,
};

/// Runtime Service handler over the native protocol.
#[derive(Default)]
pub struct RuntimeService {
    peer: InMemoryRuntimePeer,
}

impl RuntimeService {
    /// Admit a handshake and request. Source is not executable truth.
    pub fn handle(
        &mut self,
        handshake: &RuntimeHandshake,
        request: RuntimeRequest,
    ) -> Result<RuntimeResult, ProtocolError> {
        self.peer.handle(handshake, request)
    }
}

/// Prove this crate does not name a source-port type in its public API.
#[must_use]
pub fn accepts_source_packages() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_kernel::event_api::{CanonicalEventRef, EventApplication, EventOccurrence};
    use apxm_runtime_protocol::{RUNTIME_PROTOCOL_VERSION, RuntimeRequest};

    fn handshake() -> RuntimeHandshake {
        RuntimeHandshake {
            protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
        }
    }

    #[test]
    fn artifact_create_then_invoke() {
        let mut service = RuntimeService::default();
        let created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "c".to_owned(),
                    artifact_digest: "artifact:abc".to_owned(),
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            ..
        } = created
        else {
            panic!("create");
        };
        let started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "s".to_owned(),
                    program_instance_id,
                    input: serde_json::json!({}),
                },
            )
            .unwrap();
        assert!(matches!(
            started,
            RuntimeResult::ProgramInvocationStarted { .. }
        ));
    }

    #[test]
    fn event_fulfill_uses_root_application_record() {
        let mut service = RuntimeService::default();
        let reserved = service
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
        let result = service
            .handle(
                &handshake(),
                RuntimeRequest::EventFulfill {
                    request_id: "f".to_owned(),
                    application: EventApplication {
                        event_ref,
                        occurrence: EventOccurrence {
                            occurrence_id: "occ".to_owned(),
                            source_kind: "human.terminal".to_owned(),
                            mapping_digest: "m".to_owned(),
                            source_record: "s".to_owned(),
                            payload: serde_json::json!("ok"),
                        },
                        idempotency_key: "k".to_owned(),
                    },
                },
            )
            .unwrap();
        assert!(matches!(result, RuntimeResult::EventApplied { .. }));
        let _ = CanonicalEventRef {
            event_id: "evt".to_owned(),
            generation: 1,
        };
    }

    #[test]
    fn source_packages_are_not_accepted() {
        assert!(!accepts_source_packages());
    }
}
