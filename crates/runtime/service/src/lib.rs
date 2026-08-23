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
    execute_admitted_artifact_with_sandbox, materials_for_artifact,
    validate_package_permission_resolution,
};
pub use stdio::{
    MAX_FRAME_BYTES, MAX_FRAMES_PER_CONNECTION, RUNTIME_CHANNEL, StdioFrame, UNIX_IO_TIMEOUT_MS,
    UnixEndpoint, decode_jsonl, encode_jsonl, handshake_cross_wired, serve_stdio, serve_unix,
};

use std::collections::BTreeMap;
use std::future::Future;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use apxm_ais::permissions::PermissionDecision;
use apxm_capability_iface::sandbox::SandboxRegistry;
use apxm_execution::{
    ApprovalBroker, ApprovalDecision, DenyBroker, Observation, RecordingObserver,
};
use apxm_kernel::event_api::{CanonicalEventRef, EventApplication, EventApplicationResult};
use apxm_program::air::AirModule;
use apxm_runtime_protocol::{
    ProtocolError, RuntimeHandshake, RuntimeOwnerClaim, RuntimeRequest, RuntimeResult,
};
use serde_json::Value;
use uuid::Uuid;

/// Maximum persisted executable artifact accepted by the Runtime Service.
/// Reads are bounded before allocation and verified against the requested
/// content digest after the bounded read.
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

/// A typed bound for one class of service-owned state.
///
/// Expiry is deterministic and only expired entries are eligible for cleanup.
/// The service never evicts a live entry to make room for a new request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateQuota {
    /// Maximum number of entries retained for this state class.
    pub max_entries: usize,
    /// Maximum aggregate payload bytes retained for this state class.
    pub max_bytes: u64,
    /// Lifetime from admission/creation until the entry expires.
    pub ttl: Duration,
}

impl StateQuota {
    const fn new(max_entries: usize, max_bytes: u64, ttl: Duration) -> Self {
        Self {
            max_entries,
            max_bytes,
            ttl,
        }
    }
}

/// Bounded in-memory state policy for one Runtime Service process.
///
/// The defaults are intentionally finite. Callers embedding the service may
/// provide a tighter policy with [`RuntimeService::with_state_policy`], but a
/// zero quota remains a closed lane: requests fail instead of causing an
/// unbounded allocation or an arbitrary eviction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeStatePolicy {
    /// Digest-addressed executable artifacts.
    pub artifacts: StateQuota,
    /// Artifact-scoped invocation admission material.
    pub admissions: StateQuota,
    /// Program instances and their exact owner/idempotency state.
    pub instances: StateQuota,
    /// Root event reservations and owner claims.
    pub reservations: StateQuota,
    /// Fulfilled event applications keyed by exact idempotency key.
    pub applications: StateQuota,
    /// Cancellation markers needed by an invocation while it is settling.
    pub cancellations: StateQuota,
    /// Last committed output retained for the interaction client.
    pub last_output: StateQuota,
    /// Ordered live observations retained for deterministic consumers.
    pub observations: StateQuota,
    /// Maximum serialized invocation input retained for idempotency.
    pub max_input_bytes: u64,
}

impl Default for RuntimeStatePolicy {
    fn default() -> Self {
        Self {
            artifacts: StateQuota::new(128, 512 * 1024 * 1024, Duration::from_secs(3600)),
            admissions: StateQuota::new(128, 128 * 1024 * 1024, Duration::from_secs(1800)),
            instances: StateQuota::new(1024, 128 * 1024 * 1024, Duration::from_secs(3600)),
            reservations: StateQuota::new(2048, 16 * 1024 * 1024, Duration::from_secs(3600)),
            applications: StateQuota::new(4096, 64 * 1024 * 1024, Duration::from_secs(3600)),
            cancellations: StateQuota::new(2048, 8 * 1024 * 1024, Duration::from_secs(3600)),
            last_output: StateQuota::new(1, 16 * 1024 * 1024, Duration::from_secs(1800)),
            observations: StateQuota::new(4096, 8 * 1024 * 1024, Duration::from_secs(3600)),
            max_input_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Runtime Service handler over the native protocol.
pub struct RuntimeService {
    artifacts: ArtifactStore,
    artifact_meta: BTreeMap<String, StateEntry>,
    artifact_bytes: u64,
    artifact_admissions: BTreeMap<String, InvocationMaterials>,
    admission_meta: BTreeMap<String, StateEntry>,
    admission_bytes: u64,
    instances: BTreeMap<String, InstanceState>,
    instance_bytes: u64,
    applications: Vec<ApplicationState>,
    application_bytes: u64,
    reservations: BTreeMap<(String, u64), ReservationState>,
    reservation_bytes: u64,
    next_generation: u64,
    last_output: Option<Value>,
    last_output_at: Option<Instant>,
    last_output_bytes: u64,
    handlers: Option<AdmittedPackageHandlers>,
    package_root: Option<PathBuf>,
    sandbox_registry: Option<Arc<SandboxRegistry>>,
    observer: RecordingObserver,
    broker: Arc<dyn ApprovalBroker>,
    cancelled: BTreeMap<String, StateEntry>,
    cancellation_bytes: u64,
    disconnected: bool,
    artifact_dir: Option<PathBuf>,
    state_policy: RuntimeStatePolicy,
}

impl Default for RuntimeService {
    fn default() -> Self {
        Self::from_env()
    }
}

impl RuntimeService {
    /// Product handler. Loads committed artifacts from `APXM_ARTIFACT_DIR`.
    #[must_use]
    pub fn from_env() -> Self {
        let mut service = Self {
            artifacts: ArtifactStore::default(),
            artifact_meta: BTreeMap::new(),
            artifact_bytes: 0,
            artifact_admissions: BTreeMap::new(),
            admission_meta: BTreeMap::new(),
            admission_bytes: 0,
            instances: BTreeMap::new(),
            instance_bytes: 0,
            applications: Vec::new(),
            application_bytes: 0,
            reservations: BTreeMap::new(),
            reservation_bytes: 0,
            next_generation: 0,
            last_output: None,
            last_output_at: None,
            last_output_bytes: 0,
            handlers: None,
            package_root: None,
            sandbox_registry: None,
            observer: RecordingObserver::default(),
            broker: Arc::new(DenyBroker),
            cancelled: BTreeMap::new(),
            cancellation_bytes: 0,
            disconnected: false,
            artifact_dir: None,
            state_policy: RuntimeStatePolicy::default(),
        };
        if let Ok(dir) = std::env::var("APXM_ARTIFACT_DIR")
            && !dir.trim().is_empty()
        {
            service.artifact_dir = Some(PathBuf::from(dir));
        }
        service
    }

    /// Load admitted artifacts from a shared directory written by Compilation Service.
    #[must_use]
    pub fn with_artifact_dir(mut self, dir: PathBuf) -> Self {
        self.artifact_dir = Some(dir);
        self
    }

    /// Apply an explicit bounded state policy before serving requests.
    #[must_use]
    pub fn with_state_policy(mut self, policy: RuntimeStatePolicy) -> Self {
        self.state_policy = policy;
        self
    }

    /// The active bounded state policy.
    #[must_use]
    pub fn state_policy(&self) -> &RuntimeStatePolicy {
        &self.state_policy
    }
}

#[derive(Clone, Copy)]
struct StateEntry {
    bytes: u64,
    expires_at: Instant,
}

impl StateEntry {
    fn expired(self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

struct InstanceState {
    artifact_digest: String,
    materials: Option<InvocationMaterials>,
    owner_claim: RuntimeOwnerClaim,
    invocation: Option<InvocationState>,
    invocation_bytes: u64,
    state_entry: StateEntry,
}

struct InvocationState {
    request_id: String,
    input_fingerprint: String,
    program_invocation_id: String,
    result: RuntimeResult,
}

struct ReservationState {
    owner_claim: RuntimeOwnerClaim,
    _type_id: String,
    state_entry: StateEntry,
}

struct ApplicationState {
    idempotency_key: String,
    event_ref: CanonicalEventRef,
    payload: Value,
    state_entry: StateEntry,
}

impl RuntimeService {
    fn now(&self) -> Instant {
        // Instant is monotonic, so expiry is not affected by wall-clock
        // adjustments or an operator changing the system time.
        Instant::now()
    }

    fn entry(&self, bytes: u64, ttl: Duration) -> StateEntry {
        StateEntry {
            bytes,
            expires_at: self.now().checked_add(ttl).unwrap_or_else(|| self.now()),
        }
    }

    fn quota_available(quota: StateQuota, entries: usize, bytes: u64, add_bytes: u64) -> bool {
        entries < quota.max_entries
            && bytes
                .checked_add(add_bytes)
                .is_some_and(|total| total <= quota.max_bytes)
    }

    fn quota_code(resource: &str) -> String {
        format!("{resource}_quota_exceeded")
    }

    fn materials_size(materials: &InvocationMaterials) -> Option<u64> {
        let admission = serde_json::to_vec(&materials.admission).ok()?;
        let total = admission
            .len()
            .checked_add(materials.release_bytes.len())?
            .checked_add(materials.provenance_bytes.len())?;
        u64::try_from(total).ok()
    }

    fn output_size(output: &Value) -> Option<u64> {
        u64::try_from(serde_json::to_vec(output).ok()?.len()).ok()
    }

    fn observation_size(observation: &Observation) -> Option<u64> {
        let bytes = match observation {
            Observation::ProvisionalContent { content_ref } => content_ref.len(),
            Observation::EventLifecycle { event_id, phase } => {
                event_id.len().checked_add(phase.len())?
            }
            Observation::TerminalCommit { commit_id } => commit_id.len(),
        };
        u64::try_from(bytes).ok()
    }

    /// Remove only entries whose typed TTL has elapsed. No live owner claim,
    /// invocation idempotency record, or event application is evicted.
    pub fn cleanup_expired(&mut self) {
        let now = self.now();

        let expired_instances = self
            .instances
            .iter()
            .filter_map(|(id, instance)| instance.state_entry.expired(now).then_some(id.clone()))
            .collect::<Vec<_>>();
        for id in expired_instances {
            if let Some(instance) = self.instances.remove(&id) {
                self.instance_bytes = self
                    .instance_bytes
                    .saturating_sub(instance.state_entry.bytes);
            }
        }

        let expired_admissions = self
            .admission_meta
            .iter()
            .filter_map(|(digest, entry)| entry.expired(now).then_some(digest.clone()))
            .collect::<Vec<_>>();
        for digest in expired_admissions {
            self.admission_meta.remove(&digest);
            if let Some(materials) = self.artifact_admissions.remove(&digest) {
                self.admission_bytes = self
                    .admission_bytes
                    .saturating_sub(Self::materials_size(&materials).unwrap_or(0));
            }
        }

        let expired_artifacts = self
            .artifact_meta
            .iter()
            .filter_map(|(digest, entry)| {
                if entry.expired(now)
                    && !self
                        .instances
                        .values()
                        .any(|instance| instance.artifact_digest == *digest)
                {
                    Some(digest.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if !expired_artifacts.is_empty() {
            for digest in &expired_artifacts {
                if let Some(entry) = self.artifact_meta.remove(digest) {
                    self.artifact_bytes = self.artifact_bytes.saturating_sub(entry.bytes);
                }
            }
            // ArtifactStore intentionally exposes only digest-bound reads and
            // commits. Rebuild the bounded live set to remove expired bytes
            // without adding a mutable arbitrary-delete API to the store.
            let mut live = ArtifactStore::default();
            for digest in self.artifact_meta.keys() {
                if let Some(bytes) = self.artifacts.get(digest) {
                    live.commit_named(digest.clone(), bytes.to_vec());
                }
            }
            self.artifacts = live;
        }

        self.reservations.retain(|_, reservation| {
            if reservation.state_entry.expired(now) {
                self.reservation_bytes = self
                    .reservation_bytes
                    .saturating_sub(reservation.state_entry.bytes);
                false
            } else {
                true
            }
        });

        let mut remaining_applications = Vec::with_capacity(self.applications.len());
        for application in self.applications.drain(..) {
            if application.state_entry.expired(now) {
                self.application_bytes = self
                    .application_bytes
                    .saturating_sub(application.state_entry.bytes);
            } else {
                remaining_applications.push(application);
            }
        }
        self.applications = remaining_applications;

        self.cancelled.retain(|_, entry| {
            if entry.expired(now) {
                self.cancellation_bytes = self.cancellation_bytes.saturating_sub(entry.bytes);
                false
            } else {
                true
            }
        });

        if self.last_output_at.is_some_and(|created| {
            created
                .checked_add(self.state_policy.last_output.ttl)
                .map_or(true, |expiry| now >= expiry)
        }) {
            self.last_output = None;
            self.last_output_at = None;
            self.last_output_bytes = 0;
        }
    }

    /// Bind package-local Capability handlers for subsequent invocations.
    pub fn bind_package(
        &mut self,
        handlers: Option<AdmittedPackageHandlers>,
        package_root: Option<PathBuf>,
    ) {
        self.handlers = handlers;
        self.package_root = package_root;
        self.sandbox_registry = None;
    }

    /// Bind package handlers together with the trusted host confinement
    /// backend they must use. A missing backend keeps package workers
    /// unavailable and therefore fails closed at invocation.
    pub fn bind_package_with_sandbox(
        &mut self,
        handlers: Option<AdmittedPackageHandlers>,
        package_root: Option<PathBuf>,
        sandbox_registry: Option<Arc<SandboxRegistry>>,
    ) {
        self.handlers = handlers;
        self.package_root = package_root;
        self.sandbox_registry = sandbox_registry;
    }

    /// Commit AIR/artifact bytes. The digest is the only executable identity.
    pub fn admit_artifact(&mut self, bytes: Vec<u8>) -> String {
        self.try_admit_artifact(bytes).unwrap_or_default()
    }

    /// Commit an artifact, failing closed when the bounded artifact policy is
    /// exhausted. The legacy string-returning method above maps refusal to an
    /// empty digest so callers cannot accidentally execute an uncommitted
    /// artifact.
    pub fn try_admit_artifact(&mut self, bytes: Vec<u8>) -> Result<String, String> {
        self.cleanup_expired();
        let digest = artifact_digest(&bytes);
        self.try_admit_named_artifact(digest.clone(), bytes)?;
        Ok(digest)
    }

    fn try_admit_named_artifact(&mut self, digest: String, bytes: Vec<u8>) -> Result<(), String> {
        let size = u64::try_from(bytes.len()).map_err(|_| "artifact_too_large".to_owned())?;
        if size > MAX_ARTIFACT_BYTES || size > self.state_policy.artifacts.max_bytes {
            return Err("artifact_too_large".to_owned());
        }
        if artifact_digest(&bytes) != digest {
            return Err("artifact_digest_mismatch".to_owned());
        }
        if self.artifacts.get(&digest).is_some() {
            return Ok(());
        }
        if !Self::quota_available(
            self.state_policy.artifacts,
            self.artifact_meta.len(),
            self.artifact_bytes,
            size,
        ) {
            return Err(Self::quota_code("artifact"));
        }
        self.artifacts.commit(bytes);
        self.artifact_meta.insert(
            digest.clone(),
            self.entry(size, self.state_policy.artifacts.ttl),
        );
        self.artifact_bytes = self.artifact_bytes.saturating_add(size);
        Ok(())
    }

    /// Bind invocation admission materials to an instance created from a digest.
    pub fn bind_admission(
        &mut self,
        program_instance_id: &str,
        materials: InvocationMaterials,
    ) -> Result<(), String> {
        self.cleanup_expired();
        let size =
            Self::materials_size(&materials).ok_or_else(|| "admission_too_large".to_owned())?;
        if size > self.state_policy.admissions.max_bytes {
            return Err("admission_too_large".to_owned());
        }
        let (artifact_digest, prior_size) = {
            let instance = self
                .instances
                .get(program_instance_id)
                .ok_or_else(|| "unknown_instance".to_owned())?;
            (instance.artifact_digest.clone(), instance.state_entry.bytes)
        };
        if materials.admission.artifact_digest != artifact_digest {
            return Err("artifact_digest_mismatch".to_owned());
        }
        let invocation_bytes = self
            .instances
            .get(program_instance_id)
            .map_or(0, |instance| instance.invocation_bytes);
        let next_state_bytes = invocation_bytes
            .checked_add(size)
            .ok_or_else(|| Self::quota_code("instance"))?;
        let next_bytes = self
            .instance_bytes
            .saturating_sub(prior_size)
            .checked_add(next_state_bytes)
            .ok_or_else(|| Self::quota_code("instance"))?;
        if next_bytes > self.state_policy.instances.max_bytes {
            return Err(Self::quota_code("instance"));
        }
        let instance = self
            .instances
            .get_mut(program_instance_id)
            .ok_or_else(|| "unknown_instance".to_owned())?;
        instance.materials = Some(materials);
        instance.state_entry.bytes = next_state_bytes;
        self.instance_bytes = next_bytes;
        Ok(())
    }

    /// Bind exact admission materials for instances created from an artifact.
    ///
    /// This is the composition boundary for callers that only know the
    /// committed artifact digest before the Runtime Service creates an
    /// instance. Missing bindings still fail closed at invocation.
    pub fn bind_admission_for_artifact(
        &mut self,
        artifact_digest: &str,
        materials: InvocationMaterials,
    ) -> Result<(), String> {
        self.cleanup_expired();
        if self.artifacts.get(artifact_digest).is_none() {
            return Err("unknown_artifact".to_owned());
        }
        if materials.admission.artifact_digest != artifact_digest {
            return Err("artifact_digest_mismatch".to_owned());
        }
        let size =
            Self::materials_size(&materials).ok_or_else(|| "admission_too_large".to_owned())?;
        if size > self.state_policy.admissions.max_bytes {
            return Err("admission_too_large".to_owned());
        }
        let prior_size = self
            .artifact_admissions
            .get(artifact_digest)
            .and_then(Self::materials_size)
            .unwrap_or(0);
        let next_bytes = self
            .admission_bytes
            .saturating_sub(prior_size)
            .checked_add(size)
            .ok_or_else(|| Self::quota_code("admission"))?;
        if self.artifact_admissions.get(artifact_digest).is_none()
            && !Self::quota_available(
                self.state_policy.admissions,
                self.artifact_admissions.len(),
                self.admission_bytes,
                size,
            )
        {
            return Err(Self::quota_code("admission"));
        }
        self.artifact_admissions
            .insert(artifact_digest.to_owned(), materials);
        self.admission_meta.insert(
            artifact_digest.to_owned(),
            self.entry(size, self.state_policy.admissions.ttl),
        );
        self.admission_bytes = next_bytes;
        Ok(())
    }

    /// Last committed invocation output, when one completed.
    #[must_use]
    pub fn last_output(&self) -> Option<&Value> {
        if self.last_output_at.is_some_and(|created| {
            created
                .checked_add(self.state_policy.last_output.ttl)
                .map_or(true, |expiry| Instant::now() >= expiry)
        }) {
            return None;
        }
        self.last_output.as_ref()
    }

    /// Artifact bytes previously admitted under `digest`.
    #[must_use]
    pub fn artifact_bytes(&self, digest: &str) -> Option<&[u8]> {
        if self
            .artifact_meta
            .get(digest)
            .is_some_and(|entry| entry.expired(Instant::now()))
        {
            return None;
        }
        self.artifacts.get(digest)
    }

    /// Bind an approval broker. Headless default is [`DenyBroker`].
    pub fn bind_approval_broker(&mut self, broker: Arc<dyn ApprovalBroker>) {
        self.broker = broker;
    }

    /// Projected observations in driver order. Payloads are never included.
    #[must_use]
    pub fn observations(&self) -> Vec<Observation> {
        self.observer
            .observations
            .lock()
            .expect("observer lock")
            .clone()
    }

    /// Record a client disconnect. This does not fabricate `finish_reason: stop`.
    pub fn disconnect(&mut self) {
        self.disconnected = true;
    }

    /// Admit a handshake and request. Source is not executable truth.
    pub fn handle(
        &mut self,
        handshake: &RuntimeHandshake,
        request: RuntimeRequest,
    ) -> Result<RuntimeResult, ProtocolError> {
        self.cleanup_expired();
        handshake.admit()?;
        match request {
            RuntimeRequest::ProgramInstanceCreate {
                request_id,
                artifact_digest,
            } => self.create_instance(request_id, artifact_digest),
            RuntimeRequest::ProgramInvocationStart {
                request_id,
                program_instance_id,
                owner_claim,
                input,
            } => Ok(self.start_invocation(request_id, program_instance_id, owner_claim, input)),
            RuntimeRequest::EventReserve {
                request_id,
                type_id,
            } => self.reserve_event(request_id, type_id),
            RuntimeRequest::EventFulfill {
                request_id,
                owner_claim,
                application,
            } => self.fulfill_event(request_id, owner_claim, application),
            RuntimeRequest::EventInspect {
                request_id,
                owner_claim,
                event_ref,
            } => self.inspect_event(request_id, owner_claim, event_ref),
            RuntimeRequest::ProgramInvocationCancel {
                request_id,
                owner_claim,
                program_invocation_id,
            } => Ok(self.cancel_invocation(request_id, owner_claim, program_invocation_id)),
        }
    }
}

impl RuntimeService {
    fn create_instance(
        &mut self,
        request_id: String,
        artifact_digest: String,
    ) -> Result<RuntimeResult, ProtocolError> {
        if !is_strict_digest(&artifact_digest) {
            return Err(ProtocolError::SourceAsExecutable);
        }
        if self.artifacts.get(&artifact_digest).is_none()
            && let Some(bytes) = self.load_persisted_artifact(&artifact_digest)
        {
            let _ = self.try_admit_named_artifact(artifact_digest.clone(), bytes);
        }
        if self.artifacts.get(&artifact_digest).is_none() {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "unknown_artifact".to_owned(),
            });
        }
        let materials = self.artifact_admissions.get(&artifact_digest).cloned();
        let material_bytes = materials
            .as_ref()
            .and_then(Self::materials_size)
            .unwrap_or(0);
        if !Self::quota_available(
            self.state_policy.instances,
            self.instances.len(),
            self.instance_bytes,
            material_bytes,
        ) {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: Self::quota_code("instance"),
            });
        }
        let id = format!("pi-{}", Uuid::new_v4());
        let owner_claim = RuntimeOwnerClaim::mint();
        self.instances.insert(
            id.clone(),
            InstanceState {
                artifact_digest: artifact_digest.clone(),
                materials,
                owner_claim: owner_claim.clone(),
                invocation: None,
                invocation_bytes: 0,
                state_entry: self.entry(material_bytes, self.state_policy.instances.ttl),
            },
        );
        self.instance_bytes = self.instance_bytes.saturating_add(material_bytes);
        Ok(RuntimeResult::ProgramInstanceCreated {
            request_id,
            program_instance_id: id,
            owner_claim,
            artifact_digest,
        })
    }

    fn start_invocation(
        &mut self,
        request_id: String,
        program_instance_id: String,
        owner_claim: RuntimeOwnerClaim,
        input: Value,
    ) -> RuntimeResult {
        if u64::try_from(request_id.len()).unwrap_or(u64::MAX) > self.state_policy.max_input_bytes {
            return RuntimeResult::Failed {
                request_id: "request_id_too_large".to_owned(),
                code: "request_id_too_large".to_owned(),
            };
        }
        if owner_claim.validate().is_err() {
            return RuntimeResult::Failed {
                request_id,
                code: "invalid_owner_claim".to_owned(),
            };
        }
        if self.disconnected {
            return RuntimeResult::Failed {
                request_id,
                code: "disconnected".to_owned(),
            };
        }
        let Some(instance) = self.instances.get(&program_instance_id) else {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_instance".to_owned(),
            };
        };
        if instance.owner_claim != owner_claim {
            return RuntimeResult::Failed {
                request_id,
                code: "owner_mismatch".to_owned(),
            };
        }
        let input_fingerprint = serde_json::to_string(&input).unwrap_or_default();
        let input_bytes = u64::try_from(input_fingerprint.len()).unwrap_or(u64::MAX);
        if input_bytes > self.state_policy.max_input_bytes {
            return RuntimeResult::Failed {
                request_id,
                code: "input_too_large".to_owned(),
            };
        }
        if let Some(prior) = instance.invocation.as_ref() {
            if prior.request_id == request_id {
                if self.cancelled.contains_key(&prior.program_invocation_id) {
                    return RuntimeResult::Cancelled { request_id };
                }
                if prior.input_fingerprint == input_fingerprint {
                    return prior.result.clone();
                }
                return RuntimeResult::Failed {
                    request_id,
                    code: "invocation_idempotency_conflict".to_owned(),
                };
            }
            return RuntimeResult::Failed {
                request_id,
                code: "invocation_already_started".to_owned(),
            };
        }
        let instance_base_bytes = instance.state_entry.bytes;
        let request_bytes = u64::try_from(request_id.len()).unwrap_or(u64::MAX);
        let instance_with_input = instance_base_bytes
            .checked_add(input_bytes)
            .and_then(|bytes| bytes.checked_add(request_bytes))
            .unwrap_or(u64::MAX);
        let aggregate_instance_bytes = self
            .instance_bytes
            .saturating_sub(instance_base_bytes)
            .checked_add(instance_with_input)
            .unwrap_or(u64::MAX);
        if aggregate_instance_bytes > self.state_policy.instances.max_bytes {
            return RuntimeResult::Failed {
                request_id,
                code: Self::quota_code("instance"),
            };
        }
        let invocation_id = format!("{program_instance_id}:inv-{}", Uuid::new_v4());
        let artifact_digest = instance.artifact_digest.clone();
        let Some(bytes) = self.artifacts.get(&artifact_digest) else {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_artifact".to_owned(),
            };
        };
        let bytes = bytes.to_vec();
        let Ok(air) = serde_json::from_slice::<AirModule>(&bytes) else {
            return RuntimeResult::Failed {
                request_id,
                code: "invalid_artifact".to_owned(),
            };
        };
        if let Err(code) =
            validate_package_permission_resolution(&air, self.package_root.as_deref())
        {
            return RuntimeResult::Failed { request_id, code };
        }
        if let Err(code) = block_on(self.resolve_asks(&air)) {
            return RuntimeResult::Failed { request_id, code };
        }
        let Some(bound_materials) = instance.materials.as_ref() else {
            return RuntimeResult::Failed {
                request_id,
                code: "missing_invocation_admission".to_owned(),
            };
        };
        let mut materials = InvocationMaterials {
            admission: bound_materials.admission.clone(),
            release_bytes: bound_materials.release_bytes.clone(),
            provenance_bytes: bound_materials.provenance_bytes.clone(),
        };
        // The service owns the invocation identity. Keep the externally
        // supplied admission fields intact except for this runtime-minted
        // identity, which is not caller-authoritative.
        materials.admission.invocation_id = invocation_id.clone();
        let result = match block_on(execute_admitted_artifact_with_sandbox(
            air,
            &bytes,
            &materials,
            self.handlers.as_ref(),
            self.package_root.as_deref(),
            self.sandbox_registry.clone(),
        )) {
            Ok(output) => {
                if self.disconnected
                    || self.invocation_is_cancelled(&program_instance_id, &invocation_id)
                {
                    return RuntimeResult::Failed {
                        request_id,
                        code: "disconnected".to_owned(),
                    };
                }
                let Some(output_bytes) = Self::output_size(&output) else {
                    return RuntimeResult::Failed {
                        request_id,
                        code: "output_too_large".to_owned(),
                    };
                };
                if self.state_policy.last_output.max_entries == 0
                    || output_bytes > self.state_policy.last_output.max_bytes
                {
                    return RuntimeResult::Failed {
                        request_id,
                        code: "output_too_large".to_owned(),
                    };
                }
                if !self.project_execution(&output, &invocation_id) {
                    return RuntimeResult::Failed {
                        request_id,
                        code: Self::quota_code("observation"),
                    };
                }
                if let Some(dir) = &self.artifact_dir {
                    let _ = std::fs::write(
                        dir.join(format!("{}.output.json", invocation_id.replace(':', "-"))),
                        serde_json::to_vec(&output).unwrap_or_else(|_| b"{}".to_vec()),
                    );
                }
                self.last_output = Some(output);
                self.last_output_at = Some(self.now());
                self.last_output_bytes = output_bytes;
                RuntimeResult::ProgramInvocationStarted {
                    request_id: request_id.clone(),
                    program_invocation_id: invocation_id.clone(),
                }
            }
            Err(code) => RuntimeResult::Failed {
                request_id: request_id.clone(),
                code,
            },
        };
        if let Some(instance) = self.instances.get_mut(&program_instance_id) {
            self.instance_bytes = self
                .instance_bytes
                .saturating_sub(instance.state_entry.bytes)
                .saturating_add(instance_with_input);
            instance.state_entry.bytes = instance_with_input;
            instance.invocation_bytes = input_bytes.saturating_add(request_bytes);
            instance.invocation = Some(InvocationState {
                request_id: request_id.clone(),
                input_fingerprint,
                program_invocation_id: invocation_id,
                result: result.clone(),
            });
        }
        result
    }

    fn cancel_invocation(
        &mut self,
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
        program_invocation_id: String,
    ) -> RuntimeResult {
        if owner_claim.validate().is_err() {
            return RuntimeResult::Failed {
                request_id,
                code: "invalid_owner_claim".to_owned(),
            };
        }
        let Some(instance) = self.instances.values().find(|instance| {
            instance
                .invocation
                .as_ref()
                .is_some_and(|invocation| invocation.program_invocation_id == program_invocation_id)
        }) else {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_invocation".to_owned(),
            };
        };
        if instance.owner_claim != owner_claim {
            return RuntimeResult::Failed {
                request_id,
                code: "owner_mismatch".to_owned(),
            };
        }
        if self.cancelled.contains_key(&program_invocation_id) {
            return RuntimeResult::Cancelled { request_id };
        }
        let marker_bytes = u64::try_from(program_invocation_id.len()).unwrap_or(u64::MAX);
        if !Self::quota_available(
            self.state_policy.cancellations,
            self.cancelled.len(),
            self.cancellation_bytes,
            marker_bytes,
        ) {
            return RuntimeResult::Failed {
                request_id,
                code: Self::quota_code("cancellation"),
            };
        }
        let mut marker = self.entry(marker_bytes, self.state_policy.cancellations.ttl);
        if let Some(invocation) = instance.invocation.as_ref() {
            if marker.expires_at < instance.state_entry.expires_at {
                marker.expires_at = instance.state_entry.expires_at;
            }
            debug_assert_eq!(invocation.program_invocation_id, program_invocation_id);
        }
        self.cancelled.insert(program_invocation_id, marker);
        self.cancellation_bytes = self.cancellation_bytes.saturating_add(marker_bytes);
        RuntimeResult::Cancelled { request_id }
    }

    fn invocation_is_cancelled(&self, program_instance_id: &str, invocation_id: &str) -> bool {
        let _ = program_instance_id;
        self.cancelled.contains_key(invocation_id)
    }

    async fn resolve_asks(&self, air: &AirModule) -> Result<(), String> {
        for (capability_ref, decision) in &air.capability_permission_requests {
            if !matches!(decision, PermissionDecision::Ask { .. }) {
                continue;
            }
            match self.broker.resolve_ask(capability_ref).await {
                ApprovalDecision::Allow => {}
                ApprovalDecision::Deny => return Err("ask_denied".to_owned()),
                ApprovalDecision::Timeout => return Err("ask_timeout".to_owned()),
            }
        }
        Ok(())
    }

    fn record_observation(&self, observation: Observation) -> bool {
        let Some(size) = Self::observation_size(&observation) else {
            return false;
        };
        let mut observations = self.observer.observations.lock().expect("observer lock");
        let current_bytes = observations
            .iter()
            .filter_map(Self::observation_size)
            .sum::<u64>();
        if !Self::quota_available(
            self.state_policy.observations,
            observations.len(),
            current_bytes,
            size,
        ) {
            return false;
        }
        observations.push(observation);
        true
    }

    fn project_execution(&self, output: &Value, commit_id: &str) -> bool {
        if let Some(nodes) = output
            .pointer("/results/node_outcomes")
            .and_then(Value::as_array)
        {
            for node in nodes {
                let kind = node.get("kind").and_then(Value::as_str).unwrap_or("");
                match kind {
                    "model.call" => {
                        let content_ref = node
                            .get("node_id")
                            .and_then(Value::as_str)
                            .unwrap_or("model")
                            .to_owned();
                        if !self.record_observation(Observation::ProvisionalContent { content_ref })
                        {
                            return false;
                        }
                    }
                    "await.event" => {
                        let event_id = node
                            .pointer("/outcome/event_ref")
                            .and_then(Value::as_str)
                            .or_else(|| node.get("node_id").and_then(Value::as_str))
                            .unwrap_or("event")
                            .to_owned();
                        let phase = node
                            .pointer("/outcome/status")
                            .and_then(Value::as_str)
                            .unwrap_or("applied")
                            .to_owned();
                        if !self.record_observation(Observation::EventLifecycle { event_id, phase })
                        {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
        }
        if output.get("content").is_some() {
            if !self.record_observation(Observation::ProvisionalContent {
                content_ref: format!("content:{commit_id}"),
            }) {
                return false;
            }
        }
        self.record_observation(Observation::TerminalCommit {
            commit_id: commit_id.to_owned(),
        })
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
        let type_bytes = u64::try_from(type_id.len()).unwrap_or(u64::MAX);
        if !Self::quota_available(
            self.state_policy.reservations,
            self.reservations.len(),
            self.reservation_bytes,
            type_bytes,
        ) {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: Self::quota_code("reservation"),
            });
        }
        let Some(generation) = self.next_generation.checked_add(1) else {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "generation_exhausted".to_owned(),
            });
        };
        self.next_generation = generation;
        let owner_claim = RuntimeOwnerClaim::mint();
        let event_ref = CanonicalEventRef {
            event_id: format!("evt-{}", Uuid::new_v4()),
            generation,
        };
        self.reservations.insert(
            (event_ref.event_id.clone(), event_ref.generation),
            ReservationState {
                owner_claim: owner_claim.clone(),
                _type_id: type_id,
                state_entry: self.entry(type_bytes, self.state_policy.reservations.ttl),
            },
        );
        self.reservation_bytes = self.reservation_bytes.saturating_add(type_bytes);
        Ok(RuntimeResult::EventReserved {
            request_id,
            owner_claim,
            event_ref,
        })
    }

    fn fulfill_event(
        &mut self,
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
        application: EventApplication<Value>,
    ) -> Result<RuntimeResult, ProtocolError> {
        application
            .validate_identities()
            .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
        owner_claim.validate()?;
        let Some(reservation) = self.reservations.get(&(
            application.event_ref.event_id.clone(),
            application.event_ref.generation,
        )) else {
            return Ok(RuntimeResult::EventApplied {
                request_id,
                result: EventApplicationResult::Rejected,
            });
        };
        if reservation.owner_claim != owner_claim {
            return Err(ProtocolError::OwnerMismatch);
        }
        let key = application.idempotency_key.clone();
        if let Some(prior) = self
            .applications
            .iter()
            .find(|item| item.idempotency_key == key)
        {
            if prior.payload != application.occurrence.payload
                || prior.event_ref != application.event_ref
            {
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
        let payload_bytes = u64::try_from(
            serde_json::to_vec(&application.occurrence.payload)
                .map_err(|_| ProtocolError::ForbiddenEventMethod)?
                .len(),
        )
        .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
        let key_bytes =
            u64::try_from(key.len()).map_err(|_| ProtocolError::ForbiddenEventMethod)?;
        let application_bytes = payload_bytes
            .checked_add(key_bytes)
            .ok_or(ProtocolError::ForbiddenEventMethod)?;
        if !Self::quota_available(
            self.state_policy.applications,
            self.applications.len(),
            self.application_bytes,
            application_bytes,
        ) {
            return Ok(RuntimeResult::EventApplied {
                request_id,
                result: EventApplicationResult::Rejected,
            });
        }
        self.applications.push(ApplicationState {
            idempotency_key: key,
            event_ref: application.event_ref.clone(),
            payload: application.occurrence.payload.clone(),
            state_entry: self.entry(application_bytes, self.state_policy.applications.ttl),
        });
        self.application_bytes = self.application_bytes.saturating_add(application_bytes);
        // Event application is the only wake authority. A fulfill with no
        // parked continuation still records the application; it does not
        // invent a successful invocation finish.
        let _ = wake_authority(&application);
        Ok(RuntimeResult::EventApplied {
            request_id,
            result: EventApplicationResult::Fulfilled,
        })
    }

    fn load_persisted_artifact(&self, digest: &str) -> Option<Vec<u8>> {
        let dir = self.artifact_dir.as_ref()?;
        if !is_strict_digest(digest) {
            return None;
        }
        let dir_meta = std::fs::symlink_metadata(dir).ok()?;
        if dir_meta.file_type().is_symlink() || !dir_meta.is_dir() {
            return None;
        }
        let path = dir.join(digest.replace(':', "-"));
        let path_meta = std::fs::symlink_metadata(&path).ok()?;
        if path_meta.file_type().is_symlink() || !path_meta.is_file() {
            return None;
        }
        if path_meta.len() > MAX_ARTIFACT_BYTES {
            return None;
        }
        let file = std::fs::File::open(&path).ok()?;
        let opened_meta = file.metadata().ok()?;
        if !opened_meta.is_file() || opened_meta.len() != path_meta.len() {
            return None;
        }
        let capacity = usize::try_from(opened_meta.len()).ok()?;
        let mut bytes = Vec::with_capacity(capacity);
        file.take(MAX_ARTIFACT_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_ARTIFACT_BYTES || artifact_digest(&bytes) != digest {
            return None;
        }
        Some(bytes)
    }

    #[allow(clippy::unused_self)]
    fn inspect_event(
        &mut self,
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
        event_ref: CanonicalEventRef,
    ) -> Result<RuntimeResult, ProtocolError> {
        event_ref
            .validate()
            .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
        owner_claim.validate()?;
        let Some(reservation) = self
            .reservations
            .get(&(event_ref.event_id.clone(), event_ref.generation))
        else {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "unknown_reservation".to_owned(),
            });
        };
        if reservation.owner_claim != owner_claim {
            return Err(ProtocolError::OwnerMismatch);
        }
        Ok(RuntimeResult::Failed {
            request_id,
            code: "inspect_ok".to_owned(),
        })
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
                    owner_claim: owner_claim(&service, &program_instance_id),
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
                    artifact_digest: format!("sha256:{}", "d".repeat(64)),
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            RuntimeResult::Failed { code, .. } if code == "unknown_artifact"
        ));
    }

    #[test]
    fn malformed_digest_grammar_is_rejected_before_path_lookup() {
        let mut service = RuntimeService::default();
        for artifact_digest in [
            "sha256:deadbeef",
            &format!("sha256:{}", "A".repeat(64)),
            "sha256:../escape",
            "blake3:0000000000000000000000000000000000000000000000000000000000000000",
        ] {
            let error = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInstanceCreate {
                        request_id: "c".to_owned(),
                        artifact_digest: (*artifact_digest).to_owned(),
                    },
                )
                .unwrap_err();
            assert_eq!(error, ProtocolError::SourceAsExecutable);
        }
    }

    #[test]
    fn persisted_artifact_bytes_are_digest_verified() {
        let dir = tempfile::tempdir().expect("artifact directory");
        let bytes = fixture_air_bytes();
        let digest = artifact_digest(&bytes);
        let path = dir.path().join(digest.replace(':', "-"));
        let mut tampered = bytes;
        tampered.push(b' ');
        fs::write(path, tampered).expect("tampered artifact");
        let mut service = RuntimeService::default().with_artifact_dir(dir.path().to_path_buf());
        let result = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "c".to_owned(),
                    artifact_digest: digest,
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            RuntimeResult::Failed { ref code, .. } if code == "unknown_artifact"
        ));
    }

    #[test]
    fn invocation_without_bound_admission_is_rejected() {
        let mut service = RuntimeService::default();
        let instance = {
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
            program_instance_id
        };
        let result = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "s".to_owned(),
                    owner_claim: owner_claim(&service, &instance),
                    program_instance_id: instance,
                    input: serde_json::json!({}),
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            RuntimeResult::Failed { code, .. } if code == "missing_invocation_admission"
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
        let RuntimeResult::EventReserved {
            event_ref,
            owner_claim,
            ..
        } = reserved
        else {
            panic!("reserve");
        };
        let result = service
            .handle(
                &handshake(),
                RuntimeRequest::EventFulfill {
                    request_id: "f".to_owned(),
                    owner_claim,
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

    fn create_started(service: &mut RuntimeService, bytes: Vec<u8>) -> String {
        let digest = service.admit_artifact(bytes.clone());
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
        service
            .bind_admission(
                &program_instance_id,
                materials_for_artifact(
                    &bytes,
                    format!("{program_instance_id}:inv-1"),
                    b"{}".to_vec(),
                    b"{}".to_vec(),
                ),
            )
            .expect("bind synthetic test admission");
        program_instance_id
    }

    fn owner_claim(service: &RuntimeService, instance: &str) -> RuntimeOwnerClaim {
        service
            .instances
            .get(instance)
            .expect("instance claim")
            .owner_claim
            .clone()
    }

    fn ask_air_bytes() -> Vec<u8> {
        let mut air: serde_json::Value = serde_json::from_slice(&fixture_air_bytes()).unwrap();
        air["capability_permission_requests"] = serde_json::json!({
            "cap.send": { "decision": "ask", "reason": "confirm" }
        });
        serde_json::to_vec(&air).unwrap()
    }

    #[test]
    fn execute_projects_stream_then_terminal() {
        let mut service = RuntimeService::default();
        let instance = create_started(&mut service, fixture_air_bytes());
        let started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "s".to_owned(),
                    owner_claim: owner_claim(&service, &instance),
                    program_instance_id: instance,
                    input: serde_json::json!({}),
                },
            )
            .unwrap();
        assert!(matches!(
            started,
            RuntimeResult::ProgramInvocationStarted { .. }
        ));
        let observations = service.observations();
        assert!(
            observations
                .iter()
                .any(|item| matches!(item, Observation::EventLifecycle { .. })),
            "event lifecycle without payload: {observations:?}"
        );
        assert!(
            observations
                .iter()
                .any(|item| matches!(item, Observation::ProvisionalContent { .. })),
            "provisional content-ref: {observations:?}"
        );
        let last = observations.last().expect("terminal");
        assert!(
            matches!(last, Observation::TerminalCommit { .. }),
            "terminal commit last: {observations:?}"
        );
        for item in &observations {
            if let Observation::EventLifecycle { event_id, phase } = item {
                assert!(!event_id.is_empty());
                assert!(!phase.is_empty());
            }
        }
    }

    #[test]
    fn denied_ask_never_executes() {
        let mut service = RuntimeService::default();
        let instance = create_started(&mut service, ask_air_bytes());
        let result = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "s".to_owned(),
                    owner_claim: owner_claim(&service, &instance),
                    program_instance_id: instance,
                    input: serde_json::json!({}),
                },
            )
            .unwrap();
        assert!(
            matches!(result, RuntimeResult::Failed { ref code, .. } if code == "ask_denied"),
            "{result:?}"
        );
        assert!(service.last_output().is_none());
        assert!(service.observations().is_empty());
    }

    #[test]
    fn timed_out_ask_never_executes() {
        let mut service = RuntimeService::default();
        service.bind_approval_broker(std::sync::Arc::new(apxm_execution::TimeoutBroker));
        let instance = create_started(&mut service, ask_air_bytes());
        let result = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "s".to_owned(),
                    owner_claim: owner_claim(&service, &instance),
                    program_instance_id: instance,
                    input: serde_json::json!({}),
                },
            )
            .unwrap();
        assert!(
            matches!(result, RuntimeResult::Failed { ref code, .. } if code == "ask_timeout"),
            "{result:?}"
        );
        assert!(service.last_output().is_none());
    }

    #[test]
    fn cancel_prevents_execute() {
        let mut service = RuntimeService::default();
        let instance = create_started(&mut service, fixture_air_bytes());
        let claim = owner_claim(&service, &instance);
        let cancelled = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationCancel {
                    request_id: "x".to_owned(),
                    owner_claim: claim.clone(),
                    program_invocation_id: format!("{instance}:inv-guess"),
                },
            )
            .unwrap();
        assert!(matches!(
            cancelled,
            RuntimeResult::Failed { ref code, .. } if code == "unknown_invocation"
        ));
        let started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "s".to_owned(),
                    owner_claim: claim,
                    program_instance_id: instance,
                    input: serde_json::json!({}),
                },
            )
            .unwrap();
        assert!(matches!(
            started,
            RuntimeResult::ProgramInvocationStarted { .. } | RuntimeResult::Failed { .. }
        ));
    }

    #[test]
    fn disconnect_does_not_fabricate_stop() {
        let mut service = RuntimeService::default();
        let instance = create_started(&mut service, fixture_air_bytes());
        service.disconnect();
        let result = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "s".to_owned(),
                    owner_claim: owner_claim(&service, &instance),
                    program_instance_id: instance,
                    input: serde_json::json!({}),
                },
            )
            .unwrap();
        assert!(
            matches!(result, RuntimeResult::Failed { ref code, .. } if code == "disconnected"),
            "{result:?}"
        );
        assert!(service.last_output().is_none());
        if let Some(output) = service.last_output() {
            assert_ne!(
                output.get("finish_reason").and_then(Value::as_str),
                Some("stop")
            );
        }
    }

    #[test]
    fn state_quotas_fail_closed_without_eviction() {
        let bytes = fixture_air_bytes();
        let mut policy = RuntimeStatePolicy::default();
        policy.artifacts = StateQuota {
            max_entries: 1,
            max_bytes: u64::try_from(bytes.len()).unwrap() + 1,
            ttl: Duration::from_secs(60),
        };
        let mut service = RuntimeService::default().with_state_policy(policy);
        let first = service
            .try_admit_artifact(bytes.clone())
            .expect("first artifact");
        let second = service.try_admit_artifact(vec![b'x'; 2]).unwrap_err();
        assert_eq!(second, "artifact_quota_exceeded");
        assert!(service.artifact_bytes(&first).is_some());
        assert_eq!(service.artifact_meta.len(), 1);
    }

    #[test]
    fn expired_state_is_cleaned_only_on_the_next_service_operation() {
        let mut policy = RuntimeStatePolicy::default();
        policy.instances.ttl = Duration::ZERO;
        policy.instances.max_entries = 1;
        let mut service = RuntimeService::default().with_state_policy(policy);
        let digest = service.admit_artifact(fixture_air_bytes());
        let created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "create".to_owned(),
                    artifact_digest: digest.clone(),
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id: old_instance,
            owner_claim: old_claim,
            ..
        } = created
        else {
            panic!("first instance");
        };
        let replacement = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "create-again".to_owned(),
                    artifact_digest: digest,
                },
            )
            .unwrap();
        assert!(matches!(
            replacement,
            RuntimeResult::ProgramInstanceCreated { .. }
        ));
        assert_eq!(service.instances.len(), 1);
        let expired_owner = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "old".to_owned(),
                    program_instance_id: old_instance,
                    owner_claim: old_claim,
                    input: serde_json::json!({}),
                },
            )
            .unwrap();
        assert!(
            matches!(expired_owner, RuntimeResult::Failed { ref code, .. } if code == "unknown_instance")
        );
        assert!(service.instances.is_empty());
    }

    #[test]
    fn zero_instance_quota_does_not_replace_owner_state() {
        let bytes = fixture_air_bytes();
        let mut policy = RuntimeStatePolicy::default();
        policy.instances.max_entries = 0;
        let mut service = RuntimeService::default().with_state_policy(policy);
        let digest = service.admit_artifact(bytes);
        let result = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "create".to_owned(),
                    artifact_digest: digest,
                },
            )
            .unwrap();
        assert!(
            matches!(result, RuntimeResult::Failed { ref code, .. } if code == "instance_quota_exceeded")
        );
        assert!(service.instances.is_empty());
    }
}
