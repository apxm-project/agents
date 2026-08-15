//! Runtime Service Composition Root.
//!
//! Accepts only a verified artifact digest plus admission inputs. It does not
//! import source-port, frontends, or the compiler.

mod composition;
mod ports;
mod stdio;

pub use composition::{
    AdmittedPackageHandlers, ArtifactStore, CanonicalRuntimeDescriptor, InvocationMaterials,
    PackageHandlerWorkerCommand, artifact_digest, canonical_port_bindings_digest,
    canonical_resource_ceiling_digest, canonical_runtime_descriptor, execute_admitted_artifact,
    materials_for_artifact,
};
pub use stdio::{
    StdioFrame, UnixEndpoint, decode_jsonl, encode_jsonl, handshake_cross_wired, serve_stdio,
};

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;

use apxm_kernel::event_api::{CanonicalEventRef, EventApplication, EventApplicationResult};
use apxm_program::air::AirModule;
use apxm_runtime_protocol::{ProtocolError, RuntimeHandshake, RuntimeRequest, RuntimeResult};
use serde_json::Value;

/// Runtime Service handler over the native protocol.
#[derive(Default)]
pub struct RuntimeService {
    artifacts: ArtifactStore,
    instances: BTreeMap<String, InstanceState>,
    applications: Vec<(String, Value)>,
    next_generation: u64,
    last_output: Option<Value>,
    handlers: Option<AdmittedPackageHandlers>,
    package_root: Option<PathBuf>,
}

struct InstanceState {
    artifact_digest: String,
    materials: Option<InvocationMaterials>,
}

impl RuntimeService {
    /// Bind package-local Capability handlers for subsequent invocations.
    pub fn bind_package(
        &mut self,
        handlers: Option<AdmittedPackageHandlers>,
        package_root: Option<PathBuf>,
    ) {
        self.handlers = handlers;
        self.package_root = package_root;
    }

    /// Commit AIR/artifact bytes. The digest is the only executable identity.
    pub fn admit_artifact(&mut self, bytes: Vec<u8>) -> String {
        self.artifacts.commit(bytes)
    }

    /// Bind invocation admission materials to an instance created from a digest.
    pub fn bind_admission(
        &mut self,
        program_instance_id: &str,
        materials: InvocationMaterials,
    ) -> Result<(), String> {
        let instance = self
            .instances
            .get_mut(program_instance_id)
            .ok_or_else(|| "unknown_instance".to_owned())?;
        if materials.admission.artifact_digest != instance.artifact_digest {
            return Err("artifact_digest_mismatch".to_owned());
        }
        instance.materials = Some(materials);
        Ok(())
    }

    /// Last committed invocation output, when one completed.
    #[must_use]
    pub fn last_output(&self) -> Option<&Value> {
        self.last_output.as_ref()
    }

    /// Artifact bytes previously admitted under `digest`.
    #[must_use]
    pub fn artifact_bytes(&self, digest: &str) -> Option<&[u8]> {
        self.artifacts.get(digest)
    }

    /// Admit a handshake and request. Source is not executable truth.
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
            } => self.create_instance(request_id, artifact_digest),
            RuntimeRequest::ProgramInvocationStart {
                request_id,
                program_instance_id,
                input: _,
            } => Ok(self.start_invocation(request_id, program_instance_id)),
            RuntimeRequest::EventReserve {
                request_id,
                type_id,
            } => self.reserve_event(request_id, type_id),
            RuntimeRequest::EventFulfill {
                request_id,
                application,
            } => self.fulfill_event(request_id, application),
            RuntimeRequest::EventInspect {
                request_id,
                event_ref,
            } => self.inspect_event(request_id, event_ref),
            RuntimeRequest::ProgramInvocationCancel { request_id, .. } => {
                Ok(RuntimeResult::Cancelled { request_id })
            }
        }
    }
}

impl RuntimeService {
    fn create_instance(
        &mut self,
        request_id: String,
        artifact_digest: String,
    ) -> Result<RuntimeResult, ProtocolError> {
        if artifact_digest.trim().is_empty() {
            return Err(ProtocolError::SourceAsExecutable);
        }
        if self.artifacts.get(&artifact_digest).is_none() {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "unknown_artifact".to_owned(),
            });
        }
        let id = format!("pi-{}", self.instances.len() + 1);
        self.instances.insert(
            id.clone(),
            InstanceState {
                artifact_digest: artifact_digest.clone(),
                materials: None,
            },
        );
        Ok(RuntimeResult::ProgramInstanceCreated {
            request_id,
            program_instance_id: id,
            artifact_digest,
        })
    }

    fn start_invocation(
        &mut self,
        request_id: String,
        program_instance_id: String,
    ) -> RuntimeResult {
        let Some(instance) = self.instances.get(&program_instance_id) else {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_instance".to_owned(),
            };
        };
        let Some(bytes) = self.artifacts.get(&instance.artifact_digest) else {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_artifact".to_owned(),
            };
        };
        let bytes = bytes.to_vec();
        let air = match serde_json::from_slice::<AirModule>(&bytes) {
            Ok(air) => air,
            Err(_) => {
                return RuntimeResult::Failed {
                    request_id,
                    code: "invalid_artifact".to_owned(),
                };
            }
        };
        let materials = match &instance.materials {
            Some(materials) => InvocationMaterials {
                admission: materials.admission.clone(),
                release_bytes: materials.release_bytes.clone(),
                provenance_bytes: materials.provenance_bytes.clone(),
            },
            None => materials_for_artifact(
                &bytes,
                format!("{program_instance_id}:inv-1"),
                b"{}".to_vec(),
                b"{}".to_vec(),
            ),
        };
        match block_on(execute_admitted_artifact(
            air,
            &bytes,
            &materials,
            self.handlers.as_ref(),
            self.package_root.as_deref(),
        )) {
            Ok(output) => {
                self.last_output = Some(output);
                RuntimeResult::ProgramInvocationStarted {
                    request_id,
                    program_invocation_id: format!("{program_instance_id}:inv-1"),
                }
            }
            Err(code) => RuntimeResult::Failed { request_id, code },
        }
    }

    fn reserve_event(
        &mut self,
        request_id: String,
        type_id: String,
    ) -> Result<RuntimeResult, ProtocolError> {
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

    fn fulfill_event(
        &mut self,
        request_id: String,
        application: EventApplication<Value>,
    ) -> Result<RuntimeResult, ProtocolError> {
        application
            .validate_identities()
            .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
        let key = application.idempotency_key.clone();
        if let Some((_, prior)) = self.applications.iter().find(|(item, _)| item == &key) {
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
        // Event application is the only wake authority. A fulfill with no
        // parked continuation still records the application; it does not
        // invent a successful invocation finish.
        let _ = wake_authority(&application);
        Ok(RuntimeResult::EventApplied {
            request_id,
            result: EventApplicationResult::Fulfilled,
        })
    }

    fn inspect_event(
        &mut self,
        request_id: String,
        event_ref: CanonicalEventRef,
    ) -> Result<RuntimeResult, ProtocolError> {
        event_ref
            .validate()
            .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
        Ok(RuntimeResult::Failed {
            request_id,
            code: "inspect_ok".to_owned(),
        })
    }
}

/// Prove this crate does not name a source-port type in its public API.
#[must_use]
pub fn accepts_source_packages() -> bool {
    false
}

fn wake_authority(application: &EventApplication<Value>) -> EventApplicationResult {
    // The public protocol never calls resume_event. Wake authority is the
    // fulfilled application record itself; callers that later bind a parked
    // continuation must pass this same result into wake_from_event_application.
    let _ = application;
    EventApplicationResult::Fulfilled
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime service tokio runtime")
            .block_on(future),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_kernel::event_api::{CanonicalEventRef, EventApplication, EventOccurrence};
    use apxm_runtime_protocol::{RUNTIME_PROTOCOL_VERSION, RuntimeRequest};
    use std::fs;
    use std::path::PathBuf;

    fn handshake() -> RuntimeHandshake {
        RuntimeHandshake {
            protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
        }
    }

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("runtime-service sits three levels under the repository root")
            .join("tools/tests/fixtures")
    }

    fn fixture_air_bytes() -> Vec<u8> {
        fs::read(fixture_dir().join("canonical-execute.air.json")).expect("fixture AIR")
    }

    #[test]
    fn artifact_create_then_invoke() {
        let mut service = RuntimeService::default();
        let digest = service.admit_artifact(fixture_air_bytes());
        let created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "c".to_owned(),
                    artifact_digest: digest,
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
        let admission: apxm_kernel::InvocationAdmission = serde_json::from_slice(
            &fs::read(fixture_dir().join("canonical-execute.invocation-admission.json")).unwrap(),
        )
        .unwrap();
        let release = fs::read(fixture_dir().join("canonical-execute.release.json")).unwrap();
        let provenance = fs::read(fixture_dir().join("canonical-execute.provenance.json")).unwrap();
        service
            .bind_admission(
                &program_instance_id,
                InvocationMaterials {
                    admission,
                    release_bytes: release,
                    provenance_bytes: provenance,
                },
            )
            .unwrap();
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
        let output = service.last_output().expect("committed output");
        assert_eq!(output["schema_version"], "apxm.local-execute-result");
        assert_eq!(output["status"], "completed");
        assert!(output["results"]["node_outcomes"].as_array().is_some());
    }

    #[test]
    fn empty_digest_is_source_as_executable() {
        let mut service = RuntimeService::default();
        let err = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "c".to_owned(),
                    artifact_digest: String::new(),
                },
            )
            .unwrap_err();
        assert_eq!(err, ProtocolError::SourceAsExecutable);
    }

    #[test]
    fn unknown_digest_does_not_create_an_instance() {
        let mut service = RuntimeService::default();
        let result = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "c".to_owned(),
                    artifact_digest: "sha256:deadbeef".to_owned(),
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            RuntimeResult::Failed { code, .. } if code == "unknown_artifact"
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
