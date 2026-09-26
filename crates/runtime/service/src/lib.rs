//! Runtime Service Composition Root.
//!
//! Accepts only a verified artifact digest plus admission inputs. It does not
//! import source-port, frontends, or the compiler.

mod composition;
mod events;
mod ports;
mod stdio;

pub use composition::{
    AdmittedPackageHandlers, ArtifactStore, CanonicalRuntimeDescriptor, InvocationMaterials,
    PackageHandlerWorkerCommand, RuntimeAdmissionProfile, RuntimeCapabilityProfile,
    artifact_digest, canonical_artifact_digest, canonical_port_bindings_digest,
    canonical_resource_ceiling_digest, canonical_runtime_descriptor, execute_admitted_artifact,
    execute_admitted_artifact_resumable_for_instance,
    execute_admitted_artifact_resumable_with_runtime_ports_and_cancellation,
    execute_admitted_artifact_with_runtime_ports,
    execute_admitted_artifact_with_runtime_ports_and_cancellation,
    execute_admitted_artifact_with_sandbox, materials_for_artifact, port_bindings_digest_for,
    resume_admitted_artifact_with_runtime_ports, runtime_descriptor_for,
    validate_package_permission_resolution, verify_invocation_materials,
    verify_invocation_materials_for_profile,
};
pub use stdio::{
    InvocationDispatcher, MAX_ACTIVE_UNIX_CONNECTIONS, MAX_FRAME_BYTES, MAX_FRAMES_PER_CONNECTION,
    RUNTIME_CHANNEL, StdioFrame, UNIX_IO_TIMEOUT_MS, UnixEndpoint, decode_jsonl, encode_jsonl,
    handshake_cross_wired, serve_stdio, serve_unix,
};

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io::Read;
use std::path::{Component, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use apxm_ais::permissions::PermissionDecision;
use apxm_capability_iface::sandbox::SandboxRegistry;
use apxm_commit_local::{
    CommitLocalError, FilesystemExecutionCommit, InMemoryExecutionCommit, ReadAccessHook,
};
use apxm_core::types::host_capability::{
    HostCapabilityOutcomeKind, HostCapabilitySettlement, is_host_capability_ref,
    is_host_capability_request_id,
};
use apxm_execution::{
    ApprovalBroker, ApprovalDecision, CancellationToken, Continuation, DenyBroker,
    ObservationRecorder,
};
use apxm_kernel::event_api::{CanonicalEventRef, EventApplication, EventApplicationResult};
use apxm_kernel::{
    ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult, ProgramInstanceRef,
};
use apxm_program::air::AirModule;
use apxm_program::artifact::ExecutableArtifact;
use apxm_runtime_protocol::{
    EventInspection, EventStatus, ExecutionObservation, ProgramInvocationStatus, ProtocolError,
    RuntimeAdmissionProfileDescriptor, RuntimeExecutionAdmissionHandshake,
    RuntimeExecutionAdmissionRequest, RuntimeFailureCode, RuntimeHandshake, RuntimeHandshakeV2,
    RuntimeOwnerClaim, RuntimeRequest, RuntimeRequestV2, RuntimeResult, RuntimeResultV2,
    capability_fulfillment_is_well_formed,
};
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
    /// Fulfilled event applications and host Capability settlement replay
    /// records.
    pub applications: StateQuota,
    /// Cancellation markers needed by an invocation while it is settling.
    pub cancellations: StateQuota,
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
    invocation_index: BTreeMap<String, String>,
    instance_bytes: u64,
    applications: BTreeMap<String, ApplicationState>,
    application_bytes: u64,
    host_capability_settlements: BTreeMap<String, HostCapabilitySettlementState>,
    host_capability_settlement_bytes: u64,
    reservations: BTreeMap<(String, u64), ReservationState>,
    event_index: events::EventIndex,
    reservation_bytes: u64,
    next_generation: u64,
    handlers: Option<AdmittedPackageHandlers>,
    package_root: Option<PathBuf>,
    sandbox_registry: Option<Arc<SandboxRegistry>>,
    /// Authoritative local commit/read adapter shared by execution and V2
    /// reads. It is the only source used for committed terminal truth.
    execution_backend: RuntimeExecutionBackend,
    /// Bounded live observation sink. It is intentionally non-authoritative;
    /// reconnect/read APIs use the commit adapter's typed records.
    observation_sink: Arc<ObservationRecorder>,
    broker: Arc<dyn ApprovalBroker>,
    /// Declared posture for an authored `Ask` on a builtin capability.
    approval_policy: ApprovalPolicy,
    cancelled: BTreeMap<String, StateEntry>,
    /// Cooperative cancellation signals for invocations currently inside the
    /// driver. Durable cancellation markers remain separate service state;
    /// this map is only the active in-process control plane.
    active_cancellations: BTreeMap<String, CancellationToken>,
    cancellation_bytes: u64,
    disconnected: bool,
    artifact_dir: Option<PathBuf>,
    state_policy: RuntimeStatePolicy,
    /// Whether protocol invocation starts persist parked continuations. The
    /// reference service uses the resumable path; the legacy embedded CLI
    /// helper can explicitly opt into single-shot execution.
    resumable_invocations: bool,
    startup_error: Option<RuntimeServiceStartupError>,
    observation_signal: ObservationSignal,
    /// Runtime-owned immutable admission carriers. When absent, invocation
    /// admission remains fail-closed.
    admission_profile: Option<RuntimeAdmissionProfile>,
    #[cfg(test)]
    resume_test_gate: Option<Arc<(Mutex<bool>, Condvar)>>,
}

/// Maximum admitted invocations that may be queued or executing effects in
/// one service process. Parked continuations do not consume this capacity.
pub(crate) const MAX_ACTIVE_INVOCATIONS: usize = 64;

/// Process-local wakeup for owner-side observation subscribers.  The durable
/// commit adapter remains the source of truth; this signal only avoids polling
/// while a long-lived Unix subscription waits for another commit.
#[derive(Clone)]
pub(crate) struct ObservationSignal(Arc<(Mutex<u64>, Condvar)>);

impl ObservationSignal {
    fn new() -> Self {
        Self(Arc::new((Mutex::new(0), Condvar::new())))
    }

    pub(crate) fn generation(&self) -> u64 {
        *self.0.0.lock().expect("observation signal lock")
    }

    pub(crate) fn notify(&self) {
        let mut generation = self.0.0.lock().expect("observation signal lock");
        *generation = generation.saturating_add(1);
        self.0.1.notify_all();
    }

    pub(crate) fn wait_for_change(&self, prior: u64) {
        let mut generation = self.0.0.lock().expect("observation signal lock");
        while *generation == prior {
            generation = self.0.1.wait(generation).expect("observation signal wait");
        }
    }
}

/// How the service resolves an authored `Ask` on a builtin capability.
///
/// APXM keeps its own broker for builtins; a host-fulfilled reference is not
/// brokered here at all (ADR-0025), so this policy never applies to one. The
/// deployment states the posture explicitly rather than inheriting whatever
/// broker a composition root happened to bind: unset means refuse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// Refuse an unresolved `Ask` immediately, without consulting a broker.
    #[default]
    Deny,
    /// Wait this long for a broker answer, then refuse.
    Timeout(Duration),
}

const APPROVAL_POLICY_DENY: &str = "deny";
const APPROVAL_POLICY_TIMEOUT_PREFIX: &str = "timeout:";

impl ApprovalPolicy {
    /// Parse one declared policy value.
    ///
    /// # Errors
    ///
    /// Returns the exact reason the value is not a policy. There is no
    /// permissive reading: an unrecognized value is a configuration error,
    /// not a silent fall back to the default.
    pub fn parse(value: &str) -> Result<Self, String> {
        let declared = value.trim();
        if declared.is_empty() || declared == APPROVAL_POLICY_DENY {
            return Ok(Self::Deny);
        }
        let Some(milliseconds) = declared.strip_prefix(APPROVAL_POLICY_TIMEOUT_PREFIX) else {
            return Err(format!(
                "{declared:?} is not a policy; expected \"deny\" or \"timeout:<ms>\""
            ));
        };
        let parsed: u64 = milliseconds.parse().map_err(|_| {
            format!(
                "{milliseconds:?} is not a whole number of milliseconds; expected \"timeout:<ms>\""
            )
        })?;
        if parsed == 0 {
            return Err(
                "\"timeout:0\" never waits; state \"deny\" to refuse an ask immediately".to_owned(),
            );
        }
        Ok(Self::Timeout(Duration::from_millis(parsed)))
    }

    /// Read the declared policy from `APXM_APPROVAL_POLICY`.
    ///
    /// # Errors
    ///
    /// Returns the parse failure so the composition root can fail closed at
    /// startup instead of serving an unstated approval posture.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var(apxm_core::constants::env::APXM_APPROVAL_POLICY) {
            Ok(value) => Self::parse(&value),
            Err(std::env::VarError::NotPresent) => Ok(Self::Deny),
            Err(std::env::VarError::NotUnicode(_)) => {
                Err("value is not valid Unicode; expected \"deny\" or \"timeout:<ms>\"".to_owned())
            }
        }
    }
}

/// Typed production startup failures. Runtime Service never silently creates
/// an ephemeral state directory because doing so would make committed reads
/// disappear across restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeServiceStartupError {
    MissingRuntimeStateDir,
    EmptyRuntimeStateDir,
    RelativeRuntimeStateDir(PathBuf),
    UnsafeRuntimeStateDir(PathBuf),
    OpenRuntimeStateDir(String),
    InvalidAdmissionProfile(String),
    InvalidApprovalPolicy(String),
}

impl std::fmt::Display for RuntimeServiceStartupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRuntimeStateDir => {
                formatter.write_str("APXM_RUNTIME_STATE_DIR is required")
            }
            Self::EmptyRuntimeStateDir => {
                formatter.write_str("APXM_RUNTIME_STATE_DIR must not be empty")
            }
            Self::RelativeRuntimeStateDir(path) => write!(
                formatter,
                "APXM_RUNTIME_STATE_DIR must be absolute: {}",
                path.display()
            ),
            Self::UnsafeRuntimeStateDir(path) => write!(
                formatter,
                "APXM_RUNTIME_STATE_DIR contains an unsafe parent component: {}",
                path.display()
            ),
            Self::OpenRuntimeStateDir(error) => {
                write!(
                    formatter,
                    "cannot open APXM runtime state directory: {error}"
                )
            }
            Self::InvalidAdmissionProfile(error) => {
                write!(formatter, "invalid APXM runtime admission profile: {error}")
            }
            Self::InvalidApprovalPolicy(error) => {
                write!(formatter, "invalid APXM_APPROVAL_POLICY: {error}")
            }
        }
    }
}

impl std::error::Error for RuntimeServiceStartupError {}

impl Default for RuntimeService {
    fn default() -> Self {
        // Keep the library default deterministic for embedded/unit callers.
        // Production composition roots must opt into `try_from_env` (or
        // `from_env`) so an absent state directory cannot be hidden.
        Self::in_memory()
    }
}

impl RuntimeService {
    fn unconfigured() -> Self {
        let observation_signal = ObservationSignal::new();
        let observation_sink = Arc::new(ObservationRecorder::default());
        let signal_for_observer = observation_signal.clone();
        observation_sink.set_notifier(Arc::new(move || signal_for_observer.notify()));
        Self {
            artifacts: ArtifactStore::default(),
            artifact_meta: BTreeMap::new(),
            artifact_bytes: 0,
            artifact_admissions: BTreeMap::new(),
            admission_meta: BTreeMap::new(),
            admission_bytes: 0,
            instances: BTreeMap::new(),
            invocation_index: BTreeMap::new(),
            instance_bytes: 0,
            applications: BTreeMap::new(),
            application_bytes: 0,
            host_capability_settlements: BTreeMap::new(),
            event_index: Default::default(),
            host_capability_settlement_bytes: 0,
            reservations: BTreeMap::new(),
            reservation_bytes: 0,
            next_generation: 0,
            handlers: None,
            package_root: None,
            sandbox_registry: None,
            execution_backend: RuntimeExecutionBackend::Unavailable(
                "runtime state has not been opened".to_owned(),
            ),
            observation_sink,
            broker: Arc::new(DenyBroker),
            approval_policy: ApprovalPolicy::Deny,
            cancelled: BTreeMap::new(),
            active_cancellations: BTreeMap::new(),
            cancellation_bytes: 0,
            disconnected: false,
            artifact_dir: None,
            state_policy: RuntimeStatePolicy::default(),
            resumable_invocations: true,
            startup_error: None,
            observation_signal,
            admission_profile: None,
            #[cfg(test)]
            resume_test_gate: None,
        }
    }

    /// Product handler. Loads committed artifacts from `APXM_ARTIFACT_DIR`.
    #[must_use]
    pub fn from_env() -> Self {
        let mut service = Self::unconfigured();
        match runtime_state_dir_from_env() {
            Ok(root) => match FilesystemExecutionCommit::open(root.clone()) {
                Ok(commit) => {
                    service.execution_backend =
                        RuntimeExecutionBackend::Filesystem(Arc::new(commit));
                    if let Err(error) = service.rehydrate_runtime_state() {
                        service.startup_error = Some(
                            RuntimeServiceStartupError::OpenRuntimeStateDir(error.clone()),
                        );
                        service.execution_backend = RuntimeExecutionBackend::Unavailable(error);
                    }
                }
                Err(error) => {
                    service.execution_backend =
                        RuntimeExecutionBackend::Unavailable(error.to_string());
                    service.startup_error = Some(RuntimeServiceStartupError::OpenRuntimeStateDir(
                        error.to_string(),
                    ));
                }
            },
            Err(error) => {
                service.execution_backend = RuntimeExecutionBackend::Unavailable(error.to_string());
                service.startup_error = Some(error);
            }
        }
        if let Ok(dir) = std::env::var("APXM_ARTIFACT_DIR")
            && !dir.trim().is_empty()
        {
            service.artifact_dir = Some(PathBuf::from(dir));
        }
        let capability_profile = RuntimeCapabilityProfile::from_env();
        match capability_profile.and_then(RuntimeAdmissionProfile::from_env_for) {
            Ok(profile) => service.admission_profile = profile,
            Err(error) => {
                service.startup_error =
                    Some(RuntimeServiceStartupError::InvalidAdmissionProfile(error));
            }
        }
        match ApprovalPolicy::from_env() {
            Ok(policy) => service.approval_policy = policy,
            // A declared-but-unreadable approval posture is a startup failure
            // of its own; it never downgrades to the default, and it never
            // hides an earlier failure the operator still has to fix.
            Err(error) if service.startup_error.is_none() => {
                service.startup_error =
                    Some(RuntimeServiceStartupError::InvalidApprovalPolicy(error));
            }
            Err(_) => {}
        }
        service
    }

    /// Fallible production constructor. A stable, absolute runtime state
    /// directory is mandatory so restart reads retain their authority.
    pub fn try_from_env() -> Result<Self, RuntimeServiceStartupError> {
        let service = Self::from_env();
        if let Some(error) = service.startup_error.clone() {
            Err(error)
        } else {
            Ok(service)
        }
    }

    /// Return the startup failure retained by the non-fallible compatibility
    /// constructor, if production state could not be opened.
    #[must_use]
    pub fn startup_error(&self) -> Option<&RuntimeServiceStartupError> {
        self.startup_error.as_ref()
    }

    /// Load admitted artifacts from a shared directory written by Compilation Service.
    #[must_use]
    pub fn with_artifact_dir(mut self, dir: PathBuf) -> Self {
        self.artifact_dir = Some(dir);
        self
    }

    /// Compose one explicit Runtime-owned admission profile. Production
    /// standalone composition normally loads this from the image carriers.
    #[must_use]
    pub fn with_admission_profile(mut self, profile: RuntimeAdmissionProfile) -> Self {
        self.admission_profile = Some(profile);
        self
    }

    fn selected_capability_profile(&self) -> RuntimeCapabilityProfile {
        self.admission_profile.as_ref().map_or(
            RuntimeCapabilityProfile::PortableLocal,
            RuntimeAdmissionProfile::capability_profile,
        )
    }

    fn verify_selected_materials(
        &self,
        artifact_bytes: &[u8],
        materials: &InvocationMaterials,
    ) -> Result<(), String> {
        if let Some(profile) = self.admission_profile.as_ref() {
            let expected = profile.materials_for_artifact(artifact_bytes);
            if materials.release_bytes != expected.release_bytes
                || materials.provenance_bytes != expected.provenance_bytes
                || materials.admission.port_bindings_digest
                    != expected.admission.port_bindings_digest
                || materials.admission.resource_ceiling_digest
                    != expected.admission.resource_ceiling_digest
            {
                return Err("admission_profile_mismatch".to_owned());
            }
        }
        verify_invocation_materials_for_profile(
            artifact_bytes,
            materials,
            self.selected_capability_profile(),
        )?;
        Ok(())
    }

    fn same_binding(left: &InvocationMaterials, right: &InvocationMaterials) -> bool {
        let mut normalized = right.clone();
        normalized
            .admission
            .invocation_id
            .clone_from(&left.admission.invocation_id);
        left == &normalized
    }

    /// Bind the service to one explicit runtime-owned state directory. This
    /// is useful for an embedding composition root and for restart tests; the
    /// directory contains the durable commit/read record, never artifacts.
    #[must_use]
    pub fn with_runtime_state_dir(mut self, dir: PathBuf) -> Self {
        self.execution_backend = match validate_runtime_state_dir(dir) {
            Err(error) => {
                self.startup_error = Some(error.clone());
                RuntimeExecutionBackend::Unavailable(error.to_string())
            }
            Ok(dir) => match FilesystemExecutionCommit::open(dir.clone()) {
                Ok(commit) => {
                    self.execution_backend = RuntimeExecutionBackend::Filesystem(Arc::new(commit));
                    if let Err(error) = self.rehydrate_runtime_state() {
                        self.startup_error = Some(RuntimeServiceStartupError::OpenRuntimeStateDir(
                            error.clone(),
                        ));
                        self.execution_backend = RuntimeExecutionBackend::Unavailable(error);
                    } else {
                        self.startup_error = None;
                    }
                    return self;
                }
                Err(error) => {
                    self.startup_error = Some(RuntimeServiceStartupError::OpenRuntimeStateDir(
                        error.to_string(),
                    ));
                    RuntimeExecutionBackend::Unavailable(error.to_string())
                }
            },
        };
        self
    }

    /// Apply an explicit bounded state policy before serving requests.
    #[must_use]
    pub fn with_state_policy(mut self, policy: RuntimeStatePolicy) -> Self {
        self.state_policy = policy;
        self
    }

    /// Use the legacy single-shot execution profile for an embedded caller.
    /// This does not change the Runtime protocol's resumable service path and
    /// is retained only for callers that require an immediate committed output
    /// from a fixture that contains an event wait.
    #[must_use]
    pub fn with_single_shot_invocations(mut self) -> Self {
        self.resumable_invocations = false;
        self
    }

    /// The active bounded state policy.
    #[must_use]
    pub fn state_policy(&self) -> &RuntimeStatePolicy {
        &self.state_policy
    }

    /// Obtain the non-authoritative wakeup used by the Unix stream adapter.
    pub(crate) fn observation_signal(&self) -> ObservationSignal {
        self.observation_signal.clone()
    }

    /// Replace the composition-owned reauthorization/audit hook used by typed
    /// execution reads. APXM does not implement product authorization; the
    /// supplied hook is invoked for every read operation.
    #[must_use]
    pub fn with_read_access_hook(mut self, hook: Arc<dyn ReadAccessHook>) -> Self {
        self.execution_backend = self.execution_backend.with_read_access_hook(hook);
        self
    }

    /// Bind the caller-supplied scope used for committed output references.
    /// This is a composition seam; APXM does not interpret the scope.
    #[must_use]
    pub fn with_output_access_scope_ref(self, reference: String) -> Self {
        self.execution_backend
            .set_default_access_scope_ref(reference);
        self
    }

    /// Explicitly authorize reads for an embedded owner-local conformance
    /// composition. Production services must bind their own policy hook.
    #[must_use]
    pub fn with_embedded_read_access(self) -> Self {
        self.with_read_access_hook(Arc::new(apxm_commit_local::AllowReadAccess))
    }

    /// Construct an explicitly in-memory service for unit and protocol tests.
    /// Production composition uses [`RuntimeService::from_env`], which opens
    /// the filesystem-backed owner-local commit adapter.
    #[must_use]
    pub fn in_memory() -> Self {
        let mut service = Self::unconfigured();
        service.execution_backend =
            RuntimeExecutionBackend::Memory(Arc::new(InMemoryExecutionCommit::new()));
        service.startup_error = None;
        if let Ok(dir) = std::env::var("APXM_ARTIFACT_DIR")
            && !dir.trim().is_empty()
        {
            service.artifact_dir = Some(PathBuf::from(dir));
        }
        service
    }
}

/// The service's injected execution/read backend. Keeping this choice at the
/// composition root prevents Protocol/2 reads and the driver from observing
/// different stores, while making in-memory state an explicit test choice.
enum RuntimeExecutionBackend {
    Memory(Arc<InMemoryExecutionCommit>),
    Filesystem(Arc<FilesystemExecutionCommit>),
    Unavailable(String),
}

impl RuntimeExecutionBackend {
    fn commit_port(&self) -> Arc<dyn ExecutionCommitPort> {
        match self {
            Self::Memory(commit) => commit.clone(),
            Self::Filesystem(commit) => commit.clone(),
            Self::Unavailable(reason) => Arc::new(UnavailableExecutionCommit {
                reason: reason.clone(),
            }),
        }
    }

    fn read_execution_with_live(
        &self,
        request: apxm_runtime_protocol::ExecutionReadRequest,
        live_observations: &[ExecutionObservation],
    ) -> Result<apxm_runtime_protocol::ExecutionReadResult, CommitLocalError> {
        match self {
            Self::Memory(commit) => commit.read_execution_with_live(request, live_observations),
            Self::Filesystem(commit) => commit.read_execution_with_live(request, live_observations),
            Self::Unavailable(reason) => Err(CommitLocalError::Io(reason.clone())),
        }
    }

    fn with_read_access_hook(self, hook: Arc<dyn ReadAccessHook>) -> Self {
        match self {
            Self::Memory(commit) => {
                commit.set_read_access_hook(hook);
                Self::Memory(commit)
            }
            Self::Filesystem(commit) => {
                let root = commit.root().to_path_buf();
                drop(commit);
                match FilesystemExecutionCommit::open_with_read_access_hook(root, hook) {
                    Ok(commit) => Self::Filesystem(Arc::new(commit)),
                    Err(error) => Self::Unavailable(error.to_string()),
                }
            }
            Self::Unavailable(reason) => Self::Unavailable(reason),
        }
    }

    fn set_default_access_scope_ref(&self, reference: String) {
        match self {
            Self::Memory(commit) => commit.set_default_access_scope_ref(reference),
            Self::Filesystem(commit) => commit.set_default_access_scope_ref(reference),
            Self::Unavailable(_) => {}
        }
    }

    fn runtime_metadata(&self) -> Option<Value> {
        match self {
            Self::Memory(commit) => commit.runtime_metadata(),
            Self::Filesystem(commit) => commit.runtime_metadata(),
            Self::Unavailable(_) => None,
        }
    }

    fn set_runtime_metadata(&self, metadata: Option<Value>) -> Result<(), String> {
        match self {
            Self::Memory(commit) => {
                commit.set_runtime_metadata(metadata);
                Ok(())
            }
            Self::Filesystem(commit) => commit
                .set_runtime_metadata(metadata)
                .map_err(|error| error.to_string()),
            Self::Unavailable(reason) => Err(reason.clone()),
        }
    }

    fn invocation_status(
        &self,
        invocation: &str,
    ) -> Option<apxm_runtime_protocol::ProgramInvocationStatus> {
        match self {
            Self::Memory(commit) => commit.invocation_status(invocation),
            Self::Filesystem(commit) => commit.invocation_status(invocation),
            Self::Unavailable(_) => None,
        }
    }

    fn terminal_failure_code(&self, invocation: &str) -> Option<String> {
        match self {
            Self::Memory(commit) => commit.terminal_failure_code(invocation),
            Self::Filesystem(commit) => commit.terminal_failure_code(invocation),
            Self::Unavailable(_) => None,
        }
    }

    fn load_continuation(
        &self,
        program_instance_ref: &ProgramInstanceRef,
    ) -> Option<apxm_kernel::CommittedContinuation> {
        match self {
            Self::Memory(commit) => {
                block_on(commit.load_continuation_with_integrity(program_instance_ref))
            }
            Self::Filesystem(commit) => {
                block_on(commit.load_continuation_with_integrity(program_instance_ref))
            }
            Self::Unavailable(_) => None,
        }
    }
}

fn runtime_state_dir_from_env() -> Result<PathBuf, RuntimeServiceStartupError> {
    runtime_state_dir_from_value(std::env::var_os("APXM_RUNTIME_STATE_DIR"))
}

fn runtime_state_dir_from_value(
    value: Option<std::ffi::OsString>,
) -> Result<PathBuf, RuntimeServiceStartupError> {
    let value = value.ok_or(RuntimeServiceStartupError::MissingRuntimeStateDir)?;
    if value.to_string_lossy().trim().is_empty() {
        return Err(RuntimeServiceStartupError::EmptyRuntimeStateDir);
    }
    validate_runtime_state_dir(PathBuf::from(value))
}

fn validate_runtime_state_dir(path: PathBuf) -> Result<PathBuf, RuntimeServiceStartupError> {
    if !path.is_absolute() {
        return Err(RuntimeServiceStartupError::RelativeRuntimeStateDir(path));
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(RuntimeServiceStartupError::UnsafeRuntimeStateDir(path));
    }
    Ok(path)
}

struct UnavailableExecutionCommit {
    reason: String,
}

#[async_trait]
impl ExecutionCommitPort for UnavailableExecutionCommit {
    async fn prepare_output(
        &self,
        _preparation: apxm_kernel::SessionOutputPreparation,
    ) -> Result<apxm_kernel::PreparedSessionOutputRef, String> {
        Err(self.reason.clone())
    }

    async fn commit(&self, _request: ExecutionCommitRequest) -> ExecutionCommitResult {
        ExecutionCommitResult::OutcomeUnknown {
            reconciliation_ref: format!("reconcile:backend-unavailable:{}", self.reason),
        }
    }

    async fn current_version(&self, _program_instance_ref: &ProgramInstanceRef) -> u64 {
        0
    }

    async fn load_continuation(&self, _program_instance_ref: &ProgramInstanceRef) -> Option<Value> {
        None
    }

    async fn load_continuation_with_integrity(
        &self,
        _program_instance_ref: &ProgramInstanceRef,
    ) -> Option<apxm_kernel::CommittedContinuation> {
        None
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
    /// Immutable request-id history retained after a terminal invocation so
    /// retries replay their original result even after the instance admits a
    /// later invocation.
    invocation_history: BTreeMap<String, InvocationState>,
    invocation_bytes: u64,
    state_entry: StateEntry,
}

#[derive(Clone)]
struct InvocationState {
    request_id: String,
    input_fingerprint: String,
    /// Exact bounded input retained until the invocation settles, so a process
    /// restart can continue an admitted invocation under the same identity.
    input: Option<Value>,
    program_invocation_id: String,
    phase: InvocationExecutionPhase,
    /// `None` while the owner is executing outside the service mutex.
    result: Option<RuntimeResult>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum InvocationExecutionPhase {
    /// The invocation identity and input are durable, but no worker has been
    /// allowed to reach an effect boundary.
    #[default]
    Pending,
    /// A worker crossed its durable start marker. A restart must reconcile or
    /// mark uncertainty; it must never redispatch this invocation blindly.
    Running,
}

/// All immutable/materialized inputs needed to execute one invocation after
/// its service-owned claim has been installed.  The lease contains no mutable
/// service maps; cancellation and finalization remain serialized by the
/// RuntimeService mutex while the driver itself runs independently.
pub(crate) struct PreparedInvocation {
    request_id: String,
    program_instance_id: String,
    invocation_id: String,
    /// The caller-supplied invocation input, bound to the artifact's exact
    /// entrypoint parameter by the composition root before the driver starts.
    input: Value,
    air: AirModule,
    artifact_bytes: Vec<u8>,
    materials: InvocationMaterials,
    handlers: Option<AdmittedPackageHandlers>,
    package_root: Option<PathBuf>,
    sandbox_registry: Option<Arc<SandboxRegistry>>,
    execution_backend: Arc<dyn ExecutionCommitPort>,
    observation_sink: Arc<ObservationRecorder>,
    broker: Arc<dyn ApprovalBroker>,
    approval_policy: ApprovalPolicy,
    cancellation: CancellationToken,
    resumable: bool,
    events: Arc<dyn apxm_kernel::EventPort>,
}

impl PreparedInvocation {
    pub(crate) fn execute(&self) -> Result<Value, String> {
        block_on(async {
            for (capability_ref, decision) in &self.air.capability_permission_requests {
                if !matches!(decision, PermissionDecision::Ask { .. }) {
                    continue;
                }
                // Permission for a host-fulfilled reference is the host's
                // decision (ADR-0025). APXM records the authored request and
                // carries it to the host in `capability.requested`; brokering
                // it here would answer a question that is not APXM's to answer,
                // and would refuse the invocation before the host ever saw it.
                if is_host_capability_ref(capability_ref) {
                    continue;
                }
                // The deployment states how long, if at all, a builtin `Ask`
                // may wait for an answer. `Deny` refuses without asking: with
                // no external answerer, waiting only defers the same refusal.
                match self.approval_policy {
                    ApprovalPolicy::Deny => return Err("ask_denied".to_owned()),
                    ApprovalPolicy::Timeout(wait) => {
                        match tokio::time::timeout(wait, self.broker.resolve_ask(capability_ref))
                            .await
                        {
                            Ok(ApprovalDecision::Allow) => {}
                            Ok(ApprovalDecision::Deny) => return Err("ask_denied".to_owned()),
                            Ok(ApprovalDecision::Timeout) | Err(_) => {
                                return Err("ask_timeout".to_owned());
                            }
                        }
                    }
                }
            }
            if self.resumable {
                composition::execute_admitted_artifact_resumable_for_instance_with_input(
                    self.air.clone(),
                    &self.artifact_bytes,
                    &self.materials,
                    self.handlers.as_ref(),
                    self.package_root.as_deref(),
                    self.sandbox_registry.clone(),
                    self.execution_backend.clone(),
                    Some(self.observation_sink.clone()),
                    Some(self.cancellation.clone()),
                    ProgramInstanceRef::new(self.program_instance_id.clone()),
                    self.input.clone(),
                    self.events.clone(),
                )
                .await
            } else {
                composition::execute_admitted_artifact_with_runtime_ports_and_cancellation_with_input(
                    self.air.clone(),
                    &self.artifact_bytes,
                    &self.materials,
                    self.handlers.as_ref(),
                    self.package_root.as_deref(),
                    self.sandbox_registry.clone(),
                    self.execution_backend.clone(),
                    Some(self.observation_sink.clone()),
                    Some(self.cancellation.clone()),
                    self.input.clone(),
                )
                .await
            }
        })
    }
}

/// Immutable lease for resuming one durably settled host Capability or reserved
/// Event outside the service mutex. Both use the same persisted start fence,
/// cancellation, bounded dispatcher and execution commit/recovery discipline.
pub(crate) struct PreparedResume {
    event_ref: apxm_kernel::EventRef,
    events: Arc<dyn apxm_kernel::EventPort>,
    invocation_request_id: String,
    program_instance_id: String,
    invocation_id: String,
    delivered: Value,
    continuation: Continuation,
    committed_continuation: apxm_kernel::CommittedContinuation,
    expected_program_state_version: u64,
    artifact_bytes: Vec<u8>,
    materials: InvocationMaterials,
    handlers: Option<AdmittedPackageHandlers>,
    package_root: Option<PathBuf>,
    sandbox_registry: Option<Arc<SandboxRegistry>>,
    execution_backend: Arc<dyn ExecutionCommitPort>,
    observation_sink: Arc<ObservationRecorder>,
    cancellation: CancellationToken,
    #[cfg(test)]
    test_gate: Option<Arc<(Mutex<bool>, Condvar)>>,
}

impl PreparedResume {
    pub(crate) fn execute(&self) -> Result<Value, String> {
        #[cfg(test)]
        if let Some(gate) = &self.test_gate {
            let (released, signal) = gate.as_ref();
            let mut released = released.lock().expect("resume test gate lock");
            while !*released {
                released = signal.wait(released).expect("resume test gate lock");
            }
        }
        block_on(composition::resume_admitted_artifact_with_events(
            self.continuation.air.clone(),
            &self.artifact_bytes,
            &self.materials,
            self.handlers.as_ref(),
            self.package_root.as_deref(),
            self.sandbox_registry.clone(),
            self.execution_backend.clone(),
            Some(self.observation_sink.clone()),
            Some(self.cancellation.clone()),
            ProgramInstanceRef::new(self.program_instance_id.clone()),
            self.event_ref.clone(),
            self.delivered.clone(),
            Some(self.events.clone()),
        ))
    }
}

struct ReservationState {
    request_id: String,
    program_instance_id: String,
    owner_claim: RuntimeOwnerClaim,
    type_id: String,
    payload_schema: apxm_program::input_schema::EntrypointInputSchema,
    schema_digest: String,
    binding: Option<apxm_kernel::event_api::EventWaitBinding>,
    resume_started: bool,
    status: EventStatus,
    occurrence_id: Option<String>,
    state_entry: StateEntry,
}

struct ApplicationState {
    event_ref: CanonicalEventRef,
    occurrence_id: String,
    source_kind: String,
    mapping_digest: String,
    source_record: String,
    payload: Value,
    state_entry: StateEntry,
}

struct HostCapabilitySettlementState {
    program_instance_id: String,
    owner_claim: RuntimeOwnerClaim,
    settlement: HostCapabilitySettlement,
    /// Set durably immediately before a worker resumes the parked
    /// continuation. Once true, restart may reconcile but must not resend.
    resume_started: bool,
    state_entry: StateEntry,
}

const RUNTIME_METADATA_SCHEMA: &str = "apxm.runtime-service.metadata.v1";

/// Only the service-owned durable metadata uses this representation. Public
/// invocation materials keep their existing wire encoding.
const DURABLE_BYTES_PREFIX: &str = "base64:";
const MAX_DURABLE_CARRIER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
struct DurableBytes<const MAX: usize>(Vec<u8>);

impl<const MAX: usize> Serialize for DurableBytes<MAX> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.0.len() > MAX {
            return Err(serde::ser::Error::custom(
                "durable bytes exceed their bound",
            ));
        }
        serializer.serialize_str(&format!("{DURABLE_BYTES_PREFIX}{}", BASE64.encode(&self.0)))
    }
}

impl<'de, const MAX: usize> Deserialize<'de> for DurableBytes<MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BytesVisitor<const MAX: usize>;

        impl<'de, const MAX: usize> Visitor<'de> for BytesVisitor<MAX> {
            type Value = DurableBytes<MAX>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("bounded base64 durable bytes or a legacy byte array")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                let encoded = value
                    .strip_prefix(DURABLE_BYTES_PREFIX)
                    .ok_or_else(|| E::custom("unknown durable byte encoding"))?;
                let max_encoded = MAX.div_ceil(3).saturating_mul(4);
                if encoded.len() > max_encoded {
                    return Err(E::custom("encoded durable bytes exceed their bound"));
                }
                let bytes = BASE64
                    .decode(encoded)
                    .map_err(|_| E::custom("invalid durable base64 bytes"))?;
                if bytes.len() > MAX || BASE64.encode(&bytes) != encoded {
                    return Err(E::custom("noncanonical or oversized durable bytes"));
                }
                Ok(DurableBytes(bytes))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                self.visit_str(&value)
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut bytes = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(MAX));
                while let Some(byte) = sequence.next_element::<u8>()? {
                    if bytes.len() == MAX {
                        return Err(serde::de::Error::custom(
                            "legacy durable bytes exceed their bound",
                        ));
                    }
                    bytes.push(byte);
                }
                Ok(DurableBytes(bytes))
            }
        }

        deserializer.deserialize_any(BytesVisitor::<MAX>)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableInvocationMaterials {
    admission: apxm_kernel::admission::InvocationAdmission,
    release_bytes: DurableBytes<MAX_DURABLE_CARRIER_BYTES>,
    provenance_bytes: DurableBytes<MAX_DURABLE_CARRIER_BYTES>,
}

impl From<&InvocationMaterials> for DurableInvocationMaterials {
    fn from(materials: &InvocationMaterials) -> Self {
        Self {
            admission: materials.admission.clone(),
            release_bytes: DurableBytes(materials.release_bytes.clone()),
            provenance_bytes: DurableBytes(materials.provenance_bytes.clone()),
        }
    }
}

impl From<DurableInvocationMaterials> for InvocationMaterials {
    fn from(materials: DurableInvocationMaterials) -> Self {
        Self {
            admission: materials.admission,
            release_bytes: materials.release_bytes.0,
            provenance_bytes: materials.provenance_bytes.0,
        }
    }
}

/// Durable service-owned metadata. The commit-local adapter stores this as
/// opaque JSON and authenticates it together with execution records; this
/// type is the only owner that interprets it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableRuntimeMetadata {
    schema_version: String,
    artifacts: BTreeMap<String, DurableBytes<{ MAX_ARTIFACT_BYTES as usize }>>,
    artifact_admissions: BTreeMap<String, DurableInvocationMaterials>,
    instances: BTreeMap<String, DurableInstanceState>,
    reservations: Vec<DurableReservationState>,
    applications: BTreeMap<String, DurableApplicationState>,
    #[serde(default)]
    host_capability_settlements: BTreeMap<String, DurableHostCapabilitySettlementState>,
    cancelled: BTreeSet<String>,
    next_generation: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableInstanceState {
    artifact_digest: String,
    materials: Option<DurableInvocationMaterials>,
    owner_claim: RuntimeOwnerClaim,
    invocation: Option<DurableInvocationState>,
    #[serde(default)]
    invocation_history: BTreeMap<String, DurableInvocationState>,
    invocation_bytes: u64,
    state_entry: DurableStateEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableInvocationState {
    request_id: String,
    input_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<Value>,
    /// JSON `null` is a valid invocation input but deserializes as `None` for
    /// `Option<Value>`. This presence bit preserves that distinction while
    /// retaining compatibility with metadata written before inputs were kept.
    #[serde(default)]
    input_present: bool,
    program_invocation_id: String,
    #[serde(default)]
    phase: InvocationExecutionPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<RuntimeResult>,
}

fn durable_invocation(invocation: &InvocationState) -> DurableInvocationState {
    DurableInvocationState {
        request_id: invocation.request_id.clone(),
        input_fingerprint: invocation.input_fingerprint.clone(),
        input: invocation.input.clone(),
        input_present: invocation.input.is_some(),
        program_invocation_id: invocation.program_invocation_id.clone(),
        phase: invocation.phase,
        result: invocation.result.clone(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableReservationState {
    event_id: String,
    generation: u64,
    request_id: String,
    program_instance_id: String,
    owner_claim: RuntimeOwnerClaim,
    type_id: String,
    payload_schema: apxm_program::input_schema::EntrypointInputSchema,
    schema_digest: String,
    binding: Option<apxm_kernel::event_api::EventWaitBinding>,
    resume_started: bool,
    status: EventStatus,
    occurrence_id: Option<String>,
    state_entry: DurableStateEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableApplicationState {
    event_ref: CanonicalEventRef,
    occurrence_id: String,
    source_kind: String,
    mapping_digest: String,
    source_record: String,
    payload: Value,
    state_entry: DurableStateEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableHostCapabilitySettlementState {
    program_instance_id: String,
    owner_claim: RuntimeOwnerClaim,
    settlement: HostCapabilitySettlement,
    #[serde(default)]
    resume_started: bool,
    state_entry: DurableStateEntry,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableStateEntry {
    bytes: u64,
    expires_at_ms: u64,
}

fn runtime_failure_code(error: &CommitLocalError) -> RuntimeFailureCode {
    match error {
        CommitLocalError::Unauthorized(_) => RuntimeFailureCode::Unauthorized,
        CommitLocalError::RetentionGap { .. } => RuntimeFailureCode::RetentionGap,
        CommitLocalError::InvalidRead(_)
        | CommitLocalError::InvalidCursor(_)
        | CommitLocalError::InvalidRequest(_) => RuntimeFailureCode::InvalidRequest,
        CommitLocalError::OutputNotPrepared { .. }
        | CommitLocalError::OutputScopeMismatch { .. }
        | CommitLocalError::ProvisionalOutput => RuntimeFailureCode::NotFound,
        _ => RuntimeFailureCode::Unavailable,
    }
}

fn invocation_result_from_execution_output(
    request_id: String,
    program_invocation_id: String,
    output: &Value,
) -> RuntimeResult {
    // A committed report carries the commit transport status separately from
    // the execution terminal status. A cancelled execution therefore has a
    // committed commit record but must remain Cancelled at the service result
    // boundary; looking only at `commit.status` would mislabel it as started.
    if output.get("status").and_then(Value::as_str) == Some("cancelled") {
        return RuntimeResult::Cancelled { request_id };
    }
    match output
        .get("commit")
        .and_then(|commit| commit.get("status"))
        .and_then(Value::as_str)
    {
        Some("committed" | "suspended") => RuntimeResult::ProgramInvocationStarted {
            request_id,
            program_invocation_id,
        },
        Some("compare_conflict") => RuntimeResult::Failed {
            request_id,
            code: "compare_conflict".to_owned(),
        },
        Some("failed") => RuntimeResult::Failed {
            request_id,
            code: "invocation_failed".to_owned(),
        },
        Some("cancelled") => RuntimeResult::Cancelled { request_id },
        Some("outcome_unknown" | _) | None => RuntimeResult::Failed {
            request_id,
            code: "outcome_unknown".to_owned(),
        },
    }
}

fn execution_admission_request_id(request: &RuntimeExecutionAdmissionRequest) -> String {
    match request {
        RuntimeExecutionAdmissionRequest::ProgramInstanceBindAdmission { request_id, .. } => {
            request_id.clone()
        }
    }
}

impl RuntimeService {
    fn now() -> Instant {
        // Instant is monotonic, so expiry is not affected by wall-clock
        // adjustments or an operator changing the system time.
        Instant::now()
    }

    fn epoch_millis() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .unwrap_or(0)
    }

    fn durable_entry(entry: StateEntry) -> DurableStateEntry {
        let remaining = entry
            .expires_at
            .checked_duration_since(Self::now())
            .unwrap_or_default();
        DurableStateEntry {
            bytes: entry.bytes,
            expires_at_ms: Self::epoch_millis()
                .saturating_add(u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX)),
        }
    }

    fn restore_entry(entry: DurableStateEntry, ttl: Duration) -> StateEntry {
        let now_ms = Self::epoch_millis();
        let remaining_ms = entry.expires_at_ms.saturating_sub(now_ms);
        let remaining = Duration::from_millis(remaining_ms).min(ttl);
        StateEntry {
            bytes: entry.bytes,
            expires_at: Self::now().checked_add(remaining).unwrap_or_else(Self::now),
        }
    }

    fn runtime_metadata(&self) -> DurableRuntimeMetadata {
        DurableRuntimeMetadata {
            schema_version: RUNTIME_METADATA_SCHEMA.to_owned(),
            artifacts: self
                .artifact_meta
                .keys()
                .filter_map(|digest| {
                    self.artifacts
                        .get(digest)
                        .map(|bytes| (digest.clone(), DurableBytes(bytes.to_vec())))
                })
                .collect(),
            artifact_admissions: self
                .artifact_admissions
                .iter()
                .map(|(digest, materials)| (digest.clone(), materials.into()))
                .collect(),
            instances: self
                .instances
                .iter()
                .map(|(id, instance)| {
                    (
                        id.clone(),
                        DurableInstanceState {
                            artifact_digest: instance.artifact_digest.clone(),
                            materials: instance
                                .materials
                                .as_ref()
                                .map(DurableInvocationMaterials::from),
                            owner_claim: instance.owner_claim.clone(),
                            invocation: instance.invocation.as_ref().map(durable_invocation),
                            invocation_history: instance
                                .invocation_history
                                .iter()
                                .map(|(request_id, invocation)| {
                                    (request_id.clone(), durable_invocation(invocation))
                                })
                                .collect(),
                            invocation_bytes: instance.invocation_bytes,
                            state_entry: Self::durable_entry(instance.state_entry),
                        },
                    )
                })
                .collect(),
            reservations: self
                .reservations
                .iter()
                .map(
                    |((event_id, generation), reservation)| DurableReservationState {
                        event_id: event_id.clone(),
                        generation: *generation,
                        request_id: reservation.request_id.clone(),
                        program_instance_id: reservation.program_instance_id.clone(),
                        owner_claim: reservation.owner_claim.clone(),
                        type_id: reservation.type_id.clone(),
                        payload_schema: reservation.payload_schema.clone(),
                        schema_digest: reservation.schema_digest.clone(),
                        binding: reservation.binding.clone(),
                        resume_started: reservation.resume_started,
                        status: reservation.status,
                        occurrence_id: reservation.occurrence_id.clone(),
                        state_entry: Self::durable_entry(reservation.state_entry),
                    },
                )
                .collect(),
            applications: self
                .applications
                .iter()
                .map(|(key, application)| {
                    (
                        key.clone(),
                        DurableApplicationState {
                            event_ref: application.event_ref.clone(),
                            occurrence_id: application.occurrence_id.clone(),
                            source_kind: application.source_kind.clone(),
                            mapping_digest: application.mapping_digest.clone(),
                            source_record: application.source_record.clone(),
                            payload: application.payload.clone(),
                            state_entry: Self::durable_entry(application.state_entry),
                        },
                    )
                })
                .collect(),
            host_capability_settlements: self
                .host_capability_settlements
                .iter()
                .map(|(capability_request_id, state)| {
                    (
                        capability_request_id.clone(),
                        DurableHostCapabilitySettlementState {
                            program_instance_id: state.program_instance_id.clone(),
                            owner_claim: state.owner_claim.clone(),
                            settlement: state.settlement.clone(),
                            resume_started: state.resume_started,
                            state_entry: Self::durable_entry(state.state_entry),
                        },
                    )
                })
                .collect(),
            cancelled: self.cancelled.keys().cloned().collect(),
            next_generation: self.next_generation,
        }
    }

    fn persist_runtime_state(&self) -> Result<(), String> {
        let metadata = serde_json::to_value(self.runtime_metadata())
            .map_err(|error| format!("runtime metadata encode failed: {error}"))?;
        self.execution_backend
            .set_runtime_metadata(Some(metadata))?;
        self.refresh_event_index()
    }

    fn rehydrate_runtime_state(&mut self) -> Result<(), String> {
        let Some(value) = self.execution_backend.runtime_metadata() else {
            return Ok(());
        };
        let metadata: DurableRuntimeMetadata = serde_json::from_value(value)
            .map_err(|error| format!("runtime metadata decode failed: {error}"))?;
        if metadata.schema_version != RUNTIME_METADATA_SCHEMA {
            return Err(format!(
                "runtime metadata schema mismatch: {}",
                metadata.schema_version
            ));
        }

        let mut artifacts = ArtifactStore::default();
        let mut artifact_meta = BTreeMap::new();
        let mut artifact_bytes = 0_u64;
        for (digest, DurableBytes(bytes)) in metadata.artifacts {
            if !apxm_core::grammar::is_digest(&digest)
                || ExecutableArtifact::decode_for_execution(&bytes, &digest).is_err()
            {
                return Err("runtime metadata contains an invalid artifact join".to_owned());
            }
            let size =
                u64::try_from(bytes.len()).map_err(|_| "artifact size overflow".to_owned())?;
            artifacts.commit_named(digest.clone(), bytes);
            artifact_meta.insert(digest, Self::entry(size, self.state_policy.artifacts.ttl));
            artifact_bytes = artifact_bytes.saturating_add(size);
        }
        let mut admission_bytes = 0_u64;
        for (digest, materials) in &metadata.artifact_admissions {
            if artifacts.get(digest).is_none() || materials.admission.artifact_digest != *digest {
                return Err("runtime metadata contains an invalid admission join".to_owned());
            }
            admission_bytes = admission_bytes
                .checked_add(
                    Self::materials_size(&materials.clone().into())
                        .ok_or("admission size overflow")?,
                )
                .ok_or("admission size overflow")?;
        }

        let mut instances = BTreeMap::new();
        let mut invocation_index = BTreeMap::new();
        let mut instance_bytes = 0_u64;
        for (instance_id, durable) in metadata.instances {
            if instance_id.trim().is_empty()
                || artifacts.get(&durable.artifact_digest).is_none()
                || durable.owner_claim.validate().is_err()
            {
                return Err("runtime metadata contains an invalid instance join".to_owned());
            }
            if durable.materials.as_ref().is_some_and(|materials| {
                materials.admission.artifact_digest != durable.artifact_digest
            }) {
                return Err(
                    "runtime metadata contains an invalid instance admission join".to_owned(),
                );
            }
            let mut invocation_history = durable.invocation_history;
            for (request_id, invocation) in &invocation_history {
                let restored_input = if invocation.input_present {
                    Some(invocation.input.clone().unwrap_or(Value::Null))
                } else {
                    invocation.input.clone()
                };
                let input_matches = restored_input.as_ref().is_none_or(|input| {
                    serde_json::to_string(input).ok().as_deref()
                        == Some(invocation.input_fingerprint.as_str())
                });
                if request_id.trim().is_empty()
                    || invocation.request_id != *request_id
                    || invocation.program_invocation_id.trim().is_empty()
                    || !input_matches
                    || (invocation.result.is_none() && restored_input.is_none())
                    || !invocation
                        .program_invocation_id
                        .starts_with(&format!("{instance_id}:"))
                {
                    return Err(
                        "runtime metadata contains an invalid invocation history".to_owned()
                    );
                }
                if invocation_index
                    .insert(
                        invocation.program_invocation_id.clone(),
                        instance_id.clone(),
                    )
                    .is_some()
                {
                    return Err("runtime metadata contains a duplicate invocation".to_owned());
                }
            }
            let invocation = durable
                .invocation
                .map(|invocation| {
                    if invocation.program_invocation_id.trim().is_empty()
                        || invocation.request_id.trim().is_empty()
                        || !invocation
                            .program_invocation_id
                            .starts_with(&format!("{instance_id}:"))
                    {
                        return Err("runtime metadata contains an invalid invocation".to_owned());
                    }
                    let state = InvocationState {
                        request_id: invocation.request_id,
                        input_fingerprint: invocation.input_fingerprint,
                        input: if invocation.input_present {
                            Some(invocation.input.unwrap_or(Value::Null))
                        } else {
                            invocation.input
                        },
                        program_invocation_id: invocation.program_invocation_id,
                        phase: invocation.phase,
                        result: invocation.result,
                    };
                    let durable_state = DurableInvocationState {
                        request_id: state.request_id.clone(),
                        input_fingerprint: state.input_fingerprint.clone(),
                        input: state.input.clone(),
                        input_present: state.input.is_some(),
                        program_invocation_id: state.program_invocation_id.clone(),
                        phase: state.phase,
                        result: state.result.clone(),
                    };
                    if let Some(existing) = invocation_history.get(&state.request_id) {
                        if existing.program_invocation_id != state.program_invocation_id
                            || existing.input_fingerprint != state.input_fingerprint
                        {
                            return Err(
                                "runtime metadata contains a conflicting invocation request"
                                    .to_owned(),
                            );
                        }
                    } else {
                        invocation_history.insert(state.request_id.clone(), durable_state);
                    }
                    Ok(state)
                })
                .transpose()?;
            let invocation_history = invocation_history
                .into_iter()
                .map(|(request_id, invocation)| {
                    (
                        request_id,
                        InvocationState {
                            request_id: invocation.request_id,
                            input_fingerprint: invocation.input_fingerprint,
                            input: if invocation.input_present {
                                Some(invocation.input.unwrap_or(Value::Null))
                            } else {
                                invocation.input
                            },
                            program_invocation_id: invocation.program_invocation_id,
                            phase: invocation.phase,
                            result: invocation.result,
                        },
                    )
                })
                .collect();
            let state_entry =
                Self::restore_entry(durable.state_entry, self.state_policy.instances.ttl);
            instance_bytes = instance_bytes.saturating_add(state_entry.bytes);
            instances.insert(
                instance_id,
                InstanceState {
                    artifact_digest: durable.artifact_digest,
                    materials: durable.materials.map(Into::into),
                    owner_claim: durable.owner_claim,
                    invocation,
                    invocation_history,
                    invocation_bytes: durable.invocation_bytes,
                    state_entry,
                },
            );
        }

        let mut reservations = BTreeMap::new();
        let mut reservation_bytes = 0_u64;
        let mut maximum_generation = 0_u64;
        for durable in metadata.reservations {
            let event_ref = CanonicalEventRef {
                event_id: durable.event_id.clone(),
                generation: durable.generation,
            };
            event_ref
                .validate()
                .map_err(|_| "runtime metadata contains an invalid event ref")?;
            maximum_generation = maximum_generation.max(durable.generation);
            durable
                .owner_claim
                .validate()
                .map_err(|_| "runtime metadata contains an invalid event claim")?;
            if durable.request_id.trim().is_empty()
                || durable.program_instance_id.trim().is_empty()
                || durable.payload_schema.canonical_digest().ok().as_deref()
                    != Some(durable.schema_digest.as_str())
                || (durable.resume_started
                    && (durable.binding.is_none() || durable.status == EventStatus::Pending))
            {
                return Err("runtime metadata contains an invalid Event contract or wake".into());
            }
            if let Some(instance) = instances.get(&durable.program_instance_id) {
                let artifact = artifacts
                    .get(&instance.artifact_digest)
                    .and_then(|bytes| ExecutableArtifact::decode(bytes).ok())
                    .ok_or("Event instance artifact is unavailable")?;
                if !artifact.air.event_requirements.iter().any(|requirement| {
                    requirement.type_id == durable.type_id
                        && requirement.payload_schema == durable.payload_schema
                        && requirement.schema_digest == durable.schema_digest
                }) {
                    return Err("Event reservation differs from admitted artifact".into());
                }
                if let Some(binding) = &durable.binding
                    && (binding.event_ref != event_ref
                        || binding.program_instance_id != durable.program_instance_id
                        || binding.node_execution_id.trim().is_empty()
                        || !instance.invocation_history.values().any(|invocation| {
                            invocation.program_invocation_id == binding.program_invocation_id
                        })
                        || !artifact.air.event_requirements.iter().any(|requirement| {
                            requirement.node_id == binding.node_id
                                && requirement.type_id == durable.type_id
                        }))
                {
                    return Err("Event reservation has an invalid exact wait binding".into());
                }
            }
            if durable.type_id.trim().is_empty()
                || reservations
                    .insert(
                        (durable.event_id.clone(), durable.generation),
                        ReservationState {
                            request_id: durable.request_id,
                            program_instance_id: durable.program_instance_id,
                            owner_claim: durable.owner_claim,
                            type_id: durable.type_id,
                            payload_schema: durable.payload_schema,
                            schema_digest: durable.schema_digest,
                            binding: durable.binding,
                            resume_started: durable.resume_started,
                            status: durable.status,
                            occurrence_id: durable.occurrence_id,
                            state_entry: Self::restore_entry(
                                durable.state_entry,
                                self.state_policy.reservations.ttl,
                            ),
                        },
                    )
                    .is_some()
            {
                return Err("runtime metadata contains a duplicate event reservation".to_owned());
            }
            reservation_bytes = reservation_bytes.saturating_add(
                reservations
                    .get(&(durable.event_id, durable.generation))
                    .expect("inserted reservation")
                    .state_entry
                    .bytes,
            );
        }
        let mut applications = BTreeMap::new();
        let mut application_bytes = 0_u64;
        for (key, durable) in metadata.applications {
            durable
                .event_ref
                .validate()
                .map_err(|_| "runtime metadata contains an invalid application event ref")?;
            let reservation = reservations
                .get(&(
                    durable.event_ref.event_id.clone(),
                    durable.event_ref.generation,
                ))
                .ok_or("runtime metadata contains an orphan event application")?;
            if reservation.status != EventStatus::Fulfilled {
                return Err("runtime metadata contains an unfulfilled event application".to_owned());
            }
            if durable.occurrence_id.trim().is_empty()
                || reservation.occurrence_id.as_deref() != Some(durable.occurrence_id.as_str())
                || durable.source_kind.trim().is_empty()
                || durable.mapping_digest.trim().is_empty()
                || durable.source_record.trim().is_empty()
                || reservation
                    .payload_schema
                    .validate_value(&durable.payload)
                    .is_err()
                || applications
                    .values()
                    .any(|prior: &ApplicationState| prior.event_ref == durable.event_ref)
            {
                return Err(
                    "runtime metadata contains an invalid event application join".to_owned(),
                );
            }
            let application_entry =
                Self::restore_entry(durable.state_entry, self.state_policy.applications.ttl);
            application_bytes = application_bytes.saturating_add(application_entry.bytes);
            if key.trim().is_empty()
                || applications
                    .insert(
                        key,
                        ApplicationState {
                            event_ref: durable.event_ref,
                            occurrence_id: durable.occurrence_id,
                            source_kind: durable.source_kind,
                            mapping_digest: durable.mapping_digest,
                            source_record: durable.source_record,
                            payload: durable.payload,
                            state_entry: application_entry,
                        },
                    )
                    .is_some()
            {
                return Err("runtime metadata contains a duplicate event application".to_owned());
            }
        }
        let mut host_capability_settlements = BTreeMap::new();
        let mut host_capability_settlement_bytes = 0_u64;
        for (capability_request_id, durable) in metadata.host_capability_settlements {
            if capability_request_id != durable.settlement.capability_request_id
                || !durable.settlement.is_well_formed()
                || durable.owner_claim.validate().is_err()
                || durable.program_instance_id.trim().is_empty()
                || instances
                    .get(&durable.program_instance_id)
                    .is_some_and(|instance| instance.owner_claim != durable.owner_claim)
            {
                return Err(
                    "runtime metadata contains an invalid host capability settlement".to_owned(),
                );
            }
            let state_entry =
                Self::restore_entry(durable.state_entry, self.state_policy.applications.ttl);
            host_capability_settlement_bytes =
                host_capability_settlement_bytes.saturating_add(state_entry.bytes);
            if host_capability_settlements
                .insert(
                    capability_request_id,
                    HostCapabilitySettlementState {
                        program_instance_id: durable.program_instance_id,
                        owner_claim: durable.owner_claim,
                        settlement: durable.settlement,
                        resume_started: durable.resume_started,
                        state_entry,
                    },
                )
                .is_some()
            {
                return Err(
                    "runtime metadata contains a duplicate host capability settlement".to_owned(),
                );
            }
        }
        let cancellation_bytes = metadata
            .cancelled
            .iter()
            .map(|id| u64::try_from(id.len()).unwrap_or(u64::MAX))
            .fold(0_u64, u64::saturating_add);
        for invocation_id in &metadata.cancelled {
            if !invocation_index.contains_key(invocation_id) {
                return Err("runtime metadata contains an orphan cancellation".to_owned());
            }
        }
        if artifact_meta.len() > self.state_policy.artifacts.max_entries
            || artifact_bytes > self.state_policy.artifacts.max_bytes
            || metadata.artifact_admissions.len() > self.state_policy.admissions.max_entries
            || admission_bytes > self.state_policy.admissions.max_bytes
            || instances.len() > self.state_policy.instances.max_entries
            || instance_bytes > self.state_policy.instances.max_bytes
            || reservations.len() > self.state_policy.reservations.max_entries
            || reservation_bytes > self.state_policy.reservations.max_bytes
            || applications
                .len()
                .checked_add(host_capability_settlements.len())
                .is_none_or(|entries| entries > self.state_policy.applications.max_entries)
            || application_bytes
                .checked_add(host_capability_settlement_bytes)
                .is_none_or(|bytes| bytes > self.state_policy.applications.max_bytes)
            || metadata.cancelled.len() > self.state_policy.cancellations.max_entries
            || cancellation_bytes > self.state_policy.cancellations.max_bytes
        {
            return Err("runtime metadata exceeds the configured state policy".to_owned());
        }
        if metadata.next_generation < maximum_generation {
            return Err("runtime metadata generation watermark is behind a reservation".to_owned());
        }

        self.artifacts = artifacts;
        self.artifact_meta = artifact_meta;
        self.artifact_bytes = artifact_bytes;
        self.artifact_admissions = metadata
            .artifact_admissions
            .into_iter()
            .map(|(digest, materials)| (digest, materials.into()))
            .collect();
        self.admission_meta = self
            .artifact_admissions
            .iter()
            .map(|(digest, materials)| {
                (
                    digest.clone(),
                    Self::entry(
                        Self::materials_size(materials).unwrap_or(0),
                        self.state_policy.admissions.ttl,
                    ),
                )
            })
            .collect();
        self.admission_bytes = admission_bytes;
        self.instances = instances;
        self.invocation_index = invocation_index;
        self.instance_bytes = instance_bytes;
        self.reservations = reservations;
        self.reservation_bytes = reservation_bytes;
        self.applications = applications;
        self.application_bytes = application_bytes;
        self.host_capability_settlements = host_capability_settlements;
        self.host_capability_settlement_bytes = host_capability_settlement_bytes;
        self.cancelled = metadata
            .cancelled
            .into_iter()
            .map(|id| {
                let bytes = u64::try_from(id.len()).unwrap_or(u64::MAX);
                (id, Self::entry(bytes, self.state_policy.cancellations.ttl))
            })
            .collect();
        self.cancellation_bytes = cancellation_bytes;
        self.next_generation = metadata.next_generation;
        self.refresh_event_index()
    }

    fn entry(bytes: u64, ttl: Duration) -> StateEntry {
        StateEntry {
            bytes,
            expires_at: Self::now().checked_add(ttl).unwrap_or_else(Self::now),
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

    fn host_capability_settlement_size(
        program_instance_id: &str,
        owner_claim: &RuntimeOwnerClaim,
        settlement: &HostCapabilitySettlement,
    ) -> Option<u64> {
        serde_json::to_vec(&(program_instance_id, owner_claim, settlement))
            .ok()
            .and_then(|bytes| u64::try_from(bytes.len()).ok())
    }

    /// Remove only entries whose typed TTL has elapsed. No live owner claim,
    /// invocation idempotency record, event application, or settlement is
    /// evicted.
    pub fn cleanup_expired(&mut self) -> Result<(), String> {
        self.expire_pending_events()?;
        let now = Self::now();

        let expired_instances = self
            .instances
            .iter()
            .filter_map(|(id, instance)| {
                // A claimed invocation may execute for longer than its
                // instance TTL. Keep that owner claim and invocation index
                // alive until finalization.
                let running = instance
                    .invocation
                    .as_ref()
                    .is_some_and(|invocation| invocation.result.is_none());
                (instance.state_entry.expired(now) && !running).then_some(id.clone())
            })
            .collect::<Vec<_>>();
        for id in expired_instances {
            if let Some(instance) = self.instances.remove(&id) {
                // Every invocation this instance ever indexed leaves with it,
                // together with its cancellation marker. A marker outlives
                // the instance by construction (its expiry is raised to the
                // instance expiry), so leaving it behind would persist a
                // cancellation for an invocation no instance explains, and
                // the next reopen would refuse the whole state as orphaned.
                let invocation_ids = instance
                    .invocation
                    .iter()
                    .chain(instance.invocation_history.values())
                    .map(|invocation| invocation.program_invocation_id.clone())
                    .collect::<BTreeSet<_>>();
                for invocation_id in invocation_ids {
                    self.invocation_index.remove(&invocation_id);
                    if let Some(marker) = self.cancelled.remove(&invocation_id) {
                        self.cancellation_bytes =
                            self.cancellation_bytes.saturating_sub(marker.bytes);
                    }
                }
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

        // Build the active-artifact index once. Checking every instance for
        // every expired artifact turns cleanup into an avoidable O(artifacts ×
        // instances) scan under load.
        let active_artifacts = self
            .instances
            .values()
            .map(|instance| instance.artifact_digest.as_str())
            .collect::<BTreeSet<_>>();
        let expired_artifacts = self
            .artifact_meta
            .iter()
            .filter_map(|(digest, entry)| {
                if entry.expired(now) && !active_artifacts.contains(digest.as_str()) {
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

        let live_instances = self.instances.keys().cloned().collect::<BTreeSet<_>>();
        self.reservations.retain(|_, reservation| {
            if reservation.state_entry.expired(now)
                && live_instances.contains(&reservation.program_instance_id)
            {
                if reservation.status == EventStatus::Pending {
                    reservation.status = EventStatus::Expired;
                }
                return true;
            }
            if reservation.state_entry.expired(now) {
                self.reservation_bytes = self
                    .reservation_bytes
                    .saturating_sub(reservation.state_entry.bytes);
                false
            } else {
                true
            }
        });

        let mut expired_application_bytes = 0_u64;
        self.applications.retain(|_, application| {
            if application.state_entry.expired(now)
                && !self.reservations.contains_key(&(
                    application.event_ref.event_id.clone(),
                    application.event_ref.generation,
                ))
            {
                expired_application_bytes =
                    expired_application_bytes.saturating_add(application.state_entry.bytes);
                false
            } else {
                true
            }
        });
        self.application_bytes = self
            .application_bytes
            .saturating_sub(expired_application_bytes);

        let live_instances = self.instances.keys().cloned().collect::<BTreeSet<_>>();
        let mut expired_settlement_bytes = 0_u64;
        self.host_capability_settlements.retain(|_, settlement| {
            if settlement.state_entry.expired(now)
                && !live_instances.contains(&settlement.program_instance_id)
            {
                expired_settlement_bytes =
                    expired_settlement_bytes.saturating_add(settlement.state_entry.bytes);
                false
            } else {
                true
            }
        });
        self.host_capability_settlement_bytes = self
            .host_capability_settlement_bytes
            .saturating_sub(expired_settlement_bytes);

        self.cancelled.retain(|_, entry| {
            if entry.expired(now) {
                self.cancellation_bytes = self.cancellation_bytes.saturating_sub(entry.bytes);
                false
            } else {
                true
            }
        });
        // Expiry is a durable lifecycle transition too. Persist the cleaned
        // projection before another operation can observe it.
        self.persist_runtime_state()
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

    /// Commit canonical executable-artifact bytes. Raw AIR is rejected.
    pub fn admit_artifact(&mut self, bytes: Vec<u8>) -> String {
        self.try_admit_artifact(bytes).unwrap_or_default()
    }

    /// Commit an artifact, failing closed when the bounded artifact policy is
    /// exhausted. The legacy string-returning method above maps refusal to an
    /// empty digest so callers cannot accidentally execute an uncommitted
    /// artifact.
    pub fn try_admit_artifact(&mut self, bytes: Vec<u8>) -> Result<String, String> {
        self.cleanup_expired()?;
        let digest =
            canonical_artifact_digest(&bytes).map_err(|_| "invalid_artifact".to_owned())?;
        self.try_admit_named_artifact(digest.clone(), bytes)?;
        Ok(digest)
    }

    fn try_admit_named_artifact(&mut self, digest: String, bytes: Vec<u8>) -> Result<(), String> {
        let size = u64::try_from(bytes.len()).map_err(|_| "artifact_too_large".to_owned())?;
        if size > MAX_ARTIFACT_BYTES || size > self.state_policy.artifacts.max_bytes {
            return Err("artifact_too_large".to_owned());
        }
        if ExecutableArtifact::decode_for_execution(&bytes, &digest).is_err() {
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
        self.artifacts.commit_named(digest.clone(), bytes);
        self.artifact_meta.insert(
            digest.clone(),
            Self::entry(size, self.state_policy.artifacts.ttl),
        );
        self.artifact_bytes = self.artifact_bytes.saturating_add(size);
        self.persist_runtime_state()
            .map_err(|_| "runtime_state_unavailable".to_owned())?;
        Ok(())
    }

    /// Bind invocation admission materials to an instance created from a digest.
    pub fn bind_admission(
        &mut self,
        program_instance_id: &str,
        materials: InvocationMaterials,
    ) -> Result<(), String> {
        self.cleanup_expired()?;
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
        let artifact_bytes = self
            .artifacts
            .get(&artifact_digest)
            .ok_or_else(|| "unknown_artifact".to_owned())?;
        self.verify_selected_materials(artifact_bytes, &materials)?;
        if let Some(existing) = self
            .instances
            .get(program_instance_id)
            .and_then(|instance| instance.materials.as_ref())
        {
            return if Self::same_binding(existing, &materials) {
                Ok(())
            } else {
                Err("admission_conflict".to_owned())
            };
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
        self.persist_runtime_state()
            .map_err(|_| "runtime_state_unavailable".to_owned())?;
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
        self.cleanup_expired()?;
        if self.artifacts.get(artifact_digest).is_none() {
            return Err("unknown_artifact".to_owned());
        }
        if materials.admission.artifact_digest != artifact_digest {
            return Err("artifact_digest_mismatch".to_owned());
        }
        let artifact_bytes = self
            .artifacts
            .get(artifact_digest)
            .ok_or_else(|| "unknown_artifact".to_owned())?;
        self.verify_selected_materials(artifact_bytes, &materials)?;
        if let Some(existing) = self.artifact_admissions.get(artifact_digest) {
            return if Self::same_binding(existing, &materials) {
                Ok(())
            } else {
                Err("admission_conflict".to_owned())
            };
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
        if !self.artifact_admissions.contains_key(artifact_digest)
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
            Self::entry(size, self.state_policy.admissions.ttl),
        );
        self.admission_bytes = next_bytes;
        self.persist_runtime_state()
            .map_err(|_| "runtime_state_unavailable".to_owned())?;
        Ok(())
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

    /// State the posture for an authored `Ask` on a builtin capability
    /// explicitly, as `APXM_APPROVAL_POLICY` does for a deployed service.
    pub fn bind_approval_policy(&mut self, policy: ApprovalPolicy) {
        self.approval_policy = policy;
    }

    /// The posture an authored `Ask` on a builtin capability is resolved under.
    #[must_use]
    pub const fn approval_policy(&self) -> ApprovalPolicy {
        self.approval_policy
    }

    /// Handle the negotiated APXM execution read/observation surface.
    pub fn handle_v2(
        &self,
        handshake: &RuntimeHandshakeV2,
        request: RuntimeRequestV2,
    ) -> Result<RuntimeResultV2, ProtocolError> {
        let negotiated = handshake.negotiate(&RuntimeHandshakeV2::server())?;
        let request_id = request.request_id();
        let feature = request.feature();
        let node_requested = matches!(
            &request,
            RuntimeRequestV2::ProgramInvocationInspect {
                node_execution_id: Some(_),
                ..
            }
        );
        let service_terminal_inspection = match &request {
            RuntimeRequestV2::ProgramInvocationInspect {
                program_invocation_id,
                node_execution_id: None,
                ..
            } => self
                .service_invocation_inspection(program_invocation_id.as_str())
                .filter(|inspection| {
                    matches!(
                        inspection.status,
                        apxm_runtime_protocol::ProgramInvocationStatus::OutcomeUnknown
                            | apxm_runtime_protocol::ProgramInvocationStatus::CancellationUnconfirmed
                            | apxm_runtime_protocol::ProgramInvocationStatus::Cancelling
                            | apxm_runtime_protocol::ProgramInvocationStatus::Cancelled
                    ) || self.profile_mismatch_is_terminal(program_invocation_id.as_str())
                }),
            _ => None,
        };
        if !negotiated.contains(&feature) {
            return Ok(RuntimeResultV2::Failed {
                request_id,
                code: RuntimeFailureCode::UnsupportedFeature,
            });
        }
        request
            .validate()
            .map_err(|_| ProtocolError::InvalidRequest)?;
        let execution_read_request = request.as_execution_read_request();
        let live_observations = if matches!(
            &execution_read_request,
            apxm_runtime_protocol::ExecutionReadRequest::ObservationSubscribe { .. }
        ) {
            self.readable_live_observations(self.observation_sink.snapshot())
        } else {
            Vec::new()
        };
        let result = self
            .execution_backend
            .read_execution_with_live(execution_read_request, &live_observations);
        match result {
            Ok(apxm_runtime_protocol::ExecutionReadResult::ObservationPage { page }) => {
                Ok(RuntimeResultV2::ObservationPage { request_id, page })
            }
            Ok(apxm_runtime_protocol::ExecutionReadResult::ProgramInvocationInspection {
                inspection,
            }) if !node_requested => {
                let service_inspection = service_terminal_inspection.filter(|service| {
                    service.status != apxm_runtime_protocol::ProgramInvocationStatus::Cancelling
                        || !matches!(
                            inspection.status,
                            apxm_runtime_protocol::ProgramInvocationStatus::CommittedReturn
                                | apxm_runtime_protocol::ProgramInvocationStatus::CommittedYield
                                | apxm_runtime_protocol::ProgramInvocationStatus::Cancelled
                                | apxm_runtime_protocol::ProgramInvocationStatus::Failed
                        )
                });
                Ok(RuntimeResultV2::ProgramInvocationInspection {
                    request_id,
                    inspection: service_inspection.unwrap_or(inspection),
                })
            }
            Ok(apxm_runtime_protocol::ExecutionReadResult::ProgramInvocationInspection {
                ..
            }) => Ok(RuntimeResultV2::Failed {
                request_id,
                code: RuntimeFailureCode::InvalidRequest,
            }),
            Ok(apxm_runtime_protocol::ExecutionReadResult::NodeExecutionInspection {
                inspection,
            }) if node_requested => {
                if request
                    .as_execution_read_request()
                    .validate_node_inspection(&inspection)
                    .is_err()
                {
                    return Ok(RuntimeResultV2::Failed {
                        request_id,
                        code: RuntimeFailureCode::InvalidRequest,
                    });
                }
                Ok(RuntimeResultV2::NodeExecutionInspection {
                    request_id,
                    inspection,
                })
            }
            Ok(apxm_runtime_protocol::ExecutionReadResult::NodeExecutionInspection { .. }) => {
                Ok(RuntimeResultV2::Failed {
                    request_id,
                    code: RuntimeFailureCode::InvalidRequest,
                })
            }
            Ok(apxm_runtime_protocol::ExecutionReadResult::Content { content }) => {
                Ok(RuntimeResultV2::Content {
                    request_id,
                    content,
                })
            }
            Ok(apxm_runtime_protocol::ExecutionReadResult::Output { output }) => {
                Ok(RuntimeResultV2::Output { request_id, output })
            }
            Ok(apxm_runtime_protocol::ExecutionReadResult::EvidencePage { page }) => {
                Ok(RuntimeResultV2::EvidencePage { request_id, page })
            }
            Err(CommitLocalError::InvalidRead(_))
                if !node_requested
                    && matches!(&request, RuntimeRequestV2::ProgramInvocationInspect { .. }) =>
            {
                let RuntimeRequestV2::ProgramInvocationInspect {
                    program_invocation_id,
                    ..
                } = &request
                else {
                    unreachable!()
                };
                match self.service_invocation_inspection(program_invocation_id.as_str()) {
                    Some(inspection) => Ok(RuntimeResultV2::ProgramInvocationInspection {
                        request_id,
                        inspection,
                    }),
                    None => Ok(RuntimeResultV2::Failed {
                        request_id,
                        code: RuntimeFailureCode::NotFound,
                    }),
                }
            }
            Err(error) => Ok(RuntimeResultV2::Failed {
                request_id,
                code: runtime_failure_code(&error),
            }),
        }
    }

    fn service_invocation_inspection(
        &self,
        invocation_id: &str,
    ) -> Option<apxm_runtime_protocol::ProgramInvocationInspection> {
        let instance_id = self.invocation_index.get(invocation_id)?;
        let invocation = self
            .instances
            .get(instance_id)?
            .invocation_history
            .values()
            .find(|value| value.program_invocation_id == invocation_id)?;
        let status = match &invocation.result {
            None if invocation.phase == InvocationExecutionPhase::Pending => {
                apxm_runtime_protocol::ProgramInvocationStatus::AdmissionPending
            }
            None => apxm_runtime_protocol::ProgramInvocationStatus::Running,
            Some(RuntimeResult::ProgramInvocationStarted { .. }) => self
                .execution_backend
                .invocation_status(invocation_id)
                .unwrap_or(apxm_runtime_protocol::ProgramInvocationStatus::Running),
            Some(RuntimeResult::Cancelled { .. }) => {
                apxm_runtime_protocol::ProgramInvocationStatus::Cancelled
            }
            Some(RuntimeResult::Failed { code, .. }) if code == "outcome_unknown" => {
                apxm_runtime_protocol::ProgramInvocationStatus::OutcomeUnknown
            }
            Some(RuntimeResult::Failed { .. }) => {
                apxm_runtime_protocol::ProgramInvocationStatus::Failed
            }
            Some(_) => apxm_runtime_protocol::ProgramInvocationStatus::Failed,
        };
        let status = if self.cancelled.contains_key(invocation_id) {
            match status {
                apxm_runtime_protocol::ProgramInvocationStatus::AdmissionPending
                | apxm_runtime_protocol::ProgramInvocationStatus::Running => {
                    apxm_runtime_protocol::ProgramInvocationStatus::Cancelling
                }
                apxm_runtime_protocol::ProgramInvocationStatus::OutcomeUnknown => {
                    apxm_runtime_protocol::ProgramInvocationStatus::CancellationUnconfirmed
                }
                other => other,
            }
        } else {
            status
        };
        Some(apxm_runtime_protocol::ProgramInvocationInspection {
            program_invocation_id: apxm_runtime_protocol::ProgramInvocationId::new(invocation_id)
                .ok()?,
            status,
            node_execution_refs: Vec::new(),
            output_refs: Vec::new(),
            evidence_refs: Vec::new(),
            cursor: apxm_runtime_protocol::ExecutionCursor::new(
                0,
                format!("service-state.{invocation_id}"),
            )
            .ok()?,
        })
    }

    fn profile_mismatch_is_terminal(&self, invocation_id: &str) -> bool {
        self.invocation_index
            .get(invocation_id)
            .and_then(|instance_id| self.instances.get(instance_id))
            .and_then(|instance| instance.invocation.as_ref())
            .filter(|invocation| invocation.program_invocation_id == invocation_id)
            .and_then(|invocation| invocation.result.as_ref())
            .is_some_and(|result| matches!(result, RuntimeResult::Failed { code, .. } if code == "admission_profile_mismatch"))
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
        handshake.admit()?;
        let request_id = match &request {
            RuntimeRequest::ProgramInstanceCreate { request_id, .. }
            | RuntimeRequest::ProgramInvocationStart { request_id, .. }
            | RuntimeRequest::EventReserve { request_id, .. }
            | RuntimeRequest::EventFulfill { request_id, .. }
            | RuntimeRequest::EventList { request_id, .. }
            | RuntimeRequest::EventInspect { request_id, .. }
            | RuntimeRequest::EventExpire { request_id, .. }
            | RuntimeRequest::EventCancel { request_id, .. }
            | RuntimeRequest::CapabilityFulfill { request_id, .. }
            | RuntimeRequest::CapabilityCancel { request_id, .. }
            | RuntimeRequest::ProgramInvocationCancel { request_id, .. } => request_id,
        };
        if self.cleanup_expired().is_err() {
            return Ok(RuntimeResult::Failed {
                request_id: request_id.to_owned(),
                code: "runtime_state_unavailable".into(),
            });
        }
        if request_id.trim().is_empty() {
            return Err(ProtocolError::InvalidRequest);
        }
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
                program_instance_id,
                owner_claim,
            } => self.reserve_event(request_id, program_instance_id, owner_claim, type_id),
            RuntimeRequest::EventFulfill {
                request_id,
                owner_claim,
                application,
            } => self.fulfill_event(request_id, owner_claim, application),
            RuntimeRequest::EventList {
                request_id,
                owner_claim,
            } => self.list_events(request_id, owner_claim),
            RuntimeRequest::EventInspect {
                request_id,
                owner_claim,
                event_ref,
            } => self.inspect_event(request_id, owner_claim, event_ref),
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
            RuntimeRequest::CapabilityFulfill {
                request_id,
                owner_claim,
                capability_request_id,
                outcome,
                output,
                receipt_ref,
                message,
            } => {
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
                Ok(self.settle_host_capability(
                    request_id,
                    owner_claim.clone(),
                    capability_request_id,
                    outcome,
                    output,
                    receipt_ref,
                    message,
                    true,
                ))
            }
            RuntimeRequest::CapabilityCancel {
                request_id,
                owner_claim,
                capability_request_id,
                message,
            } => {
                if !is_host_capability_request_id(&capability_request_id) {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "invalid_request".to_owned(),
                    });
                }
                Ok(self.settle_host_capability(
                    request_id,
                    owner_claim,
                    capability_request_id,
                    HostCapabilityOutcomeKind::Cancelled,
                    None,
                    None,
                    message,
                    true,
                ))
            }
            RuntimeRequest::ProgramInvocationCancel {
                request_id,
                owner_claim,
                program_invocation_id,
            } => Ok(self.cancel_invocation(request_id, owner_claim, program_invocation_id)),
        }
    }

    /// Handle the separately negotiated execution-admission mutation
    /// envelope. It is deliberately not accepted by the frozen Protocol/1
    /// request union or by the read-only Runtime/2 handshake.
    pub fn handle_execution_admission(
        &mut self,
        handshake: &RuntimeExecutionAdmissionHandshake,
        request: RuntimeExecutionAdmissionRequest,
    ) -> RuntimeResult {
        if self.cleanup_expired().is_err() {
            return RuntimeResult::Failed {
                request_id: execution_admission_request_id(&request),
                code: "runtime_state_unavailable".into(),
            };
        }
        if let Err(error) = handshake.admit() {
            return RuntimeResult::Failed {
                request_id: execution_admission_request_id(&request),
                code: format!("{error:?}"),
            };
        }
        match request {
            RuntimeExecutionAdmissionRequest::ProgramInstanceBindAdmission {
                request_id,
                program_instance_id,
                owner_claim,
                admission_profile_ref,
            } => self.bind_invocation_admission(
                request_id,
                program_instance_id,
                owner_claim,
                admission_profile_ref,
            ),
        }
    }

    fn bind_invocation_admission(
        &mut self,
        request_id: String,
        program_instance_id: String,
        owner_claim: RuntimeOwnerClaim,
        admission_profile_ref: String,
    ) -> RuntimeResult {
        if request_id.trim().is_empty() {
            return RuntimeResult::Failed {
                request_id,
                code: "invalid_request".to_owned(),
            };
        }
        if owner_claim.validate().is_err() {
            return RuntimeResult::Failed {
                request_id,
                code: "invalid_owner_claim".to_owned(),
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
        let Some(profile) = self.admission_profile.as_ref() else {
            return RuntimeResult::Failed {
                request_id,
                code: "missing_invocation_admission_profile".to_owned(),
            };
        };
        if profile.profile_ref() != admission_profile_ref {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_invocation_admission_profile".to_owned(),
            };
        }
        let Some(artifact_bytes) = self.artifacts.get(&instance.artifact_digest) else {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_artifact".to_owned(),
            };
        };
        let local_materials = profile.materials_for_artifact(artifact_bytes);
        if self
            .verify_selected_materials(artifact_bytes, &local_materials)
            .is_err()
        {
            return RuntimeResult::Failed {
                request_id,
                code: "invalid_invocation_admission".to_owned(),
            };
        }
        if let Some(existing) = instance.materials.as_ref()
            && existing != &local_materials
        {
            return RuntimeResult::Failed {
                request_id,
                code: "admission_conflict".to_owned(),
            };
        }
        match self.bind_admission(&program_instance_id, local_materials) {
            Ok(()) => {
                let artifact_digest = self
                    .instances
                    .get(&program_instance_id)
                    .map(|instance| instance.artifact_digest.clone())
                    .unwrap_or_default();
                RuntimeResult::ProgramInstanceAdmissionBound {
                    request_id,
                    program_instance_id,
                    artifact_digest,
                }
            }
            Err(code) => RuntimeResult::Failed { request_id, code },
        }
    }
}

impl RuntimeService {
    fn create_instance(
        &mut self,
        request_id: String,
        artifact_digest: String,
    ) -> Result<RuntimeResult, ProtocolError> {
        if !apxm_core::grammar::is_digest(&artifact_digest) {
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
                invocation_history: BTreeMap::new(),
                invocation_bytes: 0,
                state_entry: Self::entry(material_bytes, self.state_policy.instances.ttl),
            },
        );
        self.instance_bytes = self.instance_bytes.saturating_add(material_bytes);
        if self.persist_runtime_state().is_err() {
            self.instances.remove(&id);
            self.instance_bytes = self.instance_bytes.saturating_sub(material_bytes);
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "runtime_state_unavailable".to_owned(),
            });
        }
        Ok(RuntimeResult::ProgramInstanceCreated {
            request_id,
            program_instance_id: id,
            owner_claim,
            artifact_digest,
            admission_profile: self.admission_profile.as_ref().map(|profile| {
                RuntimeAdmissionProfileDescriptor {
                    profile_ref: profile.profile_ref().to_owned(),
                    port_bindings_digest: port_bindings_digest_for(profile.capability_profile()),
                    resource_ceiling_digest: canonical_resource_ceiling_digest(),
                }
            }),
        })
    }

    fn start_invocation(
        &mut self,
        request_id: String,
        program_instance_id: String,
        owner_claim: RuntimeOwnerClaim,
        input: Value,
    ) -> RuntimeResult {
        let prepared =
            match self.prepare_invocation(request_id, program_instance_id, owner_claim, input) {
                Ok(prepared) => prepared,
                Err(result) => return result,
            };
        if !matches!(self.begin_invocation(&prepared.invocation_id), Ok(true)) {
            self.release_invocation_claim(&prepared.invocation_id);
            return RuntimeResult::Failed {
                request_id: prepared.request_id.clone(),
                code: "runtime_state_unavailable".to_owned(),
            };
        }
        let execution = prepared.execute();
        self.finish_invocation(&prepared, execution)
    }

    /// Atomically validate and claim an invocation, returning only owned
    /// execution inputs. The caller must run the returned driver without the
    /// service mutex and then call [`Self::finish_invocation`].
    #[allow(clippy::result_large_err)]
    pub(crate) fn prepare_invocation(
        &mut self,
        request_id: String,
        program_instance_id: String,
        owner_claim: RuntimeOwnerClaim,
        input: Value,
    ) -> Result<PreparedInvocation, RuntimeResult> {
        if request_id.trim().is_empty() {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "invalid_request".to_owned(),
            });
        }
        if u64::try_from(request_id.len()).unwrap_or(u64::MAX) > self.state_policy.max_input_bytes {
            return Err(RuntimeResult::Failed {
                request_id: "request_id_too_large".to_owned(),
                code: "request_id_too_large".to_owned(),
            });
        }
        if owner_claim.validate().is_err() {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "invalid_owner_claim".to_owned(),
            });
        }
        if self.disconnected {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "disconnected".to_owned(),
            });
        }
        let Some(instance) = self.instances.get(&program_instance_id) else {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "unknown_instance".to_owned(),
            });
        };
        if instance.owner_claim != owner_claim {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "owner_mismatch".to_owned(),
            });
        }
        let input_fingerprint = serde_json::to_string(&input).unwrap_or_default();
        let input_bytes = u64::try_from(input_fingerprint.len()).unwrap_or(u64::MAX);
        if input_bytes > self.state_policy.max_input_bytes {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "input_too_large".to_owned(),
            });
        }
        if let Some(prior) = instance.invocation_history.get(&request_id) {
            if prior.input_fingerprint != input_fingerprint {
                return Err(RuntimeResult::Failed {
                    request_id,
                    code: "invocation_idempotency_conflict".to_owned(),
                });
            }
            if let Some(result) = &prior.result
                && (matches!(
                    result,
                    RuntimeResult::Failed { .. } | RuntimeResult::Cancelled { .. }
                ) || matches!(result, RuntimeResult::ProgramInvocationStarted { .. })
                    && matches!(
                        self.execution_backend
                            .invocation_status(&prior.program_invocation_id),
                        Some(
                            apxm_runtime_protocol::ProgramInvocationStatus::CommittedReturn
                                | apxm_runtime_protocol::ProgramInvocationStatus::CommittedYield
                        )
                    ))
            {
                return Err(result.clone());
            }
            if self.cancelled.contains_key(&prior.program_invocation_id) {
                return Err(RuntimeResult::Cancelled { request_id });
            }
            return Err(prior.result.clone().unwrap_or_else(|| {
                RuntimeResult::ProgramInvocationStarted {
                    request_id,
                    program_invocation_id: prior.program_invocation_id.clone(),
                }
            }));
        }
        if let Some(prior) = instance.invocation.as_ref() {
            if prior.request_id == request_id {
                if let Some(result) = &prior.result
                    && (matches!(result, RuntimeResult::Failed { .. } | RuntimeResult::Cancelled { .. })
                        || matches!(result, RuntimeResult::ProgramInvocationStarted { .. })
                            && matches!(
                                self.execution_backend.invocation_status(&prior.program_invocation_id),
                                Some(
                                    apxm_runtime_protocol::ProgramInvocationStatus::CommittedReturn
                                        | apxm_runtime_protocol::ProgramInvocationStatus::CommittedYield
                                )
                            ))
                {
                    return Err(result.clone());
                }
                if self.cancelled.contains_key(&prior.program_invocation_id) {
                    return Err(RuntimeResult::Cancelled { request_id });
                }
                if prior.input_fingerprint == input_fingerprint {
                    return Err(prior.result.clone().unwrap_or_else(|| {
                        RuntimeResult::ProgramInvocationStarted {
                            request_id: request_id.clone(),
                            program_invocation_id: prior.program_invocation_id.clone(),
                        }
                    }));
                }
                return Err(RuntimeResult::Failed {
                    request_id,
                    code: "invocation_idempotency_conflict".to_owned(),
                });
            }
            let status = self
                .execution_backend
                .invocation_status(&prior.program_invocation_id);
            if status == Some(apxm_runtime_protocol::ProgramInvocationStatus::CommittedReturn) {
                return Err(RuntimeResult::Failed {
                    request_id,
                    code: "program_instance_completed".to_owned(),
                });
            }
            if prior.result.is_none()
                && (status != Some(apxm_runtime_protocol::ProgramInvocationStatus::CommittedYield)
                    || self
                        .active_cancellations
                        .contains_key(&prior.program_invocation_id))
            {
                return Err(RuntimeResult::Failed {
                    request_id,
                    code: "invocation_already_started".to_owned(),
                });
            }
        }
        let owned_work = self
            .instances
            .values()
            .filter_map(|instance| instance.invocation.as_ref())
            .filter(|invocation| {
                invocation.result.is_none()
                    && (invocation.phase == InvocationExecutionPhase::Pending
                        || self
                            .active_cancellations
                            .contains_key(&invocation.program_invocation_id))
            })
            .count();
        if owned_work >= MAX_ACTIVE_INVOCATIONS {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "invocation_capacity_exhausted".to_owned(),
            });
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
            .saturating_add(instance_with_input);
        if aggregate_instance_bytes > self.state_policy.instances.max_bytes {
            return Err(RuntimeResult::Failed {
                request_id,
                code: Self::quota_code("instance"),
            });
        }
        let invocation_id = format!("{program_instance_id}:inv-{}", Uuid::new_v4());
        let artifact_digest = instance.artifact_digest.clone();
        let Some(bytes) = self.artifacts.get(&artifact_digest) else {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "unknown_artifact".to_owned(),
            });
        };
        let bytes = bytes.to_vec();
        let Ok(artifact) = ExecutableArtifact::decode_for_execution(&bytes, &artifact_digest)
        else {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "invalid_artifact".to_owned(),
            });
        };
        let air = artifact.air.clone();
        if let Err(code) =
            validate_package_permission_resolution(&air, self.package_root.as_deref())
        {
            return Err(RuntimeResult::Failed { request_id, code });
        }
        // An instance parked at an owner request admits only a closed answer
        // envelope its committed request accepts. Refusing here, before an
        // invocation identity is minted, keeps a rejected answer from
        // becoming a failed invocation on the instance's history.
        if self.resumable_invocations
            && let Some(continuation) = self
                .execution_backend
                .load_continuation(&ProgramInstanceRef::new(program_instance_id.clone()))
                .and_then(|value| serde_json::from_value::<Continuation>(value.payload).ok())
            && apxm_execution::admit_owner_answer(&continuation, &input).is_err()
        {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "owner_answer_rejected".to_owned(),
            });
        }
        let Some(bound_materials) = instance.materials.as_ref() else {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "missing_invocation_admission".to_owned(),
            });
        };
        if self
            .verify_selected_materials(&bytes, bound_materials)
            .is_err()
        {
            return Err(RuntimeResult::Failed {
                request_id,
                code: "admission_profile_mismatch".to_owned(),
            });
        }
        let mut materials = InvocationMaterials {
            admission: bound_materials.admission.clone(),
            release_bytes: bound_materials.release_bytes.clone(),
            provenance_bytes: bound_materials.provenance_bytes.clone(),
        };
        // The service owns the invocation identity. Keep the externally
        // supplied admission fields intact except for this runtime-minted
        // identity, which is not caller-authoritative.
        materials.admission.invocation_id.clone_from(&invocation_id);
        let cancellation = CancellationToken::new();
        self.active_cancellations
            .insert(invocation_id.clone(), cancellation.clone());
        let prepared = PreparedInvocation {
            request_id: request_id.clone(),
            program_instance_id: program_instance_id.clone(),
            invocation_id: invocation_id.clone(),
            input: input.clone(),
            air,
            artifact_bytes: bytes,
            materials,
            handlers: self.handlers.clone(),
            package_root: self.package_root.clone(),
            sandbox_registry: self.sandbox_registry.clone(),
            execution_backend: self.execution_backend.commit_port(),
            observation_sink: self.observation_sink.clone(),
            broker: self.broker.clone(),
            approval_policy: self.approval_policy,
            cancellation,
            resumable: self.resumable_invocations,
            events: self.event_port(program_instance_id.clone()),
        };
        // Publish the invocation index and provisional state before releasing
        // the mutex. Cancellation therefore sees the exact owner claim.
        if let Some(instance) = self.instances.get_mut(&program_instance_id) {
            self.instance_bytes = self
                .instance_bytes
                .saturating_sub(instance.state_entry.bytes)
                .saturating_add(instance_with_input);
            instance.state_entry.bytes = instance_with_input;
            instance.invocation_bytes = input_bytes.saturating_add(request_bytes);
            let invocation = InvocationState {
                request_id: request_id.clone(),
                input_fingerprint: input_fingerprint.clone(),
                input: Some(input),
                program_invocation_id: invocation_id.clone(),
                phase: InvocationExecutionPhase::Pending,
                result: None,
            };
            instance
                .invocation_history
                .insert(request_id.clone(), invocation.clone());
            instance.invocation = Some(invocation);
        }
        self.invocation_index
            .insert(invocation_id, program_instance_id.clone());
        if self.persist_runtime_state().is_err() {
            self.invocation_index.remove(&prepared.invocation_id);
            self.active_cancellations.remove(&prepared.invocation_id);
            if let Some(instance) = self.instances.get_mut(&program_instance_id) {
                self.instance_bytes = self
                    .instance_bytes
                    .saturating_sub(instance.state_entry.bytes)
                    .saturating_add(instance_base_bytes);
                instance.state_entry.bytes = instance_base_bytes;
                instance.invocation_bytes = 0;
                instance.invocation = None;
                instance.invocation_history.remove(&request_id);
            }
            return Err(RuntimeResult::Failed {
                request_id,
                code: "runtime_state_unavailable".to_owned(),
            });
        }
        Ok(prepared)
    }

    /// Finalize a previously claimed invocation after its driver has run.
    /// Durable commit output remains the only execution authority; this method
    /// only publishes the protocol result and releases the active token.
    pub(crate) fn finish_invocation(
        &mut self,
        prepared: &PreparedInvocation,
        execution: Result<Value, String>,
    ) -> RuntimeResult {
        let suspended = execution
            .as_ref()
            .is_ok_and(|output| self.execution_is_waiting(&prepared.invocation_id, output));
        let result = match execution {
            Ok(output) => {
                let result = invocation_result_from_execution_output(
                    prepared.request_id.clone(),
                    prepared.invocation_id.clone(),
                    &output,
                );
                if !matches!(result, RuntimeResult::ProgramInvocationStarted { .. }) {
                    result
                } else if self.disconnected
                    || self.invocation_is_cancelled(
                        &prepared.program_instance_id,
                        &prepared.invocation_id,
                    )
                {
                    RuntimeResult::Failed {
                        request_id: prepared.request_id.clone(),
                        code: "disconnected".to_owned(),
                    }
                } else {
                    result
                }
            }
            Err(code) => RuntimeResult::Failed {
                request_id: prepared.request_id.clone(),
                code,
            },
        };
        self.active_cancellations.remove(&prepared.invocation_id);
        self.observation_signal.notify();
        if !suspended
            && let Some(instance) = self.instances.get_mut(&prepared.program_instance_id)
            && let Some(invocation) = instance.invocation.as_mut()
            && invocation.program_invocation_id == prepared.invocation_id
        {
            invocation.result = Some(result.clone());
        }
        if !suspended
            && let Some(instance) = self.instances.get_mut(&prepared.program_instance_id)
            && let Some(invocation) = instance.invocation_history.get_mut(&prepared.request_id)
        {
            invocation.result = Some(result.clone());
        }
        if self
            .persist_runtime_state()
            .and_then(|()| self.reconcile_event_waits())
            .is_err()
        {
            return RuntimeResult::Failed {
                request_id: prepared.request_id.clone(),
                code: "runtime_state_unavailable".to_owned(),
            };
        }
        if suspended
            && self.invocation_is_cancelled(&prepared.program_instance_id, &prepared.invocation_id)
        {
            // Cancellation can be acknowledged after the driver's last token
            // check but before it commits a host wait. Withdraw that newly
            // parked request under the same durable cancellation marker.
            self.cancel_outstanding_host_capabilities(&prepared.invocation_id);
        }
        result
    }

    /// A structural yield settles one invocation; only an Event wait stays open.
    fn execution_is_waiting(&self, invocation_id: &str, output: &Value) -> bool {
        output.get("status").and_then(Value::as_str) == Some("suspended")
            && self.execution_backend.invocation_status(invocation_id)
                != Some(ProgramInvocationStatus::CommittedYield)
    }

    /// Rebuild the immutable execution lease for one durably admitted pending
    /// invocation. The active cancellation token is the process-local worker
    /// claim, so concurrent identical starts cannot enqueue a duplicate.
    pub(crate) fn claim_pending_invocation(
        &mut self,
        invocation_id: &str,
    ) -> Result<Option<PreparedInvocation>, String> {
        if self.active_cancellations.contains_key(invocation_id) {
            return Ok(None);
        }
        if self.active_cancellations.len() >= MAX_ACTIVE_INVOCATIONS {
            return Err("invocation_capacity_exhausted".to_owned());
        }
        let instance_id = self
            .invocation_index
            .get(invocation_id)
            .cloned()
            .ok_or_else(|| "unknown_invocation".to_owned())?;
        if self
            .execution_backend
            .load_continuation(&ProgramInstanceRef::new(instance_id.clone()))
            .and_then(|value| serde_json::from_value::<Continuation>(value.payload).ok())
            .is_some_and(|continuation| {
                continuation.program_invocation_ref.as_str() == invocation_id
                    || continuation.event_ref.is_some()
            })
        {
            return Ok(None);
        }
        let (request_id, input, artifact_digest, bound_materials) = {
            let instance = self
                .instances
                .get(&instance_id)
                .ok_or_else(|| "unknown_instance".to_owned())?;
            let invocation = instance
                .invocation
                .as_ref()
                .filter(|value| value.program_invocation_id == invocation_id)
                .ok_or_else(|| "unknown_invocation".to_owned())?;
            if invocation.result.is_some() || invocation.phase != InvocationExecutionPhase::Pending
            {
                return Ok(None);
            }
            (
                invocation.request_id.clone(),
                invocation
                    .input
                    .clone()
                    .ok_or_else(|| "pending_invocation_input_unavailable".to_owned())?,
                instance.artifact_digest.clone(),
                instance
                    .materials
                    .clone()
                    .ok_or_else(|| "missing_invocation_admission".to_owned())?,
            )
        };
        let artifact_bytes = self
            .artifacts
            .get(&artifact_digest)
            .ok_or_else(|| "unknown_artifact".to_owned())?
            .to_vec();
        if let Err(error) = self.verify_selected_materials(&artifact_bytes, &bound_materials) {
            if error != "admission_profile_mismatch" {
                return Err(error);
            }
            let result = self.fail_pending_invocation(invocation_id, &error);
            return match result {
                RuntimeResult::Failed { code, .. } if code == "runtime_state_unavailable" => {
                    Err(code)
                }
                _ => Ok(None),
            };
        }
        let artifact = ExecutableArtifact::decode_for_execution(&artifact_bytes, &artifact_digest)
            .map_err(|_| "invalid_artifact".to_owned())?;
        validate_package_permission_resolution(&artifact.air, self.package_root.as_deref())?;
        let mut materials = bound_materials;
        invocation_id.clone_into(&mut materials.admission.invocation_id);
        let cancellation = CancellationToken::new();
        // A durable cancellation marker may precede this claim (a cancel that
        // arrived while no worker held a token, or one persisted before a
        // restart). The fresh token must carry it, or the driver would run an
        // invocation its owner already cancelled.
        if self.cancelled.contains_key(invocation_id) {
            cancellation.cancel();
        }
        self.active_cancellations
            .insert(invocation_id.to_owned(), cancellation.clone());
        Ok(Some(PreparedInvocation {
            request_id,
            program_instance_id: instance_id.clone(),
            invocation_id: invocation_id.to_owned(),
            input,
            air: artifact.air,
            artifact_bytes,
            materials,
            handlers: self.handlers.clone(),
            package_root: self.package_root.clone(),
            sandbox_registry: self.sandbox_registry.clone(),
            execution_backend: self.execution_backend.commit_port(),
            observation_sink: self.observation_sink.clone(),
            broker: self.broker.clone(),
            approval_policy: self.approval_policy,
            cancellation,
            resumable: self.resumable_invocations,
            events: self.event_port(instance_id),
        }))
    }

    /// Cross the durable worker-start marker immediately before execution.
    /// Once this succeeds, restart recovery may not safely redispatch.
    pub(crate) fn begin_invocation(&mut self, invocation_id: &str) -> Result<bool, String> {
        let instance_id = self
            .invocation_index
            .get(invocation_id)
            .cloned()
            .ok_or_else(|| "unknown_invocation".to_owned())?;
        let instance_for_profile = self.instances.get(&instance_id).ok_or("unknown_instance")?;
        let artifact_bytes = self
            .artifacts
            .get(&instance_for_profile.artifact_digest)
            .ok_or("unknown_artifact")?;
        let materials = instance_for_profile
            .materials
            .as_ref()
            .ok_or("missing_invocation_admission")?;
        if let Err(error) = self.verify_selected_materials(artifact_bytes, materials) {
            if error != "admission_profile_mismatch" {
                return Err(error);
            }
            let result = self.fail_pending_invocation(invocation_id, &error);
            return match result {
                RuntimeResult::Failed { code, .. } if code == "runtime_state_unavailable" => {
                    Err(code)
                }
                _ => Ok(false),
            };
        }
        let instance = self
            .instances
            .get_mut(&instance_id)
            .ok_or_else(|| "unknown_instance".to_owned())?;
        let Some(invocation) = instance.invocation.as_mut() else {
            return Ok(false);
        };
        if invocation.program_invocation_id != invocation_id
            || invocation.result.is_some()
            || invocation.phase != InvocationExecutionPhase::Pending
        {
            return Ok(false);
        }
        invocation.phase = InvocationExecutionPhase::Running;
        if let Some(history) = instance.invocation_history.get_mut(&invocation.request_id) {
            history.phase = InvocationExecutionPhase::Running;
        }
        if let Err(error) = self.persist_runtime_state() {
            if let Some(instance) = self.instances.get_mut(&instance_id)
                && let Some(invocation) = instance.invocation.as_mut()
            {
                invocation.phase = InvocationExecutionPhase::Pending;
                if let Some(history) = instance.invocation_history.get_mut(&invocation.request_id) {
                    history.phase = InvocationExecutionPhase::Pending;
                }
            }
            self.active_cancellations.remove(invocation_id);
            return Err(error);
        }
        Ok(true)
    }

    pub(crate) fn release_invocation_claim(&mut self, invocation_id: &str) {
        self.active_cancellations.remove(invocation_id);
    }

    /// Claim the still-parked continuation for a durably accepted host
    /// settlement. The returned lease can execute without holding the service
    /// mutex; a duplicate settlement observes the process-local claim and
    /// cannot enqueue another resume.
    pub(crate) fn claim_host_capability_resume(
        &mut self,
        capability_request_id: &str,
    ) -> Result<Option<PreparedResume>, String> {
        let settlement_state = self
            .host_capability_settlements
            .get(capability_request_id)
            .ok_or_else(|| "unknown_capability_request".to_owned())?;
        if settlement_state.resume_started {
            return Ok(None);
        }
        let instance_id = settlement_state.program_instance_id.clone();
        let settlement = settlement_state.settlement.clone();
        let delivered = Value::String(
            serde_json::to_string(&settlement)
                .map_err(|error| format!("host capability settlement encode failed: {error}"))?,
        );
        let event_ref =
            apxm_kernel::EventRef::new(capability_request_id).map_err(|error| error.to_string())?;
        self.claim_continuation_resume(instance_id, event_ref, delivered)
    }

    /// Persist the resume-start marker immediately before a worker may call a
    /// provider or capability. Restart treats this marker as a possible send.
    pub(crate) fn begin_continuation_resume(
        &mut self,
        prepared: &PreparedResume,
    ) -> Result<bool, String> {
        if !self
            .active_cancellations
            .contains_key(&prepared.invocation_id)
        {
            return Ok(false);
        }
        let Some(started) = self.resume_marker(prepared).map(|value| *value) else {
            return Ok(false);
        };
        if started {
            return Ok(false);
        }
        if let Err(error) =
            self.verify_selected_materials(&prepared.artifact_bytes, &prepared.materials)
        {
            if error != "admission_profile_mismatch" {
                return Err(error);
            }
            self.fail_unstarted_profile_resume(
                &prepared.program_instance_id,
                &prepared.invocation_id,
                &prepared.event_ref,
                prepared.expected_program_state_version,
                &prepared.committed_continuation,
            )?;
            self.active_cancellations.remove(&prepared.invocation_id);
            return Ok(false);
        }
        let Some(started) = self.resume_marker(prepared) else {
            return Ok(false);
        };
        *started = true;
        if let Err(error) = self.persist_runtime_state() {
            if let Some(started) = self.resume_marker(prepared) {
                *started = false;
            }
            self.active_cancellations.remove(&prepared.invocation_id);
            return Err(error);
        }
        Ok(true)
    }

    pub(crate) fn finish_continuation_resume(
        &mut self,
        prepared: &PreparedResume,
        output: Result<Value, String>,
    ) -> RuntimeResult {
        self.active_cancellations.remove(&prepared.invocation_id);
        let suspended = output
            .as_ref()
            .is_ok_and(|output| self.execution_is_waiting(&prepared.invocation_id, output));
        let result = match output {
            Ok(output) => invocation_result_from_execution_output(
                prepared.invocation_request_id.clone(),
                prepared.invocation_id.clone(),
                &output,
            ),
            Err(_)
                if self.invocation_is_cancelled(
                    &prepared.program_instance_id,
                    &prepared.invocation_id,
                ) && self
                    .host_capability_settlements
                    .get(prepared.event_ref.as_str())
                    .is_some_and(|state| {
                        state.settlement.outcome == HostCapabilityOutcomeKind::Cancelled
                            && !state.resume_started
                    })
                    && matches!(
                        self.execution_backend
                            .invocation_status(&prepared.invocation_id),
                        Some(ProgramInvocationStatus::Cancelled),
                    ) =>
            {
                RuntimeResult::Cancelled {
                    request_id: prepared.invocation_request_id.clone(),
                }
            }
            Err(_) => RuntimeResult::Failed {
                request_id: prepared.invocation_request_id.clone(),
                code: "outcome_unknown".to_owned(),
            },
        };
        if !suspended
            && let Some(instance) = self.instances.get_mut(&prepared.program_instance_id)
            && let Some(invocation) = instance.invocation.as_mut()
            && invocation.program_invocation_id == prepared.invocation_id
        {
            invocation.result = Some(result.clone());
            if let Some(history) = instance
                .invocation_history
                .get_mut(&prepared.invocation_request_id)
            {
                history.result = Some(result.clone());
            }
        }
        self.observation_signal.notify();
        if self
            .persist_runtime_state()
            .and_then(|()| self.reconcile_event_waits())
            .is_err()
        {
            return RuntimeResult::Failed {
                request_id: prepared.invocation_request_id.clone(),
                code: "runtime_state_unavailable".to_owned(),
            };
        }
        result
    }

    /// Recover only settlements whose resume-start marker was never crossed.
    /// If a started resume still points at the same continuation, execution
    /// may have reached an external effect and is made uncertain instead of
    /// being sent again.
    #[cfg(test)]
    pub(crate) fn recover_continuation_resumes(&mut self) -> Result<Vec<PreparedResume>, String> {
        self.reconcile_started_continuation_resumes()?;
        let mut pending = Vec::new();
        while let Some(prepared) = self.claim_next_continuation_resume()? {
            pending.push(prepared);
        }
        Ok(pending)
    }

    /// Reconcile a resume whose durable start marker was crossed before the
    /// previous process stopped. Such work may already have reached an
    /// external effect, so it becomes uncertain and is never redispatched.
    fn reconcile_started_continuation_resumes(&mut self) -> Result<(), String> {
        let targets = self
            .host_capability_settlements
            .iter()
            .filter(|(_, state)| state.resume_started)
            .map(|(request_id, state)| {
                (
                    state.program_instance_id.clone(),
                    apxm_kernel::EventRef::HostCapability {
                        request_id: request_id.clone(),
                    },
                )
            })
            .chain(
                self.reservations
                    .iter()
                    .filter(|(_, state)| state.resume_started)
                    .map(|((event_id, generation), state)| {
                        (
                            state.program_instance_id.clone(),
                            apxm_kernel::EventRef::Reserved {
                                reference: CanonicalEventRef {
                                    event_id: event_id.clone(),
                                    generation: *generation,
                                },
                            },
                        )
                    }),
            )
            .collect::<Vec<_>>();
        let mut uncertain = Vec::new();
        for (instance_id, target) in targets {
            let Some(invocation) = self
                .instances
                .get(&instance_id)
                .and_then(|instance| instance.invocation.as_ref())
            else {
                continue;
            };
            if self
                .active_cancellations
                .contains_key(&invocation.program_invocation_id)
            {
                continue;
            }
            let same_continuation = self
                .execution_backend
                .load_continuation(&ProgramInstanceRef::new(instance_id.clone()))
                .and_then(|committed| {
                    serde_json::from_value::<Continuation>(committed.payload).ok()
                })
                .is_some_and(|continuation| {
                    continuation.event_ref.as_ref() == Some(&target)
                        && continuation.program_invocation_ref.as_str()
                            == invocation.program_invocation_id
                        && continuation.program_instance_ref.as_str() == instance_id
                });
            if !same_continuation {
                continue;
            }
            uncertain.push(instance_id);
        }
        let mut changed = false;
        for instance_id in uncertain {
            let Some(instance) = self.instances.get_mut(&instance_id) else {
                continue;
            };
            let Some(invocation) = instance.invocation.as_mut() else {
                continue;
            };
            if invocation.result.is_some() {
                continue;
            }
            let result = RuntimeResult::Failed {
                request_id: invocation.request_id.clone(),
                code: "outcome_unknown".to_owned(),
            };
            invocation.result = Some(result.clone());
            if let Some(history) = instance.invocation_history.get_mut(&invocation.request_id) {
                history.result = Some(result);
            }
            changed = true;
        }
        if changed {
            self.persist_runtime_state()?;
        }
        Ok(())
    }

    /// Claim one durable host settlement for bounded recovery. Reaching the
    /// process-local execution limit is ordinary backpressure: the settlement
    /// remains pending and a later dispatcher drain retries it.
    pub(crate) fn claim_next_continuation_resume(
        &mut self,
    ) -> Result<Option<PreparedResume>, String> {
        self.reconcile_reserved_event_resumes()?;
        if self.active_cancellations.len() >= MAX_ACTIVE_INVOCATIONS {
            return Ok(None);
        }
        let capability_request_ids = self
            .host_capability_settlements
            .keys()
            .filter(|capability_request_id| {
                !self.host_capability_settlements[*capability_request_id].resume_started
            })
            .cloned()
            .collect::<Vec<_>>();
        for capability_request_id in capability_request_ids {
            if let Some(prepared) = self.claim_host_capability_resume(&capability_request_id)? {
                return Ok(Some(prepared));
            }
        }
        let pending_events = self
            .reservations
            .iter()
            .filter(|(_, state)| {
                !state.resume_started
                    && state.binding.is_some()
                    && state.status != EventStatus::Pending
            })
            .map(|((event_id, generation), _)| CanonicalEventRef {
                event_id: event_id.clone(),
                generation: *generation,
            })
            .collect::<Vec<_>>();
        for reference in pending_events {
            if let Some(prepared) = self.claim_reserved_event_resume(&reference)? {
                return Ok(Some(prepared));
            }
        }
        Ok(None)
    }

    /// Convert a durably pending invocation into a durable refusal when the
    /// bounded dispatcher cannot reserve execution capacity. This keeps the
    /// replay identity authoritative: an identical retry observes the same
    /// refusal and cannot create a second invocation.
    pub(crate) fn fail_pending_invocation(
        &mut self,
        invocation_id: &str,
        code: &str,
    ) -> RuntimeResult {
        self.active_cancellations.remove(invocation_id);
        let Some(instance_id) = self.invocation_index.get(invocation_id).cloned() else {
            return RuntimeResult::Failed {
                request_id: "unknown".to_owned(),
                code: "unknown_invocation".to_owned(),
            };
        };
        let Some(instance) = self.instances.get_mut(&instance_id) else {
            return RuntimeResult::Failed {
                request_id: "unknown".to_owned(),
                code: "unknown_instance".to_owned(),
            };
        };
        let Some(invocation) = instance
            .invocation
            .as_mut()
            .filter(|value| value.program_invocation_id == invocation_id)
        else {
            return RuntimeResult::Failed {
                request_id: "unknown".to_owned(),
                code: "unknown_invocation".to_owned(),
            };
        };
        let request_id = invocation.request_id.clone();
        if let Some(result) = invocation.result.clone() {
            return result;
        }
        if invocation.phase != InvocationExecutionPhase::Pending {
            return RuntimeResult::Failed {
                request_id,
                code: "outcome_unknown".to_owned(),
            };
        }
        let result = RuntimeResult::Failed {
            request_id: request_id.clone(),
            code: code.to_owned(),
        };
        invocation.result = Some(result.clone());
        if let Some(history) = instance.invocation_history.get_mut(&request_id) {
            history.result = Some(result.clone());
        }
        if self.persist_runtime_state().is_err() {
            if let Some(instance) = self.instances.get_mut(&instance_id) {
                if let Some(invocation) = instance.invocation.as_mut() {
                    invocation.result = None;
                }
                if let Some(history) = instance.invocation_history.get_mut(&request_id) {
                    history.result = None;
                }
            }
            return RuntimeResult::Failed {
                request_id,
                code: "runtime_state_unavailable".to_owned(),
            };
        }
        result
    }

    /// Reconcile durable pending/running invocations at process startup.
    /// Pending work was never allowed to reach an effect and may be queued.
    /// Running work is never resent: a terminal commit is replayed, a parked
    /// continuation remains parked, and every other case becomes uncertain.
    #[cfg(test)]
    pub(crate) fn recover_invocations(&mut self) -> Result<Vec<PreparedInvocation>, String> {
        self.reconcile_started_invocations()?;
        let mut pending = Vec::new();
        while let Some(prepared) = self.claim_next_pending_invocation()? {
            pending.push(prepared);
        }
        Ok(pending)
    }

    /// Reconcile running invocations without claiming pending work. This is
    /// separated from bounded dispatch so startup can become responsive even
    /// when more durable work exists than the in-process execution limit.
    fn reconcile_started_invocations(&mut self) -> Result<(), String> {
        let candidates = self
            .instances
            .values()
            .filter_map(|instance| instance.invocation.as_ref())
            .filter(|invocation| invocation.result.is_none())
            .map(|invocation| invocation.program_invocation_id.clone())
            .collect::<Vec<_>>();
        let mut changed = false;
        for invocation_id in candidates {
            let instance_id = self
                .invocation_index
                .get(&invocation_id)
                .cloned()
                .ok_or_else(|| "unknown_invocation".to_owned())?;
            if self
                .execution_backend
                .load_continuation(&ProgramInstanceRef::new(instance_id.clone()))
                .and_then(|value| serde_json::from_value::<Continuation>(value.payload).ok())
                .is_some_and(|continuation| {
                    continuation.program_invocation_ref.as_str() == invocation_id
                        && continuation.event_ref.is_some()
                })
            {
                continue;
            }
            let phase = self
                .instances
                .get(&instance_id)
                .and_then(|instance| instance.invocation.as_ref())
                .map(|invocation| invocation.phase)
                .ok_or_else(|| "unknown_invocation".to_owned())?;
            if phase == InvocationExecutionPhase::Pending {
                continue;
            }
            let request_id = self
                .instances
                .get(&instance_id)
                .and_then(|instance| instance.invocation.as_ref())
                .map(|invocation| invocation.request_id.clone())
                .ok_or_else(|| "unknown_invocation".to_owned())?;
            let result = match self.execution_backend.invocation_status(&invocation_id) {
                Some(
                    apxm_runtime_protocol::ProgramInvocationStatus::CommittedReturn
                    | apxm_runtime_protocol::ProgramInvocationStatus::CommittedYield,
                ) => RuntimeResult::ProgramInvocationStarted {
                    request_id: request_id.clone(),
                    program_invocation_id: invocation_id.clone(),
                },
                Some(apxm_runtime_protocol::ProgramInvocationStatus::Failed) => {
                    RuntimeResult::Failed {
                        request_id: request_id.clone(),
                        code: self
                            .execution_backend
                            .terminal_failure_code(&invocation_id)
                            .unwrap_or_else(|| "invocation_failed".to_owned()),
                    }
                }
                Some(apxm_runtime_protocol::ProgramInvocationStatus::Cancelled) => {
                    RuntimeResult::Cancelled {
                        request_id: request_id.clone(),
                    }
                }
                _ => RuntimeResult::Failed {
                    request_id: request_id.clone(),
                    code: "outcome_unknown".to_owned(),
                },
            };
            if let Some(instance) = self.instances.get_mut(&instance_id)
                && let Some(invocation) = instance.invocation.as_mut()
            {
                invocation.result = Some(result.clone());
                if let Some(history) = instance.invocation_history.get_mut(&request_id) {
                    history.result = Some(result);
                }
                changed = true;
            }
        }
        if changed {
            self.persist_runtime_state()?;
        }
        Ok(())
    }

    /// Reconcile all possible-send markers before the recovery dispatcher is
    /// allowed to claim safe pending work.
    pub(crate) fn reconcile_recovery_state(&mut self) -> Result<(), String> {
        self.reconcile_started_invocations()?;
        self.reconcile_cancelled_parked_invocations()?;
        self.reconcile_started_continuation_resumes()?;
        self.reconcile_reserved_event_resumes()
    }

    /// Complete the cancellation transition for a parked invocation after a
    /// restart. The cancellation marker is durable before host withdrawal is
    /// attempted, so a crash in that window must recreate the synthetic host
    /// settlement instead of leaving the continuation parked forever.
    fn reconcile_cancelled_parked_invocations(&mut self) -> Result<(), String> {
        let candidates = self
            .instances
            .iter()
            .filter_map(|(instance_id, instance)| {
                let invocation = instance.invocation.as_ref()?;
                if invocation.result.is_some()
                    || !self
                        .cancelled
                        .contains_key(&invocation.program_invocation_id)
                {
                    return None;
                }
                let committed = self
                    .execution_backend
                    .load_continuation(&ProgramInstanceRef::new(instance_id.clone()))?;
                let continuation: Continuation = serde_json::from_value(committed.payload).ok()?;
                let capability_request_id = continuation.event_ref?.as_str().to_owned();
                if self
                    .host_capability_settlements
                    .get(&capability_request_id)
                    .is_some_and(|state| state.resume_started)
                {
                    // A crossed resume marker is a possible external send;
                    // the uncertainty reconciler below owns that case.
                    return None;
                }
                is_host_capability_request_id(&capability_request_id).then(|| {
                    (
                        instance.owner_claim.clone(),
                        capability_request_id,
                        invocation.request_id.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();

        for (owner_claim, capability_request_id, request_id) in candidates {
            match self.settle_host_capability(
                format!("{request_id}.cancel.recovery"),
                owner_claim,
                capability_request_id,
                HostCapabilityOutcomeKind::Cancelled,
                None,
                None,
                Some("the Program Invocation was cancelled".to_owned()),
                true,
            ) {
                RuntimeResult::CapabilitySettled { .. } => {}
                RuntimeResult::Failed { code, .. } => {
                    return Err(format!("cancelled host recovery failed: {code}"));
                }
                other => {
                    return Err(format!(
                        "cancelled host recovery returned unexpected result: {other:?}"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Claim one durably admitted invocation for bounded recovery. Capacity
    /// exhaustion leaves every remaining invocation pending for the recovery
    /// dispatcher rather than making service startup fail.
    pub(crate) fn claim_next_pending_invocation(
        &mut self,
    ) -> Result<Option<PreparedInvocation>, String> {
        if self.active_cancellations.len() >= MAX_ACTIVE_INVOCATIONS {
            return Ok(None);
        }
        let invocation_ids = self
            .instances
            .values()
            .filter_map(|instance| instance.invocation.as_ref())
            .filter(|invocation| {
                invocation.result.is_none()
                    && invocation.phase == InvocationExecutionPhase::Pending
                    && !self
                        .active_cancellations
                        .contains_key(&invocation.program_invocation_id)
            })
            .map(|invocation| invocation.program_invocation_id.clone())
            .collect::<Vec<_>>();
        for invocation_id in invocation_ids {
            if let Some(prepared) = self.claim_pending_invocation(&invocation_id)? {
                return Ok(Some(prepared));
            }
        }
        Ok(None)
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
        let Some(instance_id) = self.invocation_index.get(&program_invocation_id).cloned() else {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_invocation".to_owned(),
            };
        };
        let Some(instance) = self.instances.get(&instance_id) else {
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
            // Cancellation markers are durable owner intent. A prior attempt
            // may have been interrupted while withdrawing a parked host
            // request, so an idempotent retry must repair that transition
            // rather than merely acknowledge the marker forever.
            self.cancel_outstanding_host_capabilities(&program_invocation_id);
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
        let mut marker = Self::entry(marker_bytes, self.state_policy.cancellations.ttl);
        if let Some(invocation) = instance.invocation.as_ref() {
            if marker.expires_at < instance.state_entry.expires_at {
                marker.expires_at = instance.state_entry.expires_at;
            }
            debug_assert_eq!(invocation.program_invocation_id, program_invocation_id);
        }
        let cancellation_id = program_invocation_id.clone();
        self.cancelled.insert(cancellation_id.clone(), marker);
        self.cancellation_bytes = self.cancellation_bytes.saturating_add(marker_bytes);
        if self.persist_runtime_state().is_err() {
            self.cancelled.remove(&cancellation_id);
            self.cancellation_bytes = self.cancellation_bytes.saturating_sub(marker_bytes);
            return RuntimeResult::Failed {
                request_id,
                code: "runtime_state_unavailable".to_owned(),
            };
        }
        // Publish the durable marker before signalling the driver. The
        // service mutex makes this an atomic owner transition: finalization
        // cannot publish a successful protocol result without observing the
        // cancellation decision.
        if let Some(cancellation) = self.active_cancellations.get(&cancellation_id) {
            cancellation.cancel();
        }
        // A live invocation observes the token at its next node boundary. A
        // parked one has no token to observe: it is waiting on a host that will
        // never answer now, so its outstanding requests are withdrawn here and
        // the invocation settles rather than staying parked on a cancellation
        // marker nothing acts on (ADR-0025).
        self.cancel_outstanding_host_capabilities(&program_invocation_id);
        RuntimeResult::Cancelled { request_id }
    }

    fn invocation_is_cancelled(&self, program_instance_id: &str, invocation_id: &str) -> bool {
        let _ = program_instance_id;
        self.cancelled.contains_key(invocation_id)
    }

    fn reserve_event(
        &mut self,
        request_id: String,
        program_instance_id: String,
        instance_owner_claim: RuntimeOwnerClaim,
        type_id: String,
    ) -> Result<RuntimeResult, ProtocolError> {
        instance_owner_claim.validate()?;
        let instance = self
            .instances
            .get(&program_instance_id)
            .ok_or(ProtocolError::OwnerMismatch)?;
        if instance.owner_claim != instance_owner_claim {
            return Err(ProtocolError::OwnerMismatch);
        }
        if request_id.trim().is_empty() || type_id.trim().is_empty() {
            return Err(ProtocolError::ForbiddenEventMethod);
        }
        for ((event_id, generation), prior) in &self.reservations {
            if prior.program_instance_id == program_instance_id && prior.request_id == request_id {
                if prior.type_id != type_id {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "invalid_request".into(),
                    });
                }
                return Ok(RuntimeResult::EventReserved {
                    request_id,
                    owner_claim: prior.owner_claim.clone(),
                    event_ref: CanonicalEventRef {
                        event_id: event_id.clone(),
                        generation: *generation,
                    },
                });
            }
        }
        if instance.materials.is_none()
            || instance.invocation.as_ref().is_some_and(|invocation| {
                let status = self
                    .execution_backend
                    .invocation_status(&invocation.program_invocation_id);
                self.cancelled
                    .contains_key(&invocation.program_invocation_id)
                    || matches!(
                        status,
                        Some(
                            ProgramInvocationStatus::CommittedReturn
                                | ProgramInvocationStatus::Cancelled
                                | ProgramInvocationStatus::CancellationUnconfirmed
                                | ProgramInvocationStatus::Failed
                                | ProgramInvocationStatus::OutcomeUnknown
                        )
                    )
                    || (invocation.result.is_some()
                        && status != Some(ProgramInvocationStatus::CommittedYield))
            })
        {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "instance_unavailable".into(),
            });
        }
        let artifact = self
            .artifacts
            .get(&instance.artifact_digest)
            .and_then(|bytes| {
                ExecutableArtifact::decode_for_execution(bytes, &instance.artifact_digest).ok()
            })
            .ok_or(ProtocolError::ForbiddenEventMethod)?;
        let Some(requirement) = artifact
            .air
            .event_requirements
            .iter()
            .find(|requirement| requirement.type_id == type_id)
        else {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "undeclared_event_type".into(),
            });
        };
        let payload_schema = requirement.payload_schema.clone();
        let schema_digest = payload_schema
            .canonical_digest()
            .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
        if schema_digest != requirement.schema_digest {
            return Err(ProtocolError::ForbiddenEventMethod);
        }
        let reservation_size = serde_json::to_vec(&payload_schema)
            .map_err(|_| ProtocolError::ForbiddenEventMethod)?
            .len()
            .saturating_add(request_id.len())
            .saturating_add(program_instance_id.len())
            .saturating_add(type_id.len())
            .saturating_add(schema_digest.len())
            .saturating_add(1024);
        let reservation_bytes = u64::try_from(reservation_size).unwrap_or(u64::MAX);
        if !Self::quota_available(
            self.state_policy.reservations,
            self.reservations.len(),
            self.reservation_bytes,
            reservation_bytes,
        ) {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: Self::quota_code("reservation"),
            });
        }
        let Some(generation) = self
            .next_generation
            .checked_add(1)
            .filter(|generation| *generation <= 9_007_199_254_740_991)
        else {
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "generation_exhausted".into(),
            });
        };
        let owner_claim = RuntimeOwnerClaim::mint();
        let event_ref = CanonicalEventRef {
            event_id: format!("evt-{}", Uuid::new_v4()),
            generation,
        };
        self.next_generation = generation;
        self.reservations.insert(
            (event_ref.event_id.clone(), generation),
            ReservationState {
                request_id: request_id.clone(),
                program_instance_id,
                owner_claim: owner_claim.clone(),
                type_id,
                payload_schema,
                schema_digest,
                binding: None,
                resume_started: false,
                status: EventStatus::Pending,
                occurrence_id: None,
                state_entry: Self::entry(reservation_bytes, self.state_policy.reservations.ttl),
            },
        );
        self.reservation_bytes = self.reservation_bytes.saturating_add(reservation_bytes);
        if self.persist_runtime_state().is_err() {
            self.reservations
                .remove(&(event_ref.event_id.clone(), generation));
            self.reservation_bytes = self.reservation_bytes.saturating_sub(reservation_bytes);
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "runtime_state_unavailable".into(),
            });
        }
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
        if let Some(prior) = self.applications.get(&key).or_else(|| {
            self.applications
                .values()
                .find(|prior| prior.event_ref == application.event_ref)
        }) {
            if prior.payload != application.occurrence.payload
                || prior.occurrence_id != application.occurrence.occurrence_id
                || prior.event_ref != application.event_ref
                || prior.source_kind != application.occurrence.source_kind
                || prior.mapping_digest != application.occurrence.mapping_digest
                || prior.source_record != application.occurrence.source_record
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
        if reservation.status != EventStatus::Pending {
            let result = match reservation.status {
                EventStatus::Fulfilled => EventApplicationResult::Rejected,
                EventStatus::Expired => EventApplicationResult::Expired,
                EventStatus::Cancelled => EventApplicationResult::Cancelled,
                EventStatus::Pending => unreachable!(),
            };
            return Ok(RuntimeResult::EventApplied { request_id, result });
        }
        if reservation
            .payload_schema
            .validate_value(&application.occurrence.payload)
            .is_err()
        {
            return Ok(RuntimeResult::EventApplied {
                request_id,
                result: EventApplicationResult::Rejected,
            });
        }
        let payload_bytes = u64::try_from(
            serde_json::to_vec(&application.occurrence)
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
            self.applications
                .len()
                .saturating_add(self.host_capability_settlements.len()),
            self.application_bytes
                .saturating_add(self.host_capability_settlement_bytes),
            application_bytes,
        ) {
            return Ok(RuntimeResult::EventApplied {
                request_id,
                result: EventApplicationResult::Rejected,
            });
        }
        self.applications.insert(
            key.clone(),
            ApplicationState {
                event_ref: application.event_ref.clone(),
                occurrence_id: application.occurrence.occurrence_id.clone(),
                source_kind: application.occurrence.source_kind.clone(),
                mapping_digest: application.occurrence.mapping_digest.clone(),
                source_record: application.occurrence.source_record.clone(),
                payload: application.occurrence.payload.clone(),
                state_entry: Self::entry(application_bytes, self.state_policy.applications.ttl),
            },
        );
        self.application_bytes = self.application_bytes.saturating_add(application_bytes);
        if let Some(reservation) = self.reservations.get_mut(&(
            application.event_ref.event_id.clone(),
            application.event_ref.generation,
        )) {
            reservation.status = EventStatus::Fulfilled;
            reservation.occurrence_id = Some(application.occurrence.occurrence_id.clone());
        }
        if self.persist_runtime_state().is_err() {
            self.applications.remove(&key);
            self.application_bytes = self.application_bytes.saturating_sub(application_bytes);
            if let Some(reservation) = self.reservations.get_mut(&(
                application.event_ref.event_id.clone(),
                application.event_ref.generation,
            )) {
                reservation.status = EventStatus::Pending;
                reservation.occurrence_id = None;
            }
            return Ok(RuntimeResult::EventApplied {
                request_id,
                result: EventApplicationResult::Rejected,
            });
        }
        // Acceptance is durable before acknowledgment. The existing bounded
        // dispatcher claims the same resume lease as host settlements; no
        // driver runs under the service mutex or inside this HTTP call.
        Ok(RuntimeResult::EventApplied {
            request_id,
            result: EventApplicationResult::Fulfilled,
        })
    }

    /// Settle one outstanding host-fulfilled Capability request.
    ///
    /// The settlement is durably recorded before the continuation resumes.
    /// That closes both crash windows: a retry can resume a still-parked node,
    /// or return the same terminal result after the node already advanced.
    #[allow(clippy::too_many_arguments)]
    fn settle_host_capability(
        &mut self,
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
        capability_request_id: String,
        outcome: HostCapabilityOutcomeKind,
        output: Option<String>,
        receipt_ref: Option<String>,
        message: Option<String>,
        resume_now: bool,
    ) -> RuntimeResult {
        if owner_claim.validate().is_err() {
            return RuntimeResult::Failed {
                request_id,
                code: "invalid_owner_claim".to_owned(),
            };
        }
        let settlement = HostCapabilitySettlement::new(
            capability_request_id.clone(),
            outcome,
            output,
            receipt_ref,
            message,
        );
        if !settlement.is_well_formed() {
            return RuntimeResult::Failed {
                request_id,
                code: "invalid_request".to_owned(),
            };
        }
        // Once the durable invocation cancellation marker exists, a late
        // host fulfillment cannot replace the cancellation-owned outcome.
        // The cancellation path (and recovery) still uses the synthetic
        // Cancelled settlement below to close the parked continuation.
        if outcome != HostCapabilityOutcomeKind::Cancelled
            && let Some(instance_id) = self.parked_host_capability_instance(&capability_request_id)
            && self
                .instances
                .get(&instance_id)
                .and_then(|instance| instance.invocation.as_ref())
                .is_some_and(|invocation| {
                    self.invocation_is_cancelled(&instance_id, &invocation.program_invocation_id)
                })
        {
            return RuntimeResult::Failed {
                request_id,
                code: "invalid_request".to_owned(),
            };
        }
        if let Some(prior) = self.host_capability_settlements.get(&capability_request_id) {
            if prior.owner_claim != owner_claim {
                return RuntimeResult::Failed {
                    request_id,
                    code: "owner_mismatch".to_owned(),
                };
            }
            if prior.settlement != settlement {
                // A host settlement may already be durably recorded while its
                // resume is still unstarted. Explicit invocation cancellation
                // wins that pre-dispatch race, but preserves the exact host
                // settlement document for the canonical cancellation commit.
                if outcome == HostCapabilityOutcomeKind::Cancelled
                    && !prior.resume_started
                    && let Some(instance_id) =
                        self.parked_host_capability_instance(&capability_request_id)
                    && let Some(invocation) = self
                        .instances
                        .get(&instance_id)
                        .and_then(|instance| instance.invocation.as_ref())
                    && self.invocation_is_cancelled(&instance_id, &invocation.program_invocation_id)
                {
                    let program_instance_id = prior.program_instance_id.clone();
                    let prior_settlement = prior.settlement.clone();
                    if resume_now
                        && (self.parked_host_capability_instance(&capability_request_id)
                            != Some(program_instance_id.clone())
                            || self
                                .resume_host_capability_settlement(&prior_settlement)
                                .is_err())
                    {
                        return RuntimeResult::Failed {
                            request_id,
                            code: "capability_settlement_failed".to_owned(),
                        };
                    }
                    return RuntimeResult::CapabilitySettled {
                        request_id,
                        capability_request_id,
                        outcome: prior_settlement.outcome,
                    };
                }
                return RuntimeResult::Failed {
                    request_id,
                    code: "invalid_request".to_owned(),
                };
            }
            let program_instance_id = prior.program_instance_id.clone();
            let prior_settlement = prior.settlement.clone();
            if resume_now
                && let Some(parked_instance_id) =
                    self.parked_host_capability_instance(&capability_request_id)
                && (parked_instance_id != program_instance_id
                    || self
                        .resume_host_capability_settlement(&prior_settlement)
                        .is_err())
            {
                return RuntimeResult::Failed {
                    request_id,
                    code: "capability_settlement_failed".to_owned(),
                };
            }
            return RuntimeResult::CapabilitySettled {
                request_id,
                capability_request_id,
                outcome,
            };
        }
        let Some(instance_id) = self.parked_host_capability_instance(&capability_request_id) else {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_capability_request".to_owned(),
            };
        };
        let Some(instance) = self.instances.get(&instance_id) else {
            return RuntimeResult::Failed {
                request_id,
                code: "unknown_capability_request".to_owned(),
            };
        };
        if instance.owner_claim != owner_claim {
            return RuntimeResult::Failed {
                request_id,
                code: "owner_mismatch".to_owned(),
            };
        }
        let Some(settlement_bytes) =
            Self::host_capability_settlement_size(&instance_id, &owner_claim, &settlement)
        else {
            return RuntimeResult::Failed {
                request_id,
                code: "internal_error".to_owned(),
            };
        };
        if !Self::quota_available(
            self.state_policy.applications,
            self.applications
                .len()
                .saturating_add(self.host_capability_settlements.len()),
            self.application_bytes
                .saturating_add(self.host_capability_settlement_bytes),
            settlement_bytes,
        ) {
            return RuntimeResult::Failed {
                request_id,
                code: Self::quota_code("capability_settlement"),
            };
        }
        let mut state_entry = Self::entry(settlement_bytes, self.state_policy.applications.ttl);
        if state_entry.expires_at < instance.state_entry.expires_at {
            state_entry.expires_at = instance.state_entry.expires_at;
        }
        self.host_capability_settlements.insert(
            capability_request_id.clone(),
            HostCapabilitySettlementState {
                program_instance_id: instance_id,
                owner_claim,
                settlement: settlement.clone(),
                resume_started: false,
                state_entry,
            },
        );
        self.host_capability_settlement_bytes = self
            .host_capability_settlement_bytes
            .saturating_add(settlement_bytes);
        if self.persist_runtime_state().is_err() {
            self.host_capability_settlements
                .remove(&capability_request_id);
            self.host_capability_settlement_bytes = self
                .host_capability_settlement_bytes
                .saturating_sub(settlement_bytes);
            return RuntimeResult::Failed {
                request_id,
                code: "runtime_state_unavailable".to_owned(),
            };
        }
        if resume_now && self.resume_host_capability_settlement(&settlement).is_err() {
            return RuntimeResult::Failed {
                request_id,
                code: "capability_settlement_failed".to_owned(),
            };
        }
        RuntimeResult::CapabilitySettled {
            request_id,
            capability_request_id,
            outcome,
        }
    }

    fn resume_host_capability_settlement(
        &mut self,
        settlement: &HostCapabilitySettlement,
    ) -> Result<(), String> {
        // An invocation cancellation owns a still-parked continuation before
        // any resume worker has crossed its durable start marker. Complete the
        // invocation directly instead of executing the synthetic cancelled
        // host value: executing it can commit an outcome_unknown continuation
        // before the service gets a chance to publish cancellation.
        if let Some(instance_id) =
            self.parked_host_capability_instance(&settlement.capability_request_id)
            && let Some(invocation_id) = self
                .instances
                .get(&instance_id)
                .and_then(|instance| instance.invocation.as_ref())
                .map(|invocation| invocation.program_invocation_id.clone())
            && self.invocation_is_cancelled(&instance_id, &invocation_id)
            && self
                .host_capability_settlements
                .get(&settlement.capability_request_id)
                .is_some_and(|state| !state.resume_started)
        {
            let Some(prepared) =
                self.claim_host_capability_resume(&settlement.capability_request_id)?
            else {
                return Ok(());
            };
            // This is a cancellation-owned pre-dispatch path: do not cross
            // the resume_started marker, but do run the canonical execution
            // cancellation path so it commits InvocationCancelled facts and
            // observations. A restart can safely retry this same cancellation
            // while the marker remains unstarted, preserving any exact host
            // settlement document that was already durable.
            prepared.cancellation.cancel();
            let output = prepared.execute();
            self.finish_continuation_resume(&prepared, output);
            return Ok(());
        }
        let Some(prepared) =
            self.claim_host_capability_resume(&settlement.capability_request_id)?
        else {
            return Ok(());
        };
        if !self.begin_continuation_resume(&prepared)? {
            self.release_invocation_claim(&prepared.invocation_id);
            return Ok(());
        }
        let output = prepared.execute();
        self.finish_continuation_resume(&prepared, output);
        Ok(())
    }

    /// A host request is actionable only after its continuation is committed.
    /// Keep its live suffix behind the same boundary so a read cursor cannot
    /// advance past the request before the durable stream contains it.
    fn readable_live_observations(
        &self,
        observations: Vec<ExecutionObservation>,
    ) -> Vec<ExecutionObservation> {
        let mut barriers = BTreeMap::<String, u64>::new();
        for observation in &observations {
            if observation.observation_kind
                != apxm_runtime_protocol::ObservationKind::CapabilityRequested
            {
                continue;
            }
            let Some(request) = &observation.host_capability else {
                continue;
            };
            if self
                .host_capability_settlements
                .contains_key(&request.capability_request_id)
                || self
                    .parked_host_capability_instance(&request.capability_request_id)
                    .is_some()
            {
                continue;
            }
            barriers
                .entry(observation.program_invocation_id.as_str().to_owned())
                .and_modify(|sequence| *sequence = (*sequence).min(observation.sequence))
                .or_insert(observation.sequence);
        }
        observations
            .into_iter()
            .filter(|observation| {
                barriers
                    .get(observation.program_invocation_id.as_str())
                    .is_none_or(|sequence| observation.sequence < *sequence)
            })
            .collect()
    }

    /// The instance whose parked continuation is waiting on this request.
    fn parked_host_capability_instance(&self, capability_request_id: &str) -> Option<String> {
        self.instances
            .keys()
            .find(|instance_id| {
                self.execution_backend
                    .load_continuation(&ProgramInstanceRef::new((*instance_id).clone()))
                    .and_then(|committed| {
                        serde_json::from_value::<Continuation>(committed.payload).ok()
                    })
                    .and_then(|continuation| continuation.event_ref)
                    .is_some_and(|reference| reference.as_str() == capability_request_id)
            })
            .cloned()
    }

    /// Withdraw every host capability request this invocation left outstanding.
    ///
    /// Cancelling an invocation cancels the requests it is waiting on. Without
    /// this a cancelled invocation would stay parked forever on a request no
    /// host will ever answer, and the cancellation would be a marker rather
    /// than an outcome.
    fn cancel_outstanding_host_capabilities(&mut self, program_invocation_id: &str) {
        let outstanding = self
            .instances
            .iter()
            .filter_map(|(instance_id, instance)| {
                let invocation = instance.invocation.as_ref()?;
                if invocation.program_invocation_id != program_invocation_id {
                    return None;
                }
                let committed = self
                    .execution_backend
                    .load_continuation(&ProgramInstanceRef::new(instance_id.clone()))?;
                let continuation: Continuation = serde_json::from_value(committed.payload).ok()?;
                let event_ref = continuation.event_ref?;
                is_host_capability_request_id(event_ref.as_str())
                    .then(|| (event_ref.as_str().to_owned(), instance.owner_claim.clone()))
            })
            .collect::<Vec<_>>();
        for (capability_request_id, owner_claim) in outstanding {
            let _ = self.settle_host_capability(
                "invocation.cancel.host-capability".to_owned(),
                owner_claim,
                capability_request_id,
                HostCapabilityOutcomeKind::Cancelled,
                None,
                None,
                Some("the Program Invocation was cancelled".to_owned()),
                true,
            );
        }
    }

    fn load_persisted_artifact(&self, digest: &str) -> Option<Vec<u8>> {
        let dir = self.artifact_dir.as_ref()?;
        if !apxm_core::grammar::is_digest(digest) {
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
        if bytes.len() as u64 > MAX_ARTIFACT_BYTES
            || canonical_artifact_digest(&bytes).ok().as_deref() != Some(digest)
            || ExecutableArtifact::decode_for_execution(&bytes, digest).is_err()
        {
            return None;
        }
        Some(bytes)
    }

    fn event_inspection(
        event_ref: &CanonicalEventRef,
        reservation: &ReservationState,
    ) -> EventInspection {
        EventInspection {
            event_ref: event_ref.clone(),
            type_id: reservation.type_id.clone(),
            status: reservation.status,
            occurrence_id: reservation.occurrence_id.clone(),
        }
    }

    fn list_events(
        &mut self,
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
    ) -> Result<RuntimeResult, ProtocolError> {
        owner_claim.validate()?;
        let events = self
            .reservations
            .iter()
            .filter(|(_, reservation)| {
                reservation.owner_claim == owner_claim && reservation.status == EventStatus::Pending
            })
            .map(|((event_id, generation), reservation)| {
                Self::event_inspection(
                    &CanonicalEventRef {
                        event_id: event_id.clone(),
                        generation: *generation,
                    },
                    reservation,
                )
            })
            .collect();
        Ok(RuntimeResult::EventListed { request_id, events })
    }

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
        Ok(RuntimeResult::EventInspected {
            request_id,
            inspection: Self::event_inspection(&event_ref, reservation),
        })
    }

    fn change_event_status(
        &mut self,
        request_id: String,
        owner_claim: RuntimeOwnerClaim,
        event_ref: CanonicalEventRef,
        status: EventStatus,
    ) -> Result<RuntimeResult, ProtocolError> {
        event_ref
            .validate()
            .map_err(|_| ProtocolError::ForbiddenEventMethod)?;
        owner_claim.validate()?;
        let reservation_key = (event_ref.event_id.clone(), event_ref.generation);
        {
            {
                let Some(reservation) = self.reservations.get_mut(&reservation_key) else {
                    return Ok(RuntimeResult::Failed {
                        request_id,
                        code: "unknown_reservation".to_owned(),
                    });
                };
                if reservation.owner_claim != owner_claim {
                    return Err(ProtocolError::OwnerMismatch);
                }
                if reservation.status != EventStatus::Pending {
                    return Ok(RuntimeResult::EventLifecycleChanged {
                        request_id,
                        inspection: Self::event_inspection(&event_ref, reservation),
                    });
                }
                reservation.status = status;
            }
        }
        if self.persist_runtime_state().is_err() {
            if let Some(reservation) = self.reservations.get_mut(&reservation_key) {
                reservation.status = EventStatus::Pending;
            }
            return Ok(RuntimeResult::Failed {
                request_id,
                code: "runtime_state_unavailable".to_owned(),
            });
        }
        let reservation = self
            .reservations
            .get(&reservation_key)
            .expect("reservation remains after status update");
        Ok(RuntimeResult::EventLifecycleChanged {
            request_id,
            inspection: Self::event_inspection(&event_ref, reservation),
        })
    }
}

/// Prove this crate does not name a source-port type in its public API.
#[must_use]
pub fn accepts_source_packages() -> bool {
    false
}

/// Drive one async port call to completion from a synchronous service call.
///
/// Callers arrive on three kinds of thread: a dispatcher worker that owns no
/// runtime, a `spawn_blocking` task of an embedding transport, and a test's own
/// runtime thread. Only the first may drive a future in place. The two obvious
/// bridges are wrong for the others by construction: `Runtime::block_on` panics
/// inside any runtime context, and `block_in_place` panics on the
/// current-thread flavor — which is exactly what `#[tokio::test]` and an
/// embedder of [`InvocationDispatcher`] build, so the requirement was a
/// multi-thread runtime nobody declared.
///
/// So when a runtime is already in scope the future moves to a thread that
/// runtime does not own and the caller waits on the join. The future borrows
/// the caller's frame, so the thread is scoped rather than detached.
fn block_on<T: Send>(future: impl Future<Output = T> + Send) -> T {
    fn drive<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime service tokio runtime")
            .block_on(future)
    }
    if tokio::runtime::Handle::try_current().is_err() {
        return drive(future);
    }
    std::thread::scope(|scope| {
        scope
            .spawn(|| drive(future))
            .join()
            .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_kernel::event_api::{CanonicalEventRef, EventApplication, EventOccurrence};
    use apxm_program::air::AirModule;
    use apxm_program::artifact::ExecutableArtifact;
    use apxm_runtime_protocol::{
        GrantRef, PrincipalRef, ProgramInvocationId, RUNTIME_PROTOCOL_VERSION, ReadContext,
        ReadPurpose, RequestId, RuntimeFailureCode, RuntimeHandshakeV2, RuntimeRequest,
        RuntimeRequestV2, RuntimeResultV2, ScopeRef,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct InvocationGate {
        released: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl ApprovalBroker for InvocationGate {
        async fn resolve_ask(&self, _ask_id: &str) -> ApprovalDecision {
            while !self.released.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
            ApprovalDecision::Allow
        }
    }

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
        let raw = fs::read(fixture_dir().join("canonical-execute.air.json")).expect("fixture AIR");
        let air: AirModule = serde_json::from_slice(&raw).expect("fixture AIR JSON");
        ExecutableArtifact::from_air(&air)
            .expect("canonical fixture artifact")
            .encode()
            .expect("canonical fixture artifact JSON")
    }

    fn capability_air_bytes() -> Vec<u8> {
        let raw = fs::read(fixture_dir().join("canonical-skill-execute.air.json"))
            .expect("skill fixture AIR");
        let air: AirModule = serde_json::from_slice(&raw).expect("skill fixture AIR JSON");
        ExecutableArtifact::from_air(&air)
            .expect("canonical skill fixture artifact")
            .encode()
            .expect("canonical skill fixture artifact JSON")
    }

    pub(super) fn event_air_bytes() -> Vec<u8> {
        let mut air: AirModule = serde_json::from_value(serde_json::json!({
            "schema_version":"apxm.air",
            "semantic_operations":[{"node_id":"event.wait","op":"await.event","parent_region_id":"Event.body","execution_order":0,
                "operands":[{"slot":"event_ref","value_id":"Event.param.input","type_ref":"EventRef"}],
                "result":{"value_id":"event.payload","type_ref":"EventOutput"}}],
            "structural_ir":[{"region_id":"Event.body","kind":"function","execution_order":0,
                "block_arguments":[{"value_id":"Event.param.input","type_ref":"Input"}]},
                {"region_id":"event.return","kind":"return","parent_region_id":"Event.body","execution_order":1,
                "operands":[{"slot":"output","value_id":"event.payload","type_ref":"EventOutput"}]}],
            "context_flow":[],"source_map":{"schema_version":"apxm.source-map","source_language":"python","node_spans":[],"region_annotations":[]}
        })).unwrap();
        air.event_requirements = vec![apxm_program::event::EventRequirement::new("event.wait".into(),"UserInput".into(),serde_json::from_value(serde_json::json!({
            "type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false
        })).unwrap()).unwrap()];
        ExecutableArtifact::from_air(&air)
            .unwrap()
            .encode()
            .unwrap()
    }

    fn host_capability_air_bytes() -> Vec<u8> {
        let raw = fs::read(fixture_dir().join("canonical-host-capability-execute.air.json"))
            .expect("host capability fixture AIR");
        let air: AirModule = serde_json::from_slice(&raw).expect("host capability fixture JSON");
        ExecutableArtifact::from_air(&air)
            .expect("canonical host capability fixture artifact")
            .encode()
            .expect("canonical host capability fixture artifact JSON")
    }

    /// A minimal stateful program with an explicit typed yield and return.
    fn yielding_air_bytes() -> Vec<u8> {
        let air: AirModule = serde_json::from_value(serde_json::json!({
            "schema_version":"apxm.air", "semantic_operations":[],
            "structural_ir":[
                {"region_id":"root", "kind":"function", "execution_order":0},
                {"region_id":"yield.next", "kind":"yield", "parent_region_id":"root", "execution_order":0,
                 "operands":[{"slot":"output", "value_id":"reply", "type_ref":"Output"}],
                 "block_arguments":[{"value_id":"next", "type_ref":"Input"}]},
                {"region_id":"return.done", "kind":"return", "parent_region_id":"root", "execution_order":1,
                 "operands":[{"slot":"output", "value_id":"next", "type_ref":"Input"}]}
            ],
            "value_assemblies":[{"value_id":"reply", "expression":{"kind":"string", "value":"ready"}}],
            "context_flow":[],
            "source_map":{"schema_version":"apxm.source-map", "source_language":"python", "node_spans":[], "region_annotations":[]}
        })).unwrap();
        ExecutableArtifact::from_air(&air)
            .unwrap()
            .encode()
            .unwrap()
    }

    fn read_context(purpose: ReadPurpose) -> ReadContext {
        ReadContext {
            request_id: RequestId::new("read.request").expect("request id"),
            scope_ref: ScopeRef::new("scope.local").expect("scope"),
            principal_ref: PrincipalRef::new("principal.local").expect("principal"),
            grant_ref: GrantRef::new("grant.local").expect("grant"),
            correlation_id: None,
            purpose,
        }
    }

    #[test]
    fn runtime_state_configuration_fails_closed_before_opening_storage() {
        assert_eq!(
            runtime_state_dir_from_value(None),
            Err(RuntimeServiceStartupError::MissingRuntimeStateDir)
        );
        assert_eq!(
            runtime_state_dir_from_value(Some(std::ffi::OsString::from(""))),
            Err(RuntimeServiceStartupError::EmptyRuntimeStateDir)
        );
        assert_eq!(
            runtime_state_dir_from_value(Some(std::ffi::OsString::from("relative/state"))),
            Err(RuntimeServiceStartupError::RelativeRuntimeStateDir(
                PathBuf::from("relative/state")
            ))
        );
        assert_eq!(
            runtime_state_dir_from_value(Some(std::ffi::OsString::from("/tmp/../state"))),
            Err(RuntimeServiceStartupError::UnsafeRuntimeStateDir(
                PathBuf::from("/tmp/../state")
            ))
        );
    }

    #[test]
    fn runtime_service_reopens_the_same_explicit_state_directory() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let first = RuntimeService::in_memory().with_runtime_state_dir(path.clone());
        assert!(first.startup_error().is_none());
        drop(first);

        let second = RuntimeService::in_memory().with_runtime_state_dir(path);
        assert!(second.startup_error().is_none());
    }

    #[test]
    fn pending_invocation_reopens_with_the_same_identity_and_input() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let (instance_id, owner_claim, invocation_id) = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance_id = create_started(&mut service, fixture_air_bytes());
            let owner_claim = owner_claim(&service, &instance_id);
            let prepared = service
                .prepare_invocation(
                    "pending.start".to_owned(),
                    instance_id.clone(),
                    owner_claim.clone(),
                    serde_json::json!({"prompt": "same"}),
                )
                .expect("durable pending invocation");
            let invocation_id = prepared.invocation_id.clone();
            service.release_invocation_claim(&invocation_id);
            (instance_id, owner_claim, invocation_id)
        };

        let mut reopened = RuntimeService::default().with_runtime_state_dir(path);
        let recovered = reopened
            .recover_invocations()
            .expect("recover pending work");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].invocation_id, invocation_id);
        assert_eq!(recovered[0].input, serde_json::json!({"prompt": "same"}));
        reopened.release_invocation_claim(&invocation_id);

        let replay = reopened.prepare_invocation(
            "pending.start".to_owned(),
            instance_id,
            owner_claim,
            serde_json::json!({"prompt": "same"}),
        );
        assert!(matches!(
            replay,
            Err(RuntimeResult::ProgramInvocationStarted {
                program_invocation_id,
                ..
            }) if program_invocation_id == invocation_id
        ));
    }

    #[test]
    fn a_pending_next_invocation_reopens_over_the_previous_yield_without_restarting() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_path_buf();
        let (instance_id, claim, invocation_id) = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance = create_started(&mut service, yielding_air_bytes());
            let claim = owner_claim(&service, &instance);
            let first = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationStart {
                        request_id: "yield.first".into(),
                        program_instance_id: instance.clone(),
                        owner_claim: claim.clone(),
                        input: Value::Null,
                    },
                )
                .unwrap();
            let RuntimeResult::ProgramInvocationStarted {
                program_invocation_id,
                ..
            } = first
            else {
                panic!("{first:?}");
            };
            assert_eq!(
                service
                    .execution_backend
                    .invocation_status(&program_invocation_id),
                Some(ProgramInvocationStatus::CommittedYield)
            );
            let next = service
                .prepare_invocation(
                    "yield.next".into(),
                    instance.clone(),
                    claim.clone(),
                    serde_json::json!({"message":"next"}),
                )
                .unwrap();
            let invocation_id = next.invocation_id.clone();
            service.release_invocation_claim(&invocation_id);
            (instance, claim, invocation_id)
        };
        let mut service = RuntimeService::default().with_runtime_state_dir(path);
        let mut recovered = service.recover_invocations().unwrap();
        assert_eq!(
            recovered.len(),
            1,
            "a prior yield does not hide newly pending work"
        );
        let next = recovered.pop().unwrap();
        assert_eq!(next.invocation_id, invocation_id);
        assert_eq!(next.input, serde_json::json!({"message":"next"}));
        assert!(service.begin_invocation(&invocation_id).unwrap());
        let result = service.finish_invocation(&next, next.execute());
        assert!(
            matches!(result, RuntimeResult::ProgramInvocationStarted { .. }),
            "{result:?}"
        );
        assert_eq!(
            service.execution_backend.invocation_status(&invocation_id),
            Some(ProgramInvocationStatus::CommittedReturn)
        );
        assert_eq!(
            service
                .service_invocation_inspection(&invocation_id)
                .expect("completed invocation inspection")
                .status,
            ProgramInvocationStatus::CommittedReturn
        );
        let cancel = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationCancel {
                    request_id: "yield.after-return.cancel".into(),
                    owner_claim: claim.clone(),
                    program_invocation_id: invocation_id.clone(),
                },
            )
            .expect("late cancellation acknowledgement");
        assert!(matches!(cancel, RuntimeResult::Cancelled { .. }));
        assert_eq!(
            service
                .service_invocation_inspection(&invocation_id)
                .expect("completed invocation after late cancellation")
                .status,
            ProgramInvocationStatus::CommittedReturn
        );
        let replay = service.prepare_invocation(
            "yield.next".into(),
            instance_id.clone(),
            claim.clone(),
            serde_json::json!({"message": "next"}),
        );
        assert!(matches!(
            replay,
            Err(RuntimeResult::ProgramInvocationStarted { program_invocation_id, .. })
                if program_invocation_id == invocation_id
        ));
        assert!(
            matches!(service.handle(&handshake(), RuntimeRequest::ProgramInvocationStart {
            request_id:"yield.after-return".into(), program_instance_id:instance_id, owner_claim:claim, input:Value::Null,
        }).unwrap(), RuntimeResult::Failed { code, .. } if code == "program_instance_completed")
        );
    }

    #[test]
    fn a_running_next_invocation_is_not_redispatched_from_the_previous_yield() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_path_buf();
        let (instance, claim, invocation) = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance = create_started(&mut service, yielding_air_bytes());
            let claim = owner_claim(&service, &instance);
            assert!(matches!(
                service
                    .handle(
                        &handshake(),
                        RuntimeRequest::ProgramInvocationStart {
                            request_id: "yield.first".into(),
                            program_instance_id: instance.clone(),
                            owner_claim: claim.clone(),
                            input: Value::Null,
                        }
                    )
                    .unwrap(),
                RuntimeResult::ProgramInvocationStarted { .. }
            ));
            let next = service
                .prepare_invocation(
                    "yield.crossed-send".into(),
                    instance.clone(),
                    claim.clone(),
                    serde_json::json!({"message":"once"}),
                )
                .unwrap();
            assert!(service.begin_invocation(&next.invocation_id).unwrap());
            (instance, claim, next.invocation_id)
        };
        let mut service = RuntimeService::default().with_runtime_state_dir(path);
        assert!(service.recover_invocations().unwrap().is_empty());
        assert!(
            matches!(service.prepare_invocation("yield.crossed-send".into(), instance, claim, serde_json::json!({"message":"once"})),
            Err(RuntimeResult::Failed { code, .. }) if code == "outcome_unknown")
        );
        assert!(!service.active_cancellations.contains_key(&invocation));
    }

    #[test]
    fn running_invocation_is_never_redispatched_after_restart() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let (instance_id, owner_claim, invocation_id) = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance_id = create_started(&mut service, fixture_air_bytes());
            let owner_claim = owner_claim(&service, &instance_id);
            let prepared = service
                .prepare_invocation(
                    "running.start".to_owned(),
                    instance_id.clone(),
                    owner_claim.clone(),
                    serde_json::json!({"prompt": "once"}),
                )
                .expect("durable pending invocation");
            assert!(
                service
                    .begin_invocation(&prepared.invocation_id)
                    .expect("durable running marker")
            );
            (instance_id, owner_claim, prepared.invocation_id)
        };

        let mut reopened = RuntimeService::default().with_runtime_state_dir(path);
        assert!(reopened.invocation_index.contains_key(&invocation_id));
        assert!(
            reopened
                .recover_invocations()
                .expect("reconcile running work")
                .is_empty()
        );
        let replay = reopened.prepare_invocation(
            "running.start".to_owned(),
            instance_id,
            owner_claim,
            serde_json::json!({"prompt": "once"}),
        );
        assert!(matches!(
            replay,
            Err(RuntimeResult::Failed { code, .. }) if code == "outcome_unknown"
        ));
        assert_eq!(
            reopened
                .service_invocation_inspection(&invocation_id)
                .expect("unmarked uncertain inspection")
                .status,
            ProgramInvocationStatus::OutcomeUnknown
        );
    }

    #[test]
    fn acknowledged_cancellation_keeps_its_marker_and_reports_uncertainty_after_running_crash() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let (instance_id, owner_claim, invocation_id) = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance_id = create_started(&mut service, fixture_air_bytes());
            let owner_claim = owner_claim(&service, &instance_id);
            let prepared = service
                .prepare_invocation(
                    "cancel.crossed-start".to_owned(),
                    instance_id.clone(),
                    owner_claim.clone(),
                    serde_json::json!({"prompt": "once"}),
                )
                .expect("durable pending invocation");
            assert!(
                service
                    .begin_invocation(&prepared.invocation_id)
                    .expect("running marker")
            );
            let status = service
                .service_invocation_inspection(&prepared.invocation_id)
                .expect("inspection")
                .status;
            assert_eq!(status, ProgramInvocationStatus::Running);
            let result = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationCancel {
                        request_id: "cancel.crossed-start.request".to_owned(),
                        owner_claim: owner_claim.clone(),
                        program_invocation_id: prepared.invocation_id.clone(),
                    },
                )
                .expect("cancellation request");
            assert!(matches!(result, RuntimeResult::Cancelled { .. }));
            assert_eq!(
                service
                    .service_invocation_inspection(&prepared.invocation_id)
                    .expect("marked inspection")
                    .status,
                ProgramInvocationStatus::Cancelling
            );
            (instance_id, owner_claim, prepared.invocation_id)
        };

        let mut reopened = RuntimeService::default().with_runtime_state_dir(path);
        assert!(
            reopened.cancelled.contains_key(&invocation_id),
            "ACKed marker must survive restart"
        );
        assert!(
            reopened
                .recover_invocations()
                .expect("reconcile running work")
                .is_empty()
        );
        assert_eq!(
            reopened
                .service_invocation_inspection(&invocation_id)
                .expect("recovered inspection")
                .status,
            ProgramInvocationStatus::CancellationUnconfirmed
        );
        let replay = reopened.prepare_invocation(
            "cancel.crossed-start".to_owned(),
            instance_id,
            owner_claim,
            serde_json::json!({"prompt": "once"}),
        );
        assert!(matches!(
            replay,
            Err(RuntimeResult::Failed { code, .. }) if code == "outcome_unknown"
        ));
    }

    #[test]
    fn cancellation_marker_closes_a_host_wait_committed_after_the_cancel_scan() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let mut service =
            RuntimeService::default().with_runtime_state_dir(directory.path().to_path_buf());
        let instance_id = create_started(&mut service, host_capability_air_bytes());
        let claim = owner_claim(&service, &instance_id);
        let prepared = service
            .prepare_invocation(
                "host.cancel.after-scan".to_owned(),
                instance_id.clone(),
                claim,
                Value::Null,
            )
            .expect("durable invocation");
        assert!(
            service
                .begin_invocation(&prepared.invocation_id)
                .expect("running marker")
        );
        let execution = prepared.execute();
        assert!(
            execution
                .as_ref()
                .is_ok_and(|value| service.execution_is_waiting(&prepared.invocation_id, value))
        );
        let marker_bytes = u64::try_from(prepared.invocation_id.len()).unwrap_or(u64::MAX);
        service.cancelled.insert(
            prepared.invocation_id.clone(),
            RuntimeService::entry(marker_bytes, service.state_policy.cancellations.ttl),
        );
        service.cancellation_bytes = service.cancellation_bytes.saturating_add(marker_bytes);
        service
            .persist_runtime_state()
            .expect("durable cancellation marker");

        service.finish_invocation(&prepared, execution);
        assert_eq!(
            service
                .service_invocation_inspection(&prepared.invocation_id)
                .expect("settled inspection")
                .status,
            ProgramInvocationStatus::Cancelled
        );
        let observations = service.observation_sink.snapshot();
        assert_eq!(
            observations
                .iter()
                .filter(|item| item.observation_kind
                    == apxm_runtime_protocol::ObservationKind::CapabilitySettled)
                .count(),
            1,
            "the newly parked host request is withdrawn once"
        );
        assert!(observations.iter().any(|item| {
            item.observation_kind == apxm_runtime_protocol::ObservationKind::InvocationCancelled
        }));
    }

    #[test]
    fn dispatcher_refusal_is_durable_and_replayed() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
        let instance_id = create_started(&mut service, fixture_air_bytes());
        let owner_claim = owner_claim(&service, &instance_id);
        let prepared = service
            .prepare_invocation(
                "capacity.start".to_owned(),
                instance_id.clone(),
                owner_claim.clone(),
                Value::Null,
            )
            .expect("durable pending invocation");
        let invocation_id = prepared.invocation_id.clone();
        service.release_invocation_claim(&invocation_id);
        assert!(matches!(
            service.fail_pending_invocation(
                &invocation_id,
                "invocation_capacity_exhausted"
            ),
            RuntimeResult::Failed { ref code, .. }
                if code == "invocation_capacity_exhausted"
        ));
        drop(prepared);
        drop(service);

        let mut reopened = RuntimeService::default().with_runtime_state_dir(path);
        assert!(
            reopened.startup_error().is_none(),
            "{:?}",
            reopened.startup_error()
        );
        assert!(reopened.instances.contains_key(&instance_id));
        assert!(reopened.recover_invocations().unwrap().is_empty());
        let replay = reopened.prepare_invocation(
            "capacity.start".to_owned(),
            instance_id,
            owner_claim,
            Value::Null,
        );
        match replay {
            Err(RuntimeResult::Failed { code, .. }) => {
                assert_eq!(code, "invocation_capacity_exhausted")
            }
            Err(other) => panic!("unexpected replay result: {other:?}"),
            Ok(_) => panic!("dispatcher refusal replay prepared new work"),
        }
    }

    #[test]
    fn live_host_requests_wait_for_durable_settlement_boundary() {
        let mut service = RuntimeService::default();
        let instance_id = create_started(&mut service, host_capability_air_bytes());
        let claim = owner_claim(&service, &instance_id);
        let started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "host.visibility.start".to_owned(),
                    program_instance_id: instance_id,
                    owner_claim: claim.clone(),
                    input: Value::Null,
                },
            )
            .expect("host invocation parks");
        assert!(matches!(
            started,
            RuntimeResult::ProgramInvocationStarted { .. }
        ));
        let observations = service.observation_sink.snapshot();
        let request = observations
            .iter()
            .find(|observation| {
                observation.observation_kind
                    == apxm_runtime_protocol::ObservationKind::CapabilityRequested
            })
            .expect("host request observation")
            .clone();
        let request_id = request
            .host_capability
            .as_ref()
            .unwrap()
            .capability_request_id
            .clone();
        let mut later = request.clone();
        later.observation_kind = apxm_runtime_protocol::ObservationKind::CapabilitySettled;
        later.sequence += 1;
        let mut unrelated = later.clone();
        unrelated.program_invocation_id =
            apxm_runtime_protocol::ProgramInvocationId::new("other.invocation")
                .expect("valid invocation id");
        let mut live = observations.clone();
        live.extend([later, unrelated.clone()]);

        // The same recorder snapshot can be published while its commit is
        // still in flight. No request or later cursor is readable then.
        let uncommitted = RuntimeService::default();
        let readable = uncommitted.readable_live_observations(live);
        assert!(
            readable
                .iter()
                .all(|observation| observation.program_invocation_id
                    != request.program_invocation_id
                    || observation.sequence < request.sequence)
        );
        assert!(readable.iter().any(
            |observation| observation.program_invocation_id == unrelated.program_invocation_id
        ));

        assert_eq!(
            service.readable_live_observations(observations.clone()),
            observations
        );
        assert!(matches!(
            service.settle_host_capability(
                "host.visibility.settle".to_owned(),
                claim,
                request_id,
                HostCapabilityOutcomeKind::Ok,
                Some("{\"matches\":1}".to_owned()),
                Some("receipt.host.visibility".to_owned()),
                None,
                true,
            ),
            RuntimeResult::CapabilitySettled { .. }
        ));
        assert_eq!(
            service.readable_live_observations(observations.clone()),
            observations
        );
    }

    #[test]
    fn pending_host_settlement_reopens_as_one_resume() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let (capability_request_id, invocation_id) = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance_id = create_started(&mut service, host_capability_air_bytes());
            let owner_claim = owner_claim(&service, &instance_id);
            let started = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationStart {
                        request_id: "host.pending.start".to_owned(),
                        program_instance_id: instance_id.clone(),
                        owner_claim: owner_claim.clone(),
                        input: Value::Null,
                    },
                )
                .expect("host invocation starts");
            let RuntimeResult::ProgramInvocationStarted {
                program_invocation_id,
                ..
            } = started
            else {
                panic!("host invocation did not park: {started:?}");
            };
            let continuation = service
                .execution_backend
                .load_continuation(&ProgramInstanceRef::new(instance_id.clone()))
                .expect("host continuation is durable");
            let old_wire = continuation.payload.clone();
            assert!(
                old_wire["event_ref"]
                    .as_str()
                    .is_some_and(is_host_capability_request_id),
                "host continuations retain their pre-Event object codec string representation"
            );
            let continuation: Continuation =
                serde_json::from_value(continuation.payload).expect("host continuation");
            assert_eq!(serde_json::to_value(&continuation).unwrap(), old_wire);
            let capability_request_id = continuation
                .event_ref
                .expect("host request")
                .as_str()
                .to_owned();
            assert!(matches!(
                service.settle_host_capability(
                    "host.pending.fulfill".to_owned(),
                    owner_claim.clone(),
                    capability_request_id.clone(),
                    HostCapabilityOutcomeKind::Ok,
                    Some("{\"matches\":1}".to_owned()),
                    Some("receipt.host.pending".to_owned()),
                    None,
                    false,
                ),
                RuntimeResult::CapabilitySettled { .. }
            ));
            let first_claim = service
                .claim_host_capability_resume(&capability_request_id)
                .expect("first dispatcher claim")
                .expect("pending host resume");
            service.release_invocation_claim(&first_claim.invocation_id);
            assert!(matches!(
                service.settle_host_capability(
                    "host.pending.fulfill.retry".to_owned(),
                    owner_claim,
                    capability_request_id.clone(),
                    HostCapabilityOutcomeKind::Ok,
                    Some("{\"matches\":1}".to_owned()),
                    Some("receipt.host.pending".to_owned()),
                    None,
                    false,
                ),
                RuntimeResult::CapabilitySettled { .. }
            ));
            let retry_claim = service
                .claim_host_capability_resume(&capability_request_id)
                .expect("retry dispatcher claim")
                .expect("the same durable settlement remains schedulable");
            assert_eq!(retry_claim.invocation_id, first_claim.invocation_id);
            service.release_invocation_claim(&retry_claim.invocation_id);
            (capability_request_id, program_invocation_id)
        };

        let mut reopened = RuntimeService::default().with_runtime_state_dir(path);
        assert!(
            reopened.startup_error().is_none(),
            "{:?}",
            reopened.startup_error()
        );
        assert_eq!(
            reopened.parked_host_capability_instance(&capability_request_id),
            reopened.invocation_index.get(&invocation_id).cloned()
        );
        assert!(
            !reopened
                .host_capability_settlements
                .get(&capability_request_id)
                .expect("durable settlement")
                .resume_started
        );
        assert!(reopened.recover_invocations().unwrap().is_empty());
        let recovered = reopened
            .recover_continuation_resumes()
            .expect("recover pending host resume");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].event_ref.as_str(), capability_request_id);
        assert_eq!(recovered[0].invocation_id, invocation_id);
        assert!(
            reopened
                .claim_host_capability_resume(&capability_request_id)
                .expect("duplicate claim check")
                .is_none(),
            "the recovered resume owns the only process-local claim"
        );
    }

    #[test]
    fn cancelled_parked_invocation_repairs_the_marker_window_after_restart() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let (invocation_id, capability_request_id, owner_claim) = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance_id = create_started(&mut service, host_capability_air_bytes());
            let owner_claim = owner_claim(&service, &instance_id);
            let started = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationStart {
                        request_id: "host.cancel.marker-window".to_owned(),
                        program_instance_id: instance_id.clone(),
                        owner_claim: owner_claim.clone(),
                        input: Value::Null,
                    },
                )
                .expect("host invocation starts");
            let RuntimeResult::ProgramInvocationStarted {
                program_invocation_id,
                ..
            } = started
            else {
                panic!("host invocation did not park: {started:?}");
            };
            let continuation = service
                .execution_backend
                .load_continuation(&ProgramInstanceRef::new(instance_id))
                .expect("host continuation is durable");
            let continuation: Continuation =
                serde_json::from_value(continuation.payload).expect("host continuation");
            let capability_request_id = continuation
                .event_ref
                .expect("host request")
                .as_str()
                .to_owned();
            let marker_bytes = u64::try_from(program_invocation_id.len()).unwrap_or(u64::MAX);
            service.cancelled.insert(
                program_invocation_id.clone(),
                RuntimeService::entry(marker_bytes, service.state_policy.cancellations.ttl),
            );
            service.cancellation_bytes = service.cancellation_bytes.saturating_add(marker_bytes);
            service
                .persist_runtime_state()
                .expect("durable cancellation marker");
            assert!(service.host_capability_settlements.is_empty());
            (program_invocation_id, capability_request_id, owner_claim)
        };

        let mut reopened = RuntimeService::default().with_runtime_state_dir(path);
        reopened
            .reconcile_recovery_state()
            .expect("repair cancelled parked invocation");
        assert_eq!(
            reopened.execution_backend.invocation_status(&invocation_id),
            Some(ProgramInvocationStatus::Cancelled)
        );
        assert!(matches!(
            reopened
                .instances
                .values()
                .find_map(|instance| instance.invocation.as_ref())
                .and_then(|invocation| invocation.result.as_ref()),
            Some(RuntimeResult::Cancelled { .. })
        ));
        assert_eq!(
            reopened
                .host_capability_settlements
                .get(&capability_request_id)
                .map(|state| state.settlement.outcome),
            Some(HostCapabilityOutcomeKind::Cancelled)
        );
        assert!(matches!(
            reopened.handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationCancel {
                    request_id: "host.cancel.marker-window.replay".to_owned(),
                    owner_claim,
                    program_invocation_id: invocation_id,
                },
            ),
            Ok(RuntimeResult::Cancelled { .. })
        ));
    }

    #[test]
    fn explicit_cancel_wins_before_a_durable_ok_resume_starts() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let (invocation_id, capability_request_id, owner_claim) = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance_id = create_started(&mut service, host_capability_air_bytes());
            let owner_claim = owner_claim(&service, &instance_id);
            let started = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationStart {
                        request_id: "host.cancel.ok-race".to_owned(),
                        program_instance_id: instance_id.clone(),
                        owner_claim: owner_claim.clone(),
                        input: Value::Null,
                    },
                )
                .expect("host invocation starts");
            let RuntimeResult::ProgramInvocationStarted {
                program_invocation_id,
                ..
            } = started
            else {
                panic!("host invocation did not park: {started:?}");
            };
            let continuation = service
                .execution_backend
                .load_continuation(&ProgramInstanceRef::new(instance_id))
                .expect("host continuation is durable");
            let continuation: Continuation =
                serde_json::from_value(continuation.payload).expect("host continuation");
            let capability_request_id = continuation
                .event_ref
                .expect("host request")
                .as_str()
                .to_owned();
            assert!(matches!(
                service.settle_host_capability(
                    "host.cancel.ok-race.settle".to_owned(),
                    owner_claim.clone(),
                    capability_request_id.clone(),
                    HostCapabilityOutcomeKind::Ok,
                    Some("{}".to_owned()),
                    Some("receipt.host.cancel.ok-race".to_owned()),
                    None,
                    false,
                ),
                RuntimeResult::CapabilitySettled { .. }
            ));
            let marker_bytes = u64::try_from(program_invocation_id.len()).unwrap_or(u64::MAX);
            service.cancelled.insert(
                program_invocation_id.clone(),
                RuntimeService::entry(marker_bytes, service.state_policy.cancellations.ttl),
            );
            service.cancellation_bytes = service.cancellation_bytes.saturating_add(marker_bytes);
            service
                .persist_runtime_state()
                .expect("durable cancellation marker");
            (program_invocation_id, capability_request_id, owner_claim)
        };

        let mut reopened = RuntimeService::default().with_runtime_state_dir(path);
        reopened
            .reconcile_recovery_state()
            .expect("repair cancellation around pending host settlement");
        assert_eq!(
            reopened.execution_backend.invocation_status(&invocation_id),
            Some(ProgramInvocationStatus::Cancelled)
        );
        assert_eq!(
            reopened
                .host_capability_settlements
                .get(&capability_request_id)
                .map(|state| state.settlement.outcome),
            Some(HostCapabilityOutcomeKind::Ok)
        );
        assert!(matches!(
            reopened.handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationCancel {
                    request_id: "host.cancel.ok-race.replay".to_owned(),
                    owner_claim,
                    program_invocation_id: invocation_id,
                },
            ),
            Ok(RuntimeResult::Cancelled { .. })
        ));
    }

    #[test]
    fn started_host_resume_becomes_unknown_after_restart_without_resend() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let (capability_request_id, invocation_id) = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance_id = create_started(&mut service, host_capability_air_bytes());
            let owner_claim = owner_claim(&service, &instance_id);
            let started = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationStart {
                        request_id: "host.running.start".to_owned(),
                        program_instance_id: instance_id.clone(),
                        owner_claim: owner_claim.clone(),
                        input: Value::Null,
                    },
                )
                .expect("host invocation starts");
            let RuntimeResult::ProgramInvocationStarted {
                program_invocation_id,
                ..
            } = started
            else {
                panic!("host invocation did not park: {started:?}");
            };
            let continuation = service
                .execution_backend
                .load_continuation(&ProgramInstanceRef::new(instance_id))
                .expect("host continuation is durable");
            let continuation: Continuation =
                serde_json::from_value(continuation.payload).expect("host continuation");
            let capability_request_id = continuation
                .event_ref
                .expect("host request")
                .as_str()
                .to_owned();
            assert!(matches!(
                service.settle_host_capability(
                    "host.running.fulfill".to_owned(),
                    owner_claim.clone(),
                    capability_request_id.clone(),
                    HostCapabilityOutcomeKind::Ok,
                    Some("{\"matches\":1}".to_owned()),
                    Some("receipt.host.running".to_owned()),
                    None,
                    false,
                ),
                RuntimeResult::CapabilitySettled { .. }
            ));
            let prepared = service
                .claim_host_capability_resume(&capability_request_id)
                .expect("claim host resume")
                .expect("pending host resume");
            assert!(
                service
                    .begin_continuation_resume(&prepared)
                    .expect("durable resume marker")
            );
            let replay = service.settle_host_capability(
                "host.running.replay".to_owned(),
                owner_claim,
                capability_request_id.clone(),
                HostCapabilityOutcomeKind::Ok,
                Some("{\"matches\":1}".to_owned()),
                Some("receipt.host.running".to_owned()),
                None,
                false,
            );
            assert!(matches!(replay, RuntimeResult::CapabilitySettled { .. }));
            let marker_bytes = u64::try_from(program_invocation_id.len()).unwrap_or(u64::MAX);
            service.cancelled.insert(
                program_invocation_id.clone(),
                RuntimeService::entry(marker_bytes, service.state_policy.cancellations.ttl),
            );
            service.cancellation_bytes = service.cancellation_bytes.saturating_add(marker_bytes);
            service
                .persist_runtime_state()
                .expect("durable cancellation marker");
            assert!(
                service
                    .claim_host_capability_resume(&capability_request_id)
                    .unwrap()
                    .is_none(),
                "an identical retry must not enqueue a second resume"
            );
            (capability_request_id, program_invocation_id)
        };

        let mut reopened = RuntimeService::default().with_runtime_state_dir(path);
        reopened
            .reconcile_recovery_state()
            .expect("reconcile started host resume");
        assert!(reopened.recover_invocations().unwrap().is_empty());
        assert!(
            reopened
                .recover_continuation_resumes()
                .expect("reconcile started host resume")
                .is_empty()
        );
        assert!(
            reopened
                .claim_host_capability_resume(&capability_request_id)
                .unwrap()
                .is_none(),
            "a possible host resume send is never retried after restart"
        );
        let inspection = reopened
            .service_invocation_inspection(&invocation_id)
            .expect("service inspection");
        assert_eq!(
            inspection.status,
            apxm_runtime_protocol::ProgramInvocationStatus::CancellationUnconfirmed
        );
    }

    fn claimed_host_resume_for_finish_test(service: &mut RuntimeService) -> PreparedResume {
        let instance_id = create_started(service, host_capability_air_bytes());
        let owner_claim = owner_claim(service, &instance_id);
        let started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "host.resume.error".to_owned(),
                    program_instance_id: instance_id.clone(),
                    owner_claim: owner_claim.clone(),
                    input: Value::Null,
                },
            )
            .expect("host invocation starts");
        let RuntimeResult::ProgramInvocationStarted { .. } = started else {
            panic!("host invocation did not park: {started:?}");
        };
        let continuation = service
            .execution_backend
            .load_continuation(&ProgramInstanceRef::new(instance_id))
            .expect("host continuation is durable");
        let continuation: Continuation =
            serde_json::from_value(continuation.payload).expect("host continuation");
        let capability_request_id = continuation
            .event_ref
            .expect("host request")
            .as_str()
            .to_owned();
        assert!(matches!(
            service.settle_host_capability(
                "host.resume.error.settle".to_owned(),
                owner_claim,
                capability_request_id.clone(),
                HostCapabilityOutcomeKind::Ok,
                Some("{}".to_owned()),
                Some("receipt.host.resume.error".to_owned()),
                None,
                false,
            ),
            RuntimeResult::CapabilitySettled { .. }
        ));
        let prepared = service
            .claim_host_capability_resume(&capability_request_id)
            .expect("claim host resume")
            .expect("pending host resume");
        assert!(
            service
                .begin_continuation_resume(&prepared)
                .expect("resume marker")
        );
        prepared
    }

    #[test]
    fn unmarked_host_resume_error_remains_unknown() {
        let mut service = RuntimeService::default();
        let prepared = claimed_host_resume_for_finish_test(&mut service);
        let result = service.finish_continuation_resume(&prepared, Err("resume failed".to_owned()));
        assert!(matches!(
            result,
            RuntimeResult::Failed { ref code, .. } if code == "outcome_unknown"
        ));
    }

    #[test]
    fn runtime_service_rehydrates_claims_invocations_reads_cancellation_and_events() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let (instance_id, claim, invocation_id, event_ref, event_claim, event_application) = {
            let mut service = RuntimeService::default()
                .with_runtime_state_dir(path.clone())
                .with_embedded_read_access();
            let instance_id = create_started(&mut service, capability_air_bytes());
            let claim = owner_claim(&service, &instance_id);
            let started = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationStart {
                        request_id: "restart.start".to_owned(),
                        program_instance_id: instance_id.clone(),
                        owner_claim: claim.clone(),
                        input: serde_json::json!({}),
                    },
                )
                .expect("start request");
            let invocation_id = match started {
                RuntimeResult::ProgramInvocationStarted {
                    program_invocation_id,
                    ..
                } => program_invocation_id,
                other => panic!("expected committed invocation, got {other:?}"),
            };
            let event_instance = create_started(&mut service, event_air_bytes());
            let reserved = service
                .handle(
                    &handshake(),
                    RuntimeRequest::EventReserve {
                        request_id: "restart.reserve".to_owned(),
                        program_instance_id: event_instance.clone(),
                        owner_claim: owner_claim(&service, &event_instance),
                        type_id: "UserInput".to_owned(),
                    },
                )
                .expect("reserve request");
            let (event_ref, event_claim) = match reserved {
                RuntimeResult::EventReserved {
                    event_ref,
                    owner_claim,
                    ..
                } => (event_ref, owner_claim),
                other => panic!("expected event reservation, got {other:?}"),
            };
            let event_application = EventApplication {
                event_ref: event_ref.clone(),
                occurrence: EventOccurrence {
                    occurrence_id: "restart.occurrence".to_owned(),
                    source_kind: "human.terminal".to_owned(),
                    mapping_digest: "restart.mapping".to_owned(),
                    source_record: "restart.source".to_owned(),
                    payload: serde_json::json!({"answer": "ok"}),
                },
                idempotency_key: "restart.application".to_owned(),
            };
            service
                .handle(
                    &handshake(),
                    RuntimeRequest::EventFulfill {
                        request_id: "restart.fulfill".to_owned(),
                        owner_claim: event_claim.clone(),
                        application: event_application.clone(),
                    },
                )
                .expect("fulfill request");
            (
                instance_id,
                claim,
                invocation_id,
                event_ref,
                event_claim,
                event_application,
            )
        };

        let mut service = RuntimeService::default()
            .with_runtime_state_dir(path)
            .with_embedded_read_access();
        let replay = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "restart.start".to_owned(),
                    program_instance_id: instance_id.clone(),
                    owner_claim: claim.clone(),
                    input: serde_json::json!({}),
                },
            )
            .expect("idempotent start after restart");
        assert!(matches!(
            replay,
            RuntimeResult::ProgramInvocationStarted { ref program_invocation_id, .. }
                if program_invocation_id == &invocation_id
        ));

        let inspection = service
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::ProgramInvocationInspect {
                    context: read_context(ReadPurpose::Inspection),
                    program_invocation_id: ProgramInvocationId::new(invocation_id.clone())
                        .expect("invocation ref"),
                    node_execution_id: None,
                },
            )
            .expect("inspection after restart");
        assert!(matches!(
            inspection,
            RuntimeResultV2::ProgramInvocationInspection { .. }
        ));
        let observations = service
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::ObservationSubscribe {
                    context: read_context(ReadPurpose::Observation),
                    program_invocation_id: ProgramInvocationId::new(invocation_id.clone())
                        .expect("invocation ref"),
                    after_cursor: None,
                    limit: 100,
                },
            )
            .expect("observations after restart");
        assert!(matches!(
            observations,
            RuntimeResultV2::ObservationPage { .. }
        ));

        let cancelled = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationCancel {
                    request_id: "restart.cancel".to_owned(),
                    owner_claim: claim,
                    program_invocation_id: invocation_id,
                },
            )
            .expect("cancel after restart");
        assert!(matches!(cancelled, RuntimeResult::Cancelled { .. }));

        let inspected = service
            .handle(
                &handshake(),
                RuntimeRequest::EventInspect {
                    request_id: "restart.inspect-event".to_owned(),
                    owner_claim: event_claim.clone(),
                    event_ref: event_ref.clone(),
                },
            )
            .expect("event inspect after restart");
        assert!(matches!(
            inspected,
            RuntimeResult::EventInspected { inspection, .. }
                if inspection.status == EventStatus::Fulfilled
        ));
        let replayed_event = service
            .handle(
                &handshake(),
                RuntimeRequest::EventFulfill {
                    request_id: "restart.fulfill-replay".to_owned(),
                    owner_claim: event_claim,
                    application: event_application,
                },
            )
            .expect("event application replay after restart");
        assert!(matches!(
            replayed_event,
            RuntimeResult::EventApplied {
                result: EventApplicationResult::Fulfilled,
                ..
            }
        ));
    }

    #[test]
    fn v2_reads_are_typed_and_fail_closed_for_unknown_invocations() {
        let service = RuntimeService::default();
        let result = service
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::ProgramInvocationInspect {
                    context: read_context(ReadPurpose::Inspection),
                    program_invocation_id: ProgramInvocationId::new("invocation.missing")
                        .expect("invocation"),
                    node_execution_id: None,
                },
            )
            .expect("protocol admission");
        assert!(matches!(
            result,
            RuntimeResultV2::Failed {
                code: RuntimeFailureCode::Unauthorized,
                ..
            }
        ));
    }

    #[test]
    fn v2_enforces_client_server_feature_intersection_per_request() {
        let service = RuntimeService::default();
        let mut client = RuntimeHandshakeV2::server();
        client
            .supported_features
            .retain(|feature| *feature == apxm_runtime_protocol::RuntimeFeature::ContentRead);
        let result = service
            .handle_v2(
                &client,
                RuntimeRequestV2::ProgramInvocationInspect {
                    context: read_context(ReadPurpose::Inspection),
                    program_invocation_id: ProgramInvocationId::new("invocation.missing")
                        .expect("invocation"),
                    node_execution_id: None,
                },
            )
            .expect("protocol admission remains valid");
        assert!(matches!(
            result,
            RuntimeResultV2::Failed {
                code: RuntimeFailureCode::UnsupportedFeature,
                ..
            }
        ));
    }

    #[test]
    fn invocation_result_preserves_commit_uncertainty() {
        let compare = invocation_result_from_execution_output(
            "request.compare".to_owned(),
            "invocation.compare".to_owned(),
            &serde_json::json!({
                "commit": {"status": "compare_conflict"}
            }),
        );
        assert!(matches!(
            compare,
            RuntimeResult::Failed { ref code, .. } if code == "compare_conflict"
        ));
        let unknown = invocation_result_from_execution_output(
            "request.unknown".to_owned(),
            "invocation.unknown".to_owned(),
            &serde_json::json!({
                "commit": {"status": "outcome_unknown"}
            }),
        );
        assert!(matches!(
            unknown,
            RuntimeResult::Failed { ref code, .. } if code == "outcome_unknown"
        ));
    }

    #[test]
    fn authorized_v2_inspection_reads_the_committed_execution_record() {
        let mut service = RuntimeService::default()
            .with_read_access_hook(std::sync::Arc::new(apxm_commit_local::AllowReadAccess));
        let instance = create_started(&mut service, capability_air_bytes());
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
            .expect("start request");
        let RuntimeResult::ProgramInvocationStarted {
            program_invocation_id,
            ..
        } = started
        else {
            panic!("expected committed invocation: {started:?}");
        };
        let result = service
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::ProgramInvocationInspect {
                    context: read_context(ReadPurpose::Inspection),
                    program_invocation_id: ProgramInvocationId::new(program_invocation_id)
                        .expect("invocation ref"),
                    node_execution_id: None,
                },
            )
            .expect("inspection request");
        let RuntimeResultV2::ProgramInvocationInspection { inspection, .. } = result else {
            panic!("expected invocation inspection");
        };
        assert_eq!(
            inspection.status,
            apxm_runtime_protocol::ProgramInvocationStatus::Failed
        );
        let invocation_ref = inspection.program_invocation_id.clone();
        let node_execution_id = inspection
            .node_execution_refs
            .first()
            .expect("committed node execution ref")
            .clone();
        let node = service
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::ProgramInvocationInspect {
                    context: read_context(ReadPurpose::Inspection),
                    program_invocation_id: invocation_ref.clone(),
                    node_execution_id: Some(node_execution_id.clone()),
                },
            )
            .expect("node inspection request");
        assert!(matches!(
            node,
            RuntimeResultV2::NodeExecutionInspection { inspection, .. }
                if inspection.node_execution_id == node_execution_id
                    && inspection.program_invocation_id == invocation_ref
        ));
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
        let release = fs::read(fixture_dir().join("canonical-execute.release.json")).unwrap();
        let provenance = fs::read(fixture_dir().join("canonical-execute.provenance.json")).unwrap();
        service
            .bind_admission(
                &program_instance_id,
                materials_for_artifact(&fixture_air_bytes(), "ignored", release, provenance),
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
        match started {
            RuntimeResult::ProgramInvocationStarted { .. } => {}
            RuntimeResult::Failed { ref code, .. } if code == "outcome_unknown" => {}
            other => panic!("unexpected invocation transition: {other:?}"),
        }
    }

    #[test]
    fn non_returned_invocation_allows_next_and_replays_immutable_history() {
        let mut service = RuntimeService::default();
        let bytes = capability_air_bytes();
        let instance = create_started(&mut service, bytes);
        let claim = owner_claim(&service, &instance);
        let first = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "history.first".to_owned(),
                    program_instance_id: instance.clone(),
                    owner_claim: claim.clone(),
                    input: serde_json::json!({"turn": 1}),
                },
            )
            .expect("first invocation");
        let first_id = match first {
            RuntimeResult::ProgramInvocationStarted {
                program_invocation_id,
                ..
            } => program_invocation_id,
            other => panic!("first invocation failed: {other:?}"),
        };
        let second = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "history.second".to_owned(),
                    program_instance_id: instance.clone(),
                    owner_claim: claim.clone(),
                    input: serde_json::json!({"turn": 2}),
                },
            )
            .expect("second invocation");
        assert!(matches!(
            second,
            RuntimeResult::ProgramInvocationStarted { .. }
        ));
        let replay = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "history.first".to_owned(),
                    program_instance_id: instance,
                    owner_claim: claim,
                    input: serde_json::json!({"turn": 1}),
                },
            )
            .expect("first retry");
        assert!(matches!(
            replay,
            RuntimeResult::ProgramInvocationStarted {
                program_invocation_id,
                ..
            } if program_invocation_id == first_id
        ));
    }

    #[test]
    fn event_fulfill_resumes_the_committed_continuation() {
        let mut service = RuntimeService::default().with_embedded_read_access();
        let instance = create_started(&mut service, event_air_bytes());
        let claim = owner_claim(&service, &instance);
        let reserved = service
            .reserve_event(
                "resume.reserve".into(),
                instance.clone(),
                claim.clone(),
                "UserInput".into(),
            )
            .unwrap();
        let RuntimeResult::EventReserved {
            event_ref,
            owner_claim: event_claim,
            ..
        } = reserved
        else {
            panic!("reserve: {reserved:?}")
        };
        let input = serde_json::to_value(&event_ref).unwrap();
        let started = service.start_invocation(
            "resume.start".into(),
            instance.clone(),
            claim.clone(),
            input.clone(),
        );
        assert!(
            matches!(started, RuntimeResult::ProgramInvocationStarted { .. }),
            "{started:?}"
        );
        let fulfilled = service
            .fulfill_event(
                "resume.fulfill".into(),
                event_claim,
                EventApplication {
                    event_ref: event_ref.clone(),
                    occurrence: EventOccurrence {
                        occurrence_id: "resume.occurrence".into(),
                        source_kind: "test.source".into(),
                        mapping_digest: "mapping.1".into(),
                        source_record: "record.1".into(),
                        payload: serde_json::json!({"answer":"ok"}),
                    },
                    idempotency_key: "resume.application".into(),
                },
            )
            .unwrap();
        assert!(matches!(
            fulfilled,
            RuntimeResult::EventApplied {
                result: EventApplicationResult::Fulfilled,
                ..
            }
        ));
        assert!(
            service.instances[&instance]
                .invocation
                .as_ref()
                .unwrap()
                .result
                .is_none(),
            "acceptance does not execute inline"
        );
        let prepared = service
            .claim_next_continuation_resume()
            .unwrap()
            .expect("durable wake");
        assert_eq!(prepared.event_ref.reservation(), Some(&event_ref));
        assert!(service.begin_continuation_resume(&prepared).unwrap());
        let execution = prepared.execute();
        assert!(execution.is_ok(), "{execution:?}");
        let completed = service.finish_continuation_resume(&prepared, execution);
        assert!(
            matches!(completed, RuntimeResult::ProgramInvocationStarted { .. }),
            "{completed:?}"
        );
        assert!(
            service.instances[&instance]
                .invocation
                .as_ref()
                .unwrap()
                .result
                .is_some()
        );
        assert!(service.claim_next_continuation_resume().unwrap().is_none());
        assert_eq!(
            service.start_invocation("resume.start".into(), instance, claim, input),
            completed
        );
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
    fn protocol_v1_rejects_empty_mutation_request_ids() {
        let owner_claim = RuntimeOwnerClaim::mint();
        let application = EventApplication {
            event_ref: CanonicalEventRef {
                event_id: "evt.1".to_owned(),
                generation: 1,
            },
            occurrence: EventOccurrence {
                occurrence_id: "occurrence.1".to_owned(),
                source_kind: "test.source".to_owned(),
                mapping_digest: "mapping.1".to_owned(),
                source_record: "record.1".to_owned(),
                payload: serde_json::json!(null),
            },
            idempotency_key: "idempotency.1".to_owned(),
        };
        let requests = vec![
            RuntimeRequest::ProgramInstanceCreate {
                request_id: String::new(),
                artifact_digest: format!("sha256:{}", "0".repeat(64)),
            },
            RuntimeRequest::ProgramInvocationStart {
                request_id: String::new(),
                program_instance_id: "pi.1".to_owned(),
                owner_claim: owner_claim.clone(),
                input: serde_json::json!(null),
            },
            RuntimeRequest::EventReserve {
                request_id: String::new(),
                program_instance_id: "pi.1".to_owned(),
                owner_claim: owner_claim.clone(),
                type_id: "UserInput".to_owned(),
            },
            RuntimeRequest::EventFulfill {
                request_id: String::new(),
                owner_claim: owner_claim.clone(),
                application,
            },
            RuntimeRequest::EventList {
                request_id: String::new(),
                owner_claim: owner_claim.clone(),
            },
            RuntimeRequest::EventInspect {
                request_id: String::new(),
                owner_claim: owner_claim.clone(),
                event_ref: CanonicalEventRef {
                    event_id: "evt.1".to_owned(),
                    generation: 1,
                },
            },
            RuntimeRequest::EventExpire {
                request_id: String::new(),
                owner_claim: owner_claim.clone(),
                event_ref: CanonicalEventRef {
                    event_id: "evt.1".to_owned(),
                    generation: 1,
                },
            },
            RuntimeRequest::EventCancel {
                request_id: String::new(),
                owner_claim: owner_claim.clone(),
                event_ref: CanonicalEventRef {
                    event_id: "evt.1".to_owned(),
                    generation: 1,
                },
            },
            RuntimeRequest::ProgramInvocationCancel {
                request_id: String::new(),
                owner_claim,
                program_invocation_id: "invocation.1".to_owned(),
            },
        ];
        for request in requests {
            let mut service = RuntimeService::default();
            assert_eq!(
                service.handle(&handshake(), request),
                Err(ProtocolError::InvalidRequest)
            );
            assert!(service.instances.is_empty());
            assert!(service.reservations.is_empty());
        }
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
        let digest = canonical_artifact_digest(&bytes).expect("canonical fixture digest");
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
        assert!(
            matches!(
                result,
                RuntimeResult::Failed { ref code, .. } if code == "unknown_artifact"
            ),
            "{result:?}"
        );
    }

    #[test]
    fn durable_bytes_accept_only_bounded_canonical_base64_or_legacy_arrays() {
        let bytes = DurableBytes::<4>(vec![0, 1, 254, 255]);
        let encoded = serde_json::to_value(&bytes).unwrap();
        assert_eq!(encoded, serde_json::json!("base64:AAH+/w=="));
        assert_eq!(
            serde_json::from_value::<DurableBytes<4>>(encoded)
                .unwrap()
                .0,
            bytes.0
        );
        assert_eq!(
            serde_json::from_value::<DurableBytes<4>>(serde_json::json!([0, 1, 254, 255]))
                .unwrap()
                .0,
            bytes.0
        );
        for malformed in [
            serde_json::json!("AAH+/w=="),
            serde_json::json!("base64:AAH+/w="),
            serde_json::json!("base64:AAH+/w==AA=="),
            serde_json::json!("base64:AAECAwQ="),
            serde_json::json!([0, 1, 2, 3, 4]),
            serde_json::json!({"base64":"AAH+/w=="}),
        ] {
            assert!(serde_json::from_value::<DurableBytes<4>>(malformed).is_err());
        }
    }

    #[test]
    fn authenticated_legacy_metadata_reopens_and_rewrites_exact_carriers() {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().to_path_buf();
        let bytes = fixture_air_bytes();
        let release = br#"{"schema_version":"apxm.test.release"}"#.to_vec();
        let provenance = br#"{"schema_version":"apxm.test.provenance"}"#.to_vec();
        let profile =
            RuntimeAdmissionProfile::from_carriers(release.clone(), provenance.clone()).unwrap();
        let mut service = RuntimeService::default()
            .with_runtime_state_dir(state_path.clone())
            .with_admission_profile(profile.clone());
        let digest = service.admit_artifact(bytes.clone());
        let created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "durable.bytes.create".into(),
                    artifact_digest: digest.clone(),
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            ..
        } = created
        else {
            panic!("expected instance: {created:?}");
        };
        let materials = profile.materials_for_artifact(&bytes);
        service
            .bind_admission_for_artifact(&digest, materials.clone())
            .unwrap();
        service
            .bind_admission(&program_instance_id, materials.clone())
            .unwrap();
        let mut metadata = service.execution_backend.runtime_metadata().unwrap();
        assert!(metadata["artifacts"][&digest].is_string());
        assert!(metadata["artifact_admissions"][&digest]["release_bytes"].is_string());
        assert!(
            metadata["instances"][&program_instance_id]["materials"]["provenance_bytes"]
                .is_string()
        );
        // Store the original array representation through the authenticated
        // commit-local path, then reopen it with the new private decoder.
        metadata["artifacts"][&digest] = serde_json::json!(bytes);
        metadata["artifact_admissions"][&digest]["release_bytes"] = serde_json::json!(release);
        metadata["artifact_admissions"][&digest]["provenance_bytes"] =
            serde_json::json!(provenance);
        metadata["instances"][&program_instance_id]["materials"]["release_bytes"] =
            serde_json::json!(release);
        metadata["instances"][&program_instance_id]["materials"]["provenance_bytes"] =
            serde_json::json!(provenance);
        service
            .execution_backend
            .set_runtime_metadata(Some(metadata))
            .unwrap();
        drop(service);

        let reopened = RuntimeService::default()
            .with_runtime_state_dir(state_path)
            .with_admission_profile(profile);
        assert!(
            reopened.startup_error().is_none(),
            "{:?}",
            reopened.startup_error()
        );
        assert_eq!(reopened.artifacts.get(&digest), Some(bytes.as_slice()));
        assert_eq!(reopened.artifact_admissions.get(&digest), Some(&materials));
        assert_eq!(
            reopened.instances[&program_instance_id].materials.as_ref(),
            Some(&materials)
        );
        reopened.persist_runtime_state().unwrap();
        let rewritten = reopened.execution_backend.runtime_metadata().unwrap();
        assert!(rewritten["artifacts"][&digest].is_string());
        assert!(rewritten["artifact_admissions"][&digest]["release_bytes"].is_string());
        assert!(
            rewritten["instances"][&program_instance_id]["materials"]["release_bytes"].is_string()
        );
        // This private representation does not change the public materials wire.
        assert!(serde_json::to_value(&materials).unwrap()["release_bytes"].is_array());
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
    fn tampered_admission_is_rejected_before_state_mutation() {
        let mut service = RuntimeService::default();
        let bytes = fixture_air_bytes();
        let digest = service.admit_artifact(bytes.clone());
        let created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "tamper.create".to_owned(),
                    artifact_digest: digest,
                },
            )
            .expect("create instance");
        let (instance, claim) = match created {
            RuntimeResult::ProgramInstanceCreated {
                program_instance_id,
                owner_claim,
                ..
            } => (program_instance_id, owner_claim),
            other => panic!("instance creation failed: {other:?}"),
        };
        let release = fs::read(fixture_dir().join("canonical-execute.release.json")).unwrap();
        let provenance = fs::read(fixture_dir().join("canonical-execute.provenance.json")).unwrap();
        let mut materials =
            materials_for_artifact(&bytes, "tamper.invocation", release, provenance);
        materials.release_bytes.push(b'x');
        let error = service
            .bind_admission(&instance, materials)
            .expect_err("tampered admission must fail before binding");
        assert!(
            error.starts_with("release digest mismatch:"),
            "unexpected admission error: {error}"
        );
        let started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "tamper.start".to_owned(),
                    program_instance_id: instance,
                    owner_claim: claim,
                    input: serde_json::json!({}),
                },
            )
            .expect("start request");
        assert!(matches!(
            started,
            RuntimeResult::Failed { code, .. } if code == "missing_invocation_admission"
        ));
    }

    #[test]
    fn host_only_profile_identity_and_materials_are_exact() {
        let release = br#"{"schema_version":"apxm.test.release"}"#.to_vec();
        let provenance = br#"{"schema_version":"apxm.test.provenance"}"#.to_vec();
        let portable =
            RuntimeAdmissionProfile::from_carriers(release.clone(), provenance.clone()).unwrap();
        let host_only = RuntimeAdmissionProfile::from_carriers_with_capability_profile(
            release,
            provenance,
            RuntimeCapabilityProfile::HostOnly,
        )
        .unwrap();
        assert_ne!(portable.profile_ref(), host_only.profile_ref());
        assert_ne!(
            canonical_port_bindings_digest(),
            port_bindings_digest_for(RuntimeCapabilityProfile::HostOnly),
        );
        let portable_descriptor = runtime_descriptor_for(RuntimeCapabilityProfile::PortableLocal);
        let host_descriptor = runtime_descriptor_for(RuntimeCapabilityProfile::HostOnly);
        assert_eq!(
            portable_descriptor.port_bindings.len(),
            host_descriptor.port_bindings.len()
        );
        for (portable, host) in portable_descriptor
            .port_bindings
            .iter()
            .zip(&host_descriptor.port_bindings)
        {
            assert_eq!(portable.slot, host.slot);
            assert_eq!(portable.port_contract_digest, host.port_contract_digest);
            if portable.slot == apxm_kernel::PortSlot::Capability.as_str() {
                assert_ne!(portable.binding_digest, host.binding_digest);
                assert_ne!(portable.proof_digest, host.proof_digest);
            } else {
                assert_eq!(portable.binding_digest, host.binding_digest);
                assert_eq!(portable.proof_digest, host.proof_digest);
            }
        }
        let bytes = fixture_air_bytes();
        let materials = host_only.materials_for_artifact(&bytes);
        verify_invocation_materials_for_profile(
            &bytes,
            &materials,
            RuntimeCapabilityProfile::HostOnly,
        )
        .expect("host-only materials verify against host-only descriptor");
        assert!(verify_invocation_materials(&bytes, &materials).is_err());
    }

    #[test]
    fn changed_profile_refuses_pending_start_without_rebinding_durable_materials() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let bytes = fixture_air_bytes();
        let release = br#"{"schema_version":"apxm.test.release"}"#.to_vec();
        let provenance = br#"{"schema_version":"apxm.test.provenance"}"#.to_vec();
        let host_only = RuntimeAdmissionProfile::from_carriers_with_capability_profile(
            release.clone(),
            provenance.clone(),
            RuntimeCapabilityProfile::HostOnly,
        )
        .unwrap();
        let portable = RuntimeAdmissionProfile::from_carriers(release, provenance).unwrap();
        let instance_id = {
            let mut service = RuntimeService::default()
                .with_runtime_state_dir(path.clone())
                .with_admission_profile(host_only.clone());
            let digest = service.admit_artifact(bytes.clone());
            let created = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInstanceCreate {
                        request_id: "profile.create".into(),
                        artifact_digest: digest,
                    },
                )
                .unwrap();
            let RuntimeResult::ProgramInstanceCreated {
                program_instance_id,
                admission_profile: Some(descriptor),
                ..
            } = created
            else {
                panic!("expected profile descriptor");
            };
            assert_eq!(descriptor.profile_ref, host_only.profile_ref());
            assert_eq!(
                descriptor.port_bindings_digest,
                port_bindings_digest_for(RuntimeCapabilityProfile::HostOnly)
            );
            let materials = host_only.materials_for_artifact(&bytes);
            service
                .bind_admission(&program_instance_id, materials.clone())
                .unwrap();
            service
                .bind_admission(&program_instance_id, materials)
                .expect("same binding is idempotent");
            assert_eq!(
                service.bind_admission(
                    &program_instance_id,
                    portable.materials_for_artifact(&bytes)
                ),
                Err("admission_profile_mismatch".into())
            );
            program_instance_id
        };
        let mut reopened = RuntimeService::default()
            .with_runtime_state_dir(path)
            .with_admission_profile(portable);
        let claim = owner_claim(&reopened, &instance_id);
        let start = reopened
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "profile.start".into(),
                    program_instance_id: instance_id.clone(),
                    owner_claim: claim,
                    input: Value::Null,
                },
            )
            .unwrap();
        assert!(
            matches!(start, RuntimeResult::Failed { code, .. } if code == "admission_profile_mismatch")
        );
        assert!(
            reopened
                .instances
                .get(&instance_id)
                .unwrap()
                .invocation
                .is_none()
        );
        assert!(reopened.active_cancellations.is_empty());
    }

    #[test]
    fn host_only_still_parks_a_host_capability_request() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let state_path = directory.path().to_path_buf();
        let bytes = host_capability_air_bytes();
        let profile = RuntimeAdmissionProfile::from_carriers_with_capability_profile(
            br#"{"schema_version":"apxm.test.release"}"#.to_vec(),
            br#"{"schema_version":"apxm.test.provenance"}"#.to_vec(),
            RuntimeCapabilityProfile::HostOnly,
        )
        .unwrap();
        let mut service = RuntimeService::default()
            .with_runtime_state_dir(state_path.clone())
            .with_admission_profile(profile.clone());
        let digest = service.admit_artifact(bytes.clone());
        let created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "host.profile.create".into(),
                    artifact_digest: digest,
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        } = created
        else {
            panic!("expected instance");
        };
        service
            .bind_admission(&program_instance_id, profile.materials_for_artifact(&bytes))
            .unwrap();
        let started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "host.profile.start".into(),
                    program_instance_id: program_instance_id.clone(),
                    owner_claim: owner_claim.clone(),
                    input: Value::Null,
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInvocationStarted {
            program_invocation_id,
            ..
        } = started
        else {
            panic!("host-only request did not park: {started:?}");
        };
        let committed = service
            .execution_backend
            .load_continuation(&ProgramInstanceRef::new(program_instance_id.clone()))
            .expect("host request is parked durably");
        let continuation: Continuation = serde_json::from_value(committed.payload).unwrap();
        let capability_request_id = continuation
            .event_ref
            .expect("host request event")
            .as_str()
            .to_owned();
        drop(service);
        let portable = RuntimeAdmissionProfile::from_carriers(
            br#"{"schema_version":"apxm.test.release"}"#.to_vec(),
            br#"{"schema_version":"apxm.test.provenance"}"#.to_vec(),
        )
        .unwrap();
        let mut reopened = RuntimeService::default()
            .with_runtime_state_dir(state_path.clone())
            .with_admission_profile(portable.clone())
            .with_embedded_read_access();
        let second_created = reopened
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "portable.profile.create".into(),
                    artifact_digest: canonical_artifact_digest(&bytes).unwrap(),
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id: second_instance_id,
            owner_claim: second_claim,
            ..
        } = second_created
        else {
            panic!("expected second instance");
        };
        reopened
            .bind_admission(&second_instance_id, portable.materials_for_artifact(&bytes))
            .unwrap();
        let second_start = reopened
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "portable.profile.start".into(),
                    program_instance_id: second_instance_id.clone(),
                    owner_claim: second_claim.clone(),
                    input: Value::Null,
                },
            )
            .unwrap();
        assert!(
            matches!(second_start, RuntimeResult::ProgramInvocationStarted { .. }),
            "{second_start:?}"
        );
        let second_committed = reopened
            .execution_backend
            .load_continuation(&ProgramInstanceRef::new(second_instance_id.clone()))
            .expect("second host wait");
        let second_version = block_on(
            reopened
                .execution_backend
                .commit_port()
                .current_version(&ProgramInstanceRef::new(second_instance_id.clone())),
        );
        let second_continuation: Continuation =
            serde_json::from_value(second_committed.payload.clone()).unwrap();
        let second_request_id = second_continuation
            .event_ref
            .expect("second host request")
            .as_str()
            .to_owned();
        let metadata_before_refusal = reopened
            .execution_backend
            .runtime_metadata()
            .expect("parked service metadata");
        for (request_id, claim, receipt) in [
            (
                capability_request_id.clone(),
                owner_claim.clone(),
                "receipt.profile.old",
            ),
            (second_request_id, second_claim, "receipt.profile.current"),
        ] {
            let settled = reopened.settle_host_capability(
                format!("settle.{receipt}"),
                claim,
                request_id,
                HostCapabilityOutcomeKind::Ok,
                Some("{\"matches\":1}".into()),
                Some(receipt.into()),
                None,
                false,
            );
            assert!(
                matches!(settled, RuntimeResult::CapabilitySettled { .. }),
                "{settled:?}"
            );
        }
        assert!(
            reopened
                .claim_host_capability_resume(&capability_request_id)
                .unwrap()
                .is_none()
        );
        let inspection = reopened
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::ProgramInvocationInspect {
                    context: read_context(ReadPurpose::Inspection),
                    program_invocation_id: ProgramInvocationId::new(program_invocation_id.clone())
                        .unwrap(),
                    node_execution_id: None,
                },
            )
            .unwrap();
        assert!(
            matches!(&inspection, RuntimeResultV2::ProgramInvocationInspection { inspection, .. }
            if inspection.status == ProgramInvocationStatus::Failed),
            "{inspection:?}"
        );
        let evidence = reopened
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::EvidenceRead {
                    context: read_context(ReadPurpose::Evidence),
                    program_invocation_id: ProgramInvocationId::new(program_invocation_id.clone())
                        .unwrap(),
                    after_cursor: None,
                    limit: 100,
                },
            )
            .unwrap();
        let RuntimeResultV2::EvidencePage {
            page: evidence_page,
            ..
        } = evidence
        else {
            panic!("expected committed failure evidence: {evidence:?}");
        };
        let refusal = evidence_page
            .items
            .iter()
            .find(|record| {
                record
                    .typed_error
                    .as_ref()
                    .is_some_and(|error| error.code_ref == "admission_profile_mismatch")
            })
            .expect("typed profile refusal evidence");
        assert_eq!(
            refusal.typed_error.as_ref().unwrap().category,
            apxm_runtime_protocol::EvidenceErrorCategory::Admission
        );
        assert!(refusal.node_execution_id.is_none());
        let observations = reopened
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::ObservationSubscribe {
                    context: read_context(ReadPurpose::Observation),
                    program_invocation_id: ProgramInvocationId::new(program_invocation_id.clone())
                        .unwrap(),
                    after_cursor: None,
                    limit: 100,
                },
            )
            .unwrap();
        assert!(
            matches!(observations, RuntimeResultV2::ObservationPage { page, .. }
            if page.items.iter().any(|observation| observation.observation_kind == apxm_runtime_protocol::ObservationKind::InvocationFailed
                && observation.evidence_ref.as_ref() == Some(&refusal.evidence_ref)
                && observation.node_execution_id.is_none()))
        );
        let replay = reopened
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "host.profile.start".into(),
                    program_instance_id: program_instance_id.clone(),
                    owner_claim: owner_claim.clone(),
                    input: Value::Null,
                },
            )
            .unwrap();
        assert!(
            matches!(replay, RuntimeResult::Failed { code, .. } if code == "admission_profile_mismatch")
        );
        let second = reopened
            .claim_next_continuation_resume()
            .unwrap()
            .expect("current profile resumes");
        assert_eq!(second.program_instance_id, second_instance_id);
        assert!(reopened.begin_continuation_resume(&second).unwrap());
        let execution = second.execute();
        assert!(execution.is_ok(), "{execution:?}");
        reopened.finish_continuation_resume(&second, execution);
        assert!(reopened.active_cancellations.is_empty());
        let stale_refusal = block_on(apxm_execution::commit_parked_admission_failure(
            reopened.execution_backend.commit_port().as_ref(),
            second_version,
            &second_committed,
        ))
        .unwrap();
        assert!(
            matches!(stale_refusal, ExecutionCommitResult::CompareConflict { .. }),
            "{stale_refusal:?}"
        );
        assert_ne!(
            reopened
                .execution_backend
                .invocation_status(&second.invocation_id),
            Some(ProgramInvocationStatus::Failed)
        );
        // Simulate a crash after the authoritative refusal commit but before
        // its separate service metadata update. Recovery must use the exact
        // typed terminal evidence and must not redispatch the parked request.
        reopened
            .execution_backend
            .set_runtime_metadata(Some(metadata_before_refusal))
            .unwrap();
        drop(second);
        drop(reopened);
        let mut reread = RuntimeService::default()
            .with_runtime_state_dir(state_path)
            .with_admission_profile(portable)
            .with_embedded_read_access();
        assert!(
            reread.startup_error().is_none(),
            "{:?}",
            reread.startup_error()
        );
        let durable = reread
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::ProgramInvocationInspect {
                    context: read_context(ReadPurpose::Inspection),
                    program_invocation_id: ProgramInvocationId::new(program_invocation_id.clone())
                        .unwrap(),
                    node_execution_id: None,
                },
            )
            .unwrap();
        assert!(
            matches!(durable, RuntimeResultV2::ProgramInvocationInspection { inspection, .. }
            if inspection.status == ProgramInvocationStatus::Failed)
        );
        let durable_evidence = reread
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::EvidenceRead {
                    context: read_context(ReadPurpose::Evidence),
                    program_invocation_id: ProgramInvocationId::new(program_invocation_id.clone())
                        .unwrap(),
                    after_cursor: None,
                    limit: 100,
                },
            )
            .unwrap();
        assert!(
            matches!(durable_evidence, RuntimeResultV2::EvidencePage { page, .. }
            if page.items.iter().filter(|record| record.typed_error.as_ref()
                .is_some_and(|error| error.code_ref == "admission_profile_mismatch")).count() == 1)
        );
        reread.reconcile_recovery_state().unwrap();
        let replay = reread
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "host.profile.start".into(),
                    program_instance_id,
                    owner_claim,
                    input: Value::Null,
                },
            )
            .unwrap();
        assert!(
            matches!(&replay, RuntimeResult::Failed { code, .. }
            if code == "admission_profile_mismatch"),
            "{replay:?}"
        );
    }

    #[test]
    fn uncertain_profile_refusal_keeps_recovery_uncertain_without_failure_fact() {
        use sha2::Digest;

        let bytes = host_capability_air_bytes();
        let host = RuntimeAdmissionProfile::from_carriers_with_capability_profile(
            br#"{"schema_version":"apxm.test.release"}"#.to_vec(),
            br#"{"schema_version":"apxm.test.provenance"}"#.to_vec(),
            RuntimeCapabilityProfile::HostOnly,
        )
        .unwrap();
        let portable = RuntimeAdmissionProfile::from_carriers(
            br#"{"schema_version":"apxm.test.release"}"#.to_vec(),
            br#"{"schema_version":"apxm.test.provenance"}"#.to_vec(),
        )
        .unwrap();
        let mut service = RuntimeService::default()
            .with_admission_profile(host.clone())
            .with_embedded_read_access();
        let digest = service.admit_artifact(bytes.clone());
        let created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "unknown.profile.create".into(),
                    artifact_digest: digest,
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        } = created
        else {
            panic!("expected instance: {created:?}");
        };
        service
            .bind_admission(&program_instance_id, host.materials_for_artifact(&bytes))
            .unwrap();
        let started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "unknown.profile.start".into(),
                    program_instance_id: program_instance_id.clone(),
                    owner_claim: owner_claim.clone(),
                    input: Value::Null,
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInvocationStarted {
            program_invocation_id,
            ..
        } = started
        else {
            panic!("expected parked invocation: {started:?}");
        };
        let committed = service
            .execution_backend
            .load_continuation(&ProgramInstanceRef::new(program_instance_id.clone()))
            .unwrap();
        let continuation: Continuation = serde_json::from_value(committed.payload.clone()).unwrap();
        let capability_request_id = continuation.event_ref.unwrap().as_str().to_owned();
        let version = block_on(
            service
                .execution_backend
                .commit_port()
                .current_version(&ProgramInstanceRef::new(program_instance_id.clone())),
        );
        let identity = format!(
            "{}:{}:{}",
            continuation.commit_id, committed.digest, version
        );
        let commit_id = format!(
            "commit.profile-refusal.{:x}",
            sha2::Sha256::digest(identity.as_bytes())
        );
        let RuntimeExecutionBackend::Memory(commit) = &service.execution_backend else {
            panic!("expected memory commit adapter");
        };
        commit.inject_outcome_unknown(commit_id);
        service.admission_profile = Some(portable.clone());
        let other_created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "unknown.profile.other.create".into(),
                    artifact_digest: canonical_artifact_digest(&bytes).unwrap(),
                },
            )
            .unwrap();
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id: other_instance_id,
            owner_claim: other_claim,
            ..
        } = other_created
        else {
            panic!("expected independent instance: {other_created:?}");
        };
        service
            .bind_admission(&other_instance_id, portable.materials_for_artifact(&bytes))
            .unwrap();
        let other_started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "unknown.profile.other.start".into(),
                    program_instance_id: other_instance_id.clone(),
                    owner_claim: other_claim.clone(),
                    input: Value::Null,
                },
            )
            .unwrap();
        assert!(matches!(
            other_started,
            RuntimeResult::ProgramInvocationStarted { .. }
        ));
        let other_committed = service
            .execution_backend
            .load_continuation(&ProgramInstanceRef::new(other_instance_id.clone()))
            .unwrap();
        let other_wait: Continuation = serde_json::from_value(other_committed.payload).unwrap();
        let other_request_id = other_wait.event_ref.unwrap().as_str().to_owned();
        let settled = service.settle_host_capability(
            "unknown.profile.settle".into(),
            owner_claim.clone(),
            capability_request_id.clone(),
            HostCapabilityOutcomeKind::Failed,
            None,
            None,
            Some("neutral failure".into()),
            false,
        );
        assert!(
            matches!(settled, RuntimeResult::CapabilitySettled { .. }),
            "{settled:?}"
        );
        assert!(
            service
                .claim_host_capability_resume(&capability_request_id)
                .unwrap()
                .is_none()
        );
        assert!(service.active_cancellations.is_empty());
        let other_settled = service.settle_host_capability(
            "unknown.profile.other.settle".into(),
            other_claim,
            other_request_id,
            HostCapabilityOutcomeKind::Ok,
            Some("{\"matches\":1}".into()),
            Some("receipt.profile.other".into()),
            None,
            false,
        );
        assert!(matches!(
            other_settled,
            RuntimeResult::CapabilitySettled { .. }
        ));
        let other = service
            .claim_next_continuation_resume()
            .unwrap()
            .expect("independent parked invocation remains eligible");
        assert_eq!(other.program_instance_id, other_instance_id);
        assert!(service.begin_continuation_resume(&other).unwrap());
        let execution = other.execute();
        assert!(execution.is_ok(), "{execution:?}");
        service.finish_continuation_resume(&other, execution);
        let replay = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "unknown.profile.start".into(),
                    program_instance_id,
                    owner_claim,
                    input: Value::Null,
                },
            )
            .unwrap();
        assert!(matches!(replay, RuntimeResult::Failed { code, .. } if code == "outcome_unknown"));
        assert_eq!(
            service
                .execution_backend
                .invocation_status(&program_invocation_id),
            Some(ProgramInvocationStatus::OutcomeUnknown)
        );
        let evidence = service
            .handle_v2(
                &RuntimeHandshakeV2::server(),
                RuntimeRequestV2::EvidenceRead {
                    context: read_context(ReadPurpose::Evidence),
                    program_invocation_id: ProgramInvocationId::new(program_invocation_id).unwrap(),
                    after_cursor: None,
                    limit: 100,
                },
            )
            .unwrap();
        assert!(
            matches!(evidence, RuntimeResultV2::EvidencePage { page, .. }
            if page.items.iter().all(|record| record.typed_error.as_ref()
                .is_none_or(|error| error.code_ref != "admission_profile_mismatch")))
        );
    }

    #[test]
    fn terminal_invocation_replays_after_profile_change_without_new_execution() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let state_path = directory.path().to_path_buf();
        let air: AirModule = serde_json::from_value(serde_json::json!({
            "schema_version":"apxm.air", "semantic_operations":[],
            "structural_ir":[
                {"region_id":"root","kind":"function","execution_order":0},
                {"region_id":"return","kind":"return","parent_region_id":"root","execution_order":1,
                 "operands":[{"slot":"output","value_id":"reply","type_ref":"Output"}]}
            ],
            "value_assemblies":[{"value_id":"reply","expression":{"kind":"string","value":"ready"}}],
            "context_flow":[],
            "source_map":{"schema_version":"apxm.source-map","source_language":"python","node_spans":[],"region_annotations":[]}
        })).unwrap();
        let bytes = ExecutableArtifact::from_air(&air)
            .unwrap()
            .encode()
            .unwrap();
        let release = br#"{"schema_version":"apxm.test.release"}"#.to_vec();
        let provenance = br#"{"schema_version":"apxm.test.provenance"}"#.to_vec();
        let host_only = RuntimeAdmissionProfile::from_carriers_with_capability_profile(
            release.clone(),
            provenance.clone(),
            RuntimeCapabilityProfile::HostOnly,
        )
        .unwrap();
        let (instance_id, claim, first) = {
            let mut service = RuntimeService::default()
                .with_runtime_state_dir(state_path.clone())
                .with_admission_profile(host_only.clone());
            let digest = service.admit_artifact(bytes.clone());
            let created = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInstanceCreate {
                        request_id: "terminal.profile.create".into(),
                        artifact_digest: digest,
                    },
                )
                .unwrap();
            let RuntimeResult::ProgramInstanceCreated {
                program_instance_id,
                owner_claim,
                ..
            } = created
            else {
                panic!("expected instance");
            };
            service
                .bind_admission(
                    &program_instance_id,
                    host_only.materials_for_artifact(&bytes),
                )
                .unwrap();
            let first = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationStart {
                        request_id: "terminal.profile.start".into(),
                        program_instance_id: program_instance_id.clone(),
                        owner_claim: owner_claim.clone(),
                        input: serde_json::json!({"value":"once"}),
                    },
                )
                .unwrap();
            let RuntimeResult::ProgramInvocationStarted {
                ref program_invocation_id,
                ..
            } = first
            else {
                panic!("pure invocation did not commit: {first:?}");
            };
            assert_eq!(
                service
                    .execution_backend
                    .invocation_status(program_invocation_id),
                Some(ProgramInvocationStatus::CommittedReturn)
            );
            (program_instance_id, owner_claim, first)
        };
        let portable = RuntimeAdmissionProfile::from_carriers(release, provenance).unwrap();
        let mut reopened = RuntimeService::default()
            .with_runtime_state_dir(state_path)
            .with_admission_profile(portable);
        let replay = reopened
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "terminal.profile.start".into(),
                    program_instance_id: instance_id,
                    owner_claim: claim,
                    input: serde_json::json!({"value":"once"}),
                },
            )
            .unwrap();
        assert_eq!(replay, first);
        assert!(reopened.active_cancellations.is_empty());
    }

    #[test]
    fn persisted_artifact_create_bind_and_start_uses_the_wire_admission_seam() {
        let artifact_directory = tempfile::tempdir().expect("artifact directory");
        let bytes = fixture_air_bytes();
        let digest = canonical_artifact_digest(&bytes).expect("canonical artifact digest");
        fs::write(
            artifact_directory.path().join(digest.replace(':', "-")),
            &bytes,
        )
        .expect("persist artifact");

        // This is the same composition used by the standalone binary: the
        // executable is loaded from the shared artifact directory, while
        // admission arrives explicitly over the Runtime protocol.
        let profile = RuntimeAdmissionProfile::from_carriers(
            br#"{"schema_version":"apxm.test.release"}"#.to_vec(),
            br#"{"schema_version":"apxm.test.provenance"}"#.to_vec(),
        )
        .expect("admission profile");
        let profile_ref = profile.profile_ref().to_owned();
        let mut service = RuntimeService::default()
            .with_artifact_dir(artifact_directory.path().to_path_buf())
            .with_admission_profile(profile);
        let created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "create.persisted".to_owned(),
                    artifact_digest: digest.clone(),
                },
            )
            .expect("create persisted artifact instance");
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        } = created
        else {
            panic!("expected instance creation");
        };
        let bound = service.handle_execution_admission(
            &RuntimeExecutionAdmissionHandshake::server(),
            RuntimeExecutionAdmissionRequest::ProgramInstanceBindAdmission {
                request_id: "bind.persisted".to_owned(),
                program_instance_id: program_instance_id.clone(),
                owner_claim: owner_claim.clone(),
                admission_profile_ref: profile_ref,
            },
        );
        assert!(matches!(
            bound,
            RuntimeResult::ProgramInstanceAdmissionBound { .. }
        ));
        let started = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "start.persisted".to_owned(),
                    program_instance_id,
                    owner_claim,
                    input: serde_json::json!({}),
                },
            )
            .expect("start admitted persisted artifact");
        assert!(
            matches!(started, RuntimeResult::ProgramInvocationStarted { .. }),
            "admitted persisted artifact did not start: {started:?}"
        );
    }

    #[test]
    fn event_fulfill_uses_root_application_record() {
        let mut service = RuntimeService::default();
        let instance = create_started(&mut service, event_air_bytes());
        let reserved = service
            .handle(
                &handshake(),
                RuntimeRequest::EventReserve {
                    request_id: "r".to_owned(),
                    program_instance_id: instance.clone(),
                    owner_claim: owner_claim(&service, &instance),
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
                            payload: serde_json::json!({"answer":"ok"}),
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

    #[test]
    fn raw_air_is_not_admitted_as_an_executable_artifact() {
        let raw_air = br#"{"schema_version":"apxm.air","semantic_operations":[],"structural_ir":[],"context_flow":[],"source_map":{"schema_version":"apxm.source-map","source_language":"python","node_spans":[],"region_annotations":[]}}"#;
        let mut service = RuntimeService::default();
        assert_eq!(service.admit_artifact(raw_air.to_vec()), "");
        assert!(service.artifact_bytes(&artifact_digest(raw_air)).is_none());
    }

    pub(super) fn create_started(service: &mut RuntimeService, bytes: Vec<u8>) -> String {
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
        let mut air: serde_json::Value = serde_json::from_slice(
            &fs::read(fixture_dir().join("canonical-capability-execute.air.json")).unwrap(),
        )
        .unwrap();
        air["capability_permission_requests"] = serde_json::json!({
            "read_skill": { "decision": "ask", "reason": "confirm" }
        });
        let air: AirModule = serde_json::from_value(air).unwrap();
        ExecutableArtifact::from_air(&air)
            .unwrap()
            .encode()
            .unwrap()
    }

    struct CountingBroker {
        asked: Arc<AtomicUsize>,
        decision: ApprovalDecision,
    }

    #[async_trait::async_trait]
    impl ApprovalBroker for CountingBroker {
        async fn resolve_ask(&self, _ask_id: &str) -> ApprovalDecision {
            self.asked.fetch_add(1, Ordering::Release);
            self.decision
        }
    }

    struct SilentBroker;

    #[async_trait::async_trait]
    impl ApprovalBroker for SilentBroker {
        async fn resolve_ask(&self, _ask_id: &str) -> ApprovalDecision {
            std::future::pending().await
        }
    }

    fn start_ask_invocation(service: &mut RuntimeService) -> RuntimeResult {
        let instance = create_started(service, ask_air_bytes());
        service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "s".to_owned(),
                    owner_claim: owner_claim(service, &instance),
                    program_instance_id: instance,
                    input: serde_json::json!({}),
                },
            )
            .unwrap()
    }

    #[test]
    fn approval_policy_reads_deny_and_timeout_and_rejects_anything_else() {
        assert_eq!(ApprovalPolicy::parse("").unwrap(), ApprovalPolicy::Deny);
        assert_eq!(ApprovalPolicy::parse("deny").unwrap(), ApprovalPolicy::Deny);
        assert_eq!(
            ApprovalPolicy::parse(" timeout:250 ").unwrap(),
            ApprovalPolicy::Timeout(Duration::from_millis(250))
        );
        for malformed in [
            "allow",
            "timeout",
            "timeout:",
            "timeout:-1",
            "timeout:2s",
            "timeout:0",
        ] {
            let error = ApprovalPolicy::parse(malformed)
                .expect_err("a value that is not a policy must not be read as one");
            assert!(
                error.contains("deny") || error.contains("timeout"),
                "{malformed}: {error}"
            );
        }
        assert_eq!(
            RuntimeServiceStartupError::InvalidApprovalPolicy(
                ApprovalPolicy::parse("allow").unwrap_err()
            )
            .to_string(),
            "invalid APXM_APPROVAL_POLICY: \"allow\" is not a policy; expected \"deny\" or \"timeout:<ms>\""
        );
    }

    #[test]
    fn deny_policy_refuses_a_builtin_ask_without_consulting_the_broker() {
        let mut service = RuntimeService::default();
        assert_eq!(service.approval_policy(), ApprovalPolicy::Deny);
        let asked = Arc::new(AtomicUsize::new(0));
        service.bind_approval_broker(Arc::new(CountingBroker {
            asked: Arc::clone(&asked),
            decision: ApprovalDecision::Allow,
        }));
        let result = start_ask_invocation(&mut service);
        assert!(
            matches!(result, RuntimeResult::Failed { ref code, .. } if code == "ask_denied"),
            "{result:?}"
        );
        assert_eq!(asked.load(Ordering::Acquire), 0);
    }

    #[test]
    fn timeout_policy_admits_an_answered_ask_and_denies_an_unanswered_one() {
        let mut service = RuntimeService::default();
        let asked = Arc::new(AtomicUsize::new(0));
        service.bind_approval_policy(ApprovalPolicy::Timeout(Duration::from_secs(30)));
        service.bind_approval_broker(Arc::new(CountingBroker {
            asked: Arc::clone(&asked),
            decision: ApprovalDecision::Allow,
        }));
        let allowed = start_ask_invocation(&mut service);
        assert!(
            !matches!(allowed, RuntimeResult::Failed { ref code, .. } if code.starts_with("ask_")),
            "{allowed:?}"
        );
        assert_eq!(asked.load(Ordering::Acquire), 1);

        let mut refusing = RuntimeService::default();
        refusing.bind_approval_policy(ApprovalPolicy::Timeout(Duration::from_millis(25)));
        refusing.bind_approval_broker(Arc::new(SilentBroker));
        let unanswered = start_ask_invocation(&mut refusing);
        assert!(
            matches!(unanswered, RuntimeResult::Failed { ref code, .. } if code == "ask_timeout"),
            "{unanswered:?}"
        );
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
    }

    #[test]
    fn timed_out_ask_never_executes() {
        let mut service = RuntimeService::default();
        service.bind_approval_policy(ApprovalPolicy::Timeout(Duration::from_millis(500)));
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
    fn a_second_client_cancels_a_long_running_invocation_and_commits_terminal_truth() {
        let bytes = ask_air_bytes();
        let mut service = RuntimeService::default().with_embedded_read_access();
        let digest = service
            .try_admit_artifact(bytes.clone())
            .expect("artifact admission");
        let created = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "create.long-running".to_owned(),
                    artifact_digest: digest.clone(),
                },
            )
            .expect("instance create");
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        } = created
        else {
            panic!("expected instance creation");
        };
        let mut materials = materials_for_artifact(
            &bytes,
            format!("{program_instance_id}:invocation-admission"),
            b"{}".to_vec(),
            b"{}".to_vec(),
        );
        // The runtime binds the canonical envelope digest, not the raw byte
        // hash used by the generic fixture helper.
        materials.admission.artifact_digest = digest;
        service
            .bind_admission(&program_instance_id, materials)
            .expect("invocation admission");

        let gate = Arc::new(InvocationGate {
            released: Arc::new(AtomicBool::new(false)),
        });
        service.bind_approval_policy(ApprovalPolicy::Timeout(Duration::from_secs(60)));
        service.bind_approval_broker(gate.clone());
        let shared = Arc::new(Mutex::new(service));
        let prepared = {
            let mut guard = shared.lock().expect("service lock");
            guard
                .prepare_invocation(
                    "start.long-running".to_owned(),
                    program_instance_id.clone(),
                    owner_claim.clone(),
                    Value::Null,
                )
                .expect("invocation claim")
        };
        let invocation_id = prepared.invocation_id.clone();
        let execution_service = Arc::clone(&shared);
        let execution = std::thread::spawn(move || {
            let output = prepared.execute();
            execution_service
                .lock()
                .expect("service lock")
                .finish_invocation(&prepared, output)
        });

        // This is the independent second-client request. It must acquire the
        // service mutex while the first client remains inside the long-running
        // invocation, then signal the exact claimed token.
        let cancelled = shared
            .lock()
            .expect("service lock")
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationCancel {
                    request_id: "cancel.long-running".to_owned(),
                    owner_claim,
                    program_invocation_id: invocation_id.clone(),
                },
            )
            .expect("cancel request");
        assert!(matches!(cancelled, RuntimeResult::Cancelled { .. }));
        gate.released.store(true, Ordering::Release);
        let finished = execution.join().expect("execution thread");
        assert!(
            matches!(
                finished,
                RuntimeResult::Cancelled { .. } | RuntimeResult::Failed { .. }
            ),
            "cancellation may settle as committed cancellation or uncertainty: {finished:?}"
        );

        let guard = shared.lock().expect("service lock");
        assert!(guard.cancelled.contains_key(&invocation_id));
        assert!(!guard.active_cancellations.contains_key(&invocation_id));
        let invocation = guard
            .instances
            .get(&program_instance_id)
            .and_then(|instance| instance.invocation.as_ref())
            .expect("claimed invocation");
        assert!(matches!(
            invocation.result,
            Some(RuntimeResult::Cancelled { .. }) | Some(RuntimeResult::Failed { .. })
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
        assert_eq!(second, "invalid_artifact");
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
    fn an_expired_cancelled_instance_takes_its_marker_and_the_state_reopens() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        let invocation_id = {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let instance_id = create_started(&mut service, fixture_air_bytes());
            let claim = owner_claim(&service, &instance_id);
            let started = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationStart {
                        request_id: "expire.cancelled".to_owned(),
                        program_instance_id: instance_id.clone(),
                        owner_claim: claim.clone(),
                        input: serde_json::json!({}),
                    },
                )
                .expect("invocation starts");
            assert!(
                !matches!(started, RuntimeResult::Failed { .. }),
                "{started:?}"
            );
            let invocation_id = service.instances[&instance_id]
                .invocation
                .as_ref()
                .expect("admitted invocation")
                .program_invocation_id
                .clone();
            let cancelled = service
                .handle(
                    &handshake(),
                    RuntimeRequest::ProgramInvocationCancel {
                        request_id: "expire.cancel".to_owned(),
                        owner_claim: claim,
                        program_invocation_id: invocation_id.clone(),
                    },
                )
                .expect("cancel");
            assert!(
                matches!(cancelled, RuntimeResult::Cancelled { .. }),
                "{cancelled:?}"
            );
            assert!(service.cancelled.contains_key(&invocation_id));

            // Expire the instance only. The marker was raised to at least the
            // instance expiry when it was written, so it is still live here.
            let instance = service.instances.get_mut(&instance_id).expect("instance");
            if let Some(invocation) = instance.invocation.as_mut()
                && invocation.result.is_none()
            {
                invocation.result = Some(RuntimeResult::Cancelled {
                    request_id: "expire.cancelled".to_owned(),
                });
            }
            instance.state_entry.expires_at = Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now);
            service.cleanup_expired().expect("expiry persists");
            assert!(service.instances.is_empty());
            assert!(!service.invocation_index.contains_key(&invocation_id));
            assert!(
                !service.cancelled.contains_key(&invocation_id),
                "an expired instance takes its cancellation marker with it"
            );
            assert_eq!(service.cancellation_bytes, 0);
            invocation_id
        };

        let reopened = RuntimeService::default().with_runtime_state_dir(path);
        assert!(
            reopened.startup_error().is_none(),
            "{:?}",
            reopened.startup_error()
        );
        assert!(!reopened.cancelled.contains_key(&invocation_id));
        assert!(reopened.instances.is_empty());
    }

    #[test]
    fn a_pending_invocation_cancelled_without_a_worker_token_never_runs() {
        let mut service = RuntimeService::default();
        let instance_id = create_started(&mut service, fixture_air_bytes());
        let claim = owner_claim(&service, &instance_id);
        let prepared = service
            .prepare_invocation(
                "pending.cancel".to_owned(),
                instance_id,
                claim.clone(),
                serde_json::json!({}),
            )
            .expect("durable pending invocation");
        let invocation_id = prepared.invocation_id.clone();
        // No worker holds a token now, exactly as after a restart before
        // recovery claims the pending invocation.
        service.release_invocation_claim(&invocation_id);
        drop(prepared);
        let cancelled = service
            .handle(
                &handshake(),
                RuntimeRequest::ProgramInvocationCancel {
                    request_id: "pending.cancel.request".to_owned(),
                    owner_claim: claim,
                    program_invocation_id: invocation_id.clone(),
                },
            )
            .expect("cancel");
        assert!(
            matches!(cancelled, RuntimeResult::Cancelled { .. }),
            "{cancelled:?}"
        );

        let reclaimed = service
            .claim_pending_invocation(&invocation_id)
            .expect("claim")
            .expect("the pending invocation is still claimable");
        assert!(
            reclaimed.cancellation.is_cancelled(),
            "a claim over a durable cancellation marker carries the cancellation"
        );
        assert!(service.begin_invocation(&invocation_id).expect("begin"));
        let execution = reclaimed.execute();
        let result = service.finish_invocation(&reclaimed, execution);
        assert!(
            matches!(result, RuntimeResult::Cancelled { .. }),
            "a cancelled pending invocation settles as cancelled: {result:?}"
        );
    }

    #[test]
    fn an_unexplained_cancellation_marker_still_refuses_reopen() {
        let directory = tempfile::tempdir().expect("runtime state directory");
        let path = directory.path().to_path_buf();
        {
            let mut service = RuntimeService::default().with_runtime_state_dir(path.clone());
            let _instance = create_started(&mut service, fixture_air_bytes());
            let orphan = "pi-unexplained:inv-orphan".to_owned();
            let marker_bytes = u64::try_from(orphan.len()).unwrap_or(u64::MAX);
            service.cancelled.insert(
                orphan,
                RuntimeService::entry(marker_bytes, service.state_policy.cancellations.ttl),
            );
            service.cancellation_bytes = service.cancellation_bytes.saturating_add(marker_bytes);
            service
                .persist_runtime_state()
                .expect("persist the unexplained marker");
        }

        let reopened = RuntimeService::default().with_runtime_state_dir(path);
        match reopened.startup_error() {
            Some(RuntimeServiceStartupError::OpenRuntimeStateDir(message)) => {
                assert!(message.contains("orphan cancellation"), "{message}");
            }
            other => panic!("an orphan cancellation must refuse reopen: {other:?}"),
        }
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
