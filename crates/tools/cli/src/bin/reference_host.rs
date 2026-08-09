//! Product-neutral APXM reference host over JSONL stdin/stdout.
//!
//! The host owns lifecycle admission and invokes the existing canonical driver
//! only after exact release, port, resource-ceiling, and provenance checks.
//! There is no HTTP compatibility route, runtime discovery, or fallback.

use std::fs;
use std::io::{BufRead, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use apxm_program::air::AirModule;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixListener;

use apxm_cli::canonical_execute::{
    CanonicalRuntime, reference_host_port_bindings_digest, reference_host_resource_ceiling_digest,
    reference_host_runtime_descriptor,
};
use apxm_kernel::{INVOCATION_ADMISSION_SCHEMA, InvocationAdmission, verify_invocation_admission};

const HOST_SCHEMA: &str = "apxm.runtime.host-response.v1";
const READINESS_SCHEMA: &str = "apxm.runtime.host.v1";
const REQUEST_SCHEMA: &str = "apxm.runtime.host-request.v1";
const ADMISSION_SCHEMA: &str = INVOCATION_ADMISSION_SCHEMA;
const STARTUP_INPUT_SCHEMA: &str = "apxm.reference-host-startup-input.v1";
const IN_FLIGHT_DRAIN_PROBE: &str = "drain_shutdown_after_in_flight_completion";
const IN_FLIGHT_DRAIN_PROBE_SCHEMA: &str = "apxm.reference-host.lifecycle-probe.v1";
const DIGEST_PREFIX: &str = "sha256:";
const OWNER_EXECUTABLE: &str = "apxm-reference-host";
const OWNER_EXECUTABLE_PATH: &str = "crates/tools/cli/src/bin/reference_host.rs";
const RELEASE_MANIFEST_SOURCE_PATH: &str =
    "contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json";
const TRANSPORT_PROTOCOL: &str = "jsonl-stdin-stdout";
const PRIVATE_TRANSPORT_PROTOCOL: &str = "jsonl-unix-stream";
const REPLAY_JOURNAL_SUFFIX: &str = ".replay.jsonl";
const FAIL_CLOSED_ON: [&str; 5] = [
    "missing",
    "placeholder",
    "dirty",
    "mismatched",
    "implicit-default",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Starting,
    Ready,
    Draining,
    Stopped,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema_version: String,
    operation: String,
    #[serde(default)]
    admission: Option<Admission>,
    #[serde(default)]
    air: Option<Value>,
}

type Admission = InvocationAdmission;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReplayRecord {
    invocation_id: String,
    response: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartupInput {
    schema_version: String,
    semantic_owner: String,
    owner_executable: String,
    owner_executable_path: String,
    transport_protocol: String,
    reference_host_release_manifest: StartupManifestRef,
    release_digest: String,
    port_bindings_digest: String,
    resource_ceiling_digest: String,
    provenance: StartupInputProvenance,
    fail_closed_on: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartupManifestRef {
    path: String,
    digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StartupInputProvenance {
    owner_revision: String,
    descriptor_semantic_digest: String,
    descriptor_exact_checksum: String,
    dirty: bool,
}

#[derive(Clone, Debug)]
struct AdmittedDigests {
    release_digest: String,
    port_bindings_digest: String,
    resource_ceiling_digest: String,
    release_bytes: Vec<u8>,
    provenance_bytes: Vec<u8>,
    provenance_digest: String,
}

struct ReplayJournal {
    file: fs::File,
}

impl ReplayJournal {
    fn open(
        path: &Path,
    ) -> Result<(
        Self,
        std::collections::BTreeMap<String, Value>,
        Option<Value>,
    )> {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if !metadata.file_type().is_file() {
                bail!("replay journal must be a regular file: {}", path.display());
            }
            #[cfg(unix)]
            if metadata.mode() & 0o777 != 0o600 {
                bail!("replay journal must be owner-only: {}", path.display());
            }
        }

        let mut options = fs::OpenOptions::new();
        options.create(true).read(true).append(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(path)
            .with_context(|| format!("open replay journal {}", path.display()))?;
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;

        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_end(&mut bytes)?;
        let complete_len = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let mut replayed = std::collections::BTreeMap::new();
        let mut last_runtime_evidence = None;
        let mut start = 0;
        while start < complete_len {
            let end = bytes[start..complete_len]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(complete_len, |index| start + index);
            let record: ReplayRecord = serde_json::from_slice(&bytes[start..end])
                .with_context(|| format!("recover replay journal {}", path.display()))?;
            if record.response["status"] != "committed"
                || record.response["invocation_id"] != record.invocation_id
            {
                bail!("replay journal contains an invalid invocation receipt");
            }
            if let Some(previous) = replayed.get(&record.invocation_id) {
                if previous != &record.response {
                    bail!("replay journal contains a conflicting invocation receipt");
                }
            } else {
                last_runtime_evidence = record.response.get("runtime_evidence").cloned();
                replayed.insert(record.invocation_id, record.response);
            }
            start = end + 1;
        }

        // A process can die after writing only part of its final line. Recover
        // all complete receipts and discard that torn tail before appending.
        if complete_len != bytes.len() {
            file.set_len(complete_len as u64)?;
        }
        file.seek(SeekFrom::End(0))?;
        Ok((Self { file }, replayed, last_runtime_evidence))
    }

    fn append(&mut self, invocation_id: &str, response: &Value) -> Result<()> {
        let record = ReplayRecord {
            invocation_id: invocation_id.to_owned(),
            response: response.clone(),
        };
        serde_json::to_writer(&mut self.file, &record)?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.file.sync_data()?;
        Ok(())
    }
}

struct Host {
    state: State,
    in_flight: u64,
    admission_revoked: bool,
    last_runtime_evidence: Option<Value>,
    admitted_digests: AdmittedDigests,
    runtime: CanonicalRuntime,
    replayed_invocations: std::collections::BTreeMap<String, Value>,
    replay_journal: Option<ReplayJournal>,
}

struct PreparedInvocation {
    admission: Admission,
    air: AirModule,
}

fn admission_error_code(error: &apxm_kernel::InvocationAdmissionError) -> &'static str {
    match error {
        apxm_kernel::InvocationAdmissionError::SchemaMismatch(_) => "invalid_admission_schema",
        apxm_kernel::InvocationAdmissionError::InvalidInvocationId => "invalid_invocation_id",
        apxm_kernel::InvocationAdmissionError::MalformedDigest(_) => "invalid_admission_digest",
        apxm_kernel::InvocationAdmissionError::ArtifactMismatch { .. } => "artifact_mismatch",
        apxm_kernel::InvocationAdmissionError::ReleaseMismatch { .. } => "release_mismatch",
        apxm_kernel::InvocationAdmissionError::ProvenanceMismatch { .. } => "provenance_mismatch",
        apxm_kernel::InvocationAdmissionError::PortBindingsDigestMismatch { .. } => {
            "port_bindings_mismatch"
        }
        apxm_kernel::InvocationAdmissionError::ResourceCeilingDigestMismatch { .. } => {
            "resource_ceiling_mismatch"
        }
        apxm_kernel::InvocationAdmissionError::ConfinementUnavailable => "confinement_unavailable",
        apxm_kernel::InvocationAdmissionError::UnconfinedForbidden => "unconfined_forbidden",
        apxm_kernel::InvocationAdmissionError::InvalidRuntimeDescriptor(_) => {
            "invalid_runtime_descriptor"
        }
    }
}

impl Host {
    fn new(admitted_digests: AdmittedDigests) -> Self {
        Self {
            state: State::Starting,
            in_flight: 0,
            admission_revoked: false,
            last_runtime_evidence: None,
            admitted_digests,
            runtime: CanonicalRuntime::new(),
            replayed_invocations: std::collections::BTreeMap::new(),
            replay_journal: None,
        }
    }

    fn mark_ready(&mut self) {
        if self.state == State::Starting {
            self.state = State::Ready;
        }
    }

    fn readiness(&self) -> Value {
        json!({
            "schema_version": READINESS_SCHEMA,
            "contract_id": READINESS_SCHEMA,
            "state": self.state.as_str(),
            "release_digest": self.admitted_digests.release_digest.clone(),
            "port_bindings_digest": self.admitted_digests.port_bindings_digest.clone(),
            "resource_ceiling_digest": self.admitted_digests.resource_ceiling_digest.clone(),
            "in_flight": self.in_flight,
        })
    }

    fn readiness_response(&self) -> Value {
        json!({
            "schema_version": HOST_SCHEMA,
            "status": "readiness",
            "readiness": self.readiness(),
        })
    }

    fn reject(&self, code: &'static str, message: impl Into<String>) -> Value {
        json!({
            "schema_version": HOST_SCHEMA,
            "status": "rejected",
            "error": {"code": code, "message": message.into()},
            "readiness": self.readiness(),
        })
    }

    fn terminal(&mut self, status: &'static str, reason: &'static str) -> Value {
        self.state = State::Stopped;
        self.in_flight = 0;
        json!({
            "schema_version": HOST_SCHEMA,
            "status": status,
            "reason": reason,
            "readiness": self.readiness(),
        })
    }

    fn restart(&mut self) -> Value {
        self.state = State::Ready;
        self.in_flight = 0;
        self.admission_revoked = false;
        json!({
            "schema_version": HOST_SCHEMA,
            "status": "restarted",
            "recovery": {
                "contract": "reconcile_from_runtime_evidence",
                "status": if self.last_runtime_evidence.is_some() {
                    "reconciled"
                } else {
                    "no_runtime_evidence"
                },
                "runtime_evidence": self.last_runtime_evidence.clone(),
            },
            "readiness": self.readiness(),
        })
    }

    fn runtime_evidence(&self, invocation_id: &str, terminal_kind: &str) -> Value {
        json!({
            "schema_version": "apxm.runtime-evidence.v1",
            "program_identity": {
                "artifact_digest": self.admitted_digests.release_digest.clone(),
                "entrypoint": "reference-host",
                "agent_identity_binding": "apxm-reference-host",
                "program_instance_id": "reference-host.instance"
            },
            "facts": [
                {
                    "fact_id": format!("{invocation_id}.created"),
                    "event_sequence": 0,
                    "fact_kind": "instance.created",
                    "instance_state": "ready"
                },
                {
                    "fact_id": format!("{invocation_id}.admitted"),
                    "event_sequence": 1,
                    "fact_kind": "invocation.admitted",
                    "invocation_state": "running"
                },
                {
                    "fact_id": format!("{invocation_id}.terminal"),
                    "event_sequence": 2,
                    "fact_kind": terminal_kind,
                    "invocation_state": "committed_return",
                    "commit_sequence": 1
                }
            ]
        })
    }

    fn validate_admission(&self, admission: &Admission) -> std::result::Result<(), Value> {
        if self.admission_revoked {
            return Err(self.reject(
                "admission_revoked",
                "invocation admission is closed until an explicit restart",
            ));
        }
        if self.state != State::Ready {
            return Err(self.reject(
                "host_not_accepting",
                format!("host state is {}", self.state.as_str()),
            ));
        }
        if let Err(error) = admission.verify_against(
            &self.admitted_digests.release_digest,
            &self.admitted_digests.port_bindings_digest,
            &self.admitted_digests.resource_ceiling_digest,
        ) {
            return Err(self.reject(admission_error_code(&error), error.to_string()));
        }
        Ok(())
    }

    fn prepare_invocation(
        &mut self,
        request: Request,
    ) -> std::result::Result<PreparedInvocation, Value> {
        let Request { admission, air, .. } = request;
        let Some(admission) = admission else {
            return Err(self.reject("missing_admission", "invocation admission is required"));
        };
        if let Err(rejection) = self.validate_admission(&admission) {
            return Err(rejection);
        }
        let Some(air_value) = air else {
            return Err(self.reject("missing_air", "canonical apxm.air.v2 is required"));
        };
        let air: AirModule = match serde_json::from_value(air_value) {
            Ok(air) => air,
            Err(error) => return Err(self.reject("invalid_air", error.to_string())),
        };
        if !air.verify().is_accepted() {
            return Err(self.reject("invalid_air", "canonical AIR verification failed"));
        }
        let descriptor = reference_host_runtime_descriptor();
        let artifact_bytes = serde_json::to_vec(&air)
            .map_err(|error| self.reject("invalid_air", error.to_string()))?;
        verify_invocation_admission(
            &admission,
            &artifact_bytes,
            &self.admitted_digests.release_bytes,
            &self.admitted_digests.provenance_bytes,
            &descriptor.port_bindings,
            descriptor.resource_ceilings,
            &descriptor.confinement,
        )
        .map_err(|error| self.reject(admission_error_code(&error), error.to_string()))?;
        Ok(PreparedInvocation { admission, air })
    }

    fn complete_invocation(
        &mut self,
        invocation_id: String,
        artifact_digest: String,
        execution: std::result::Result<Value, String>,
    ) -> Result<Value> {
        let result = match execution {
            Ok(output) => {
                let runtime_evidence =
                    self.runtime_evidence(&invocation_id, "invocation.committed");
                let mut runtime_evidence = runtime_evidence;
                runtime_evidence["program_identity"]["artifact_digest"] =
                    Value::String(artifact_digest);
                let response = json!({
                    "schema_version": HOST_SCHEMA,
                    "status": "committed",
                    "invocation_id": invocation_id.clone(),
                    "result": output,
                    "runtime_evidence": runtime_evidence,
                });
                if let Some(journal) = self.replay_journal.as_mut() {
                    journal.append(&invocation_id, &response)?;
                }
                self.last_runtime_evidence = Some(runtime_evidence.clone());
                self.replayed_invocations
                    .insert(invocation_id, response.clone());
                response
            }
            Err(error) => json!({
                "schema_version": HOST_SCHEMA,
                "status": "failed",
                "invocation_id": invocation_id,
                "error": {"code": "canonical_execution_failed", "message": error},
            }),
        };
        self.in_flight = 0;
        if self.state == State::Draining {
            self.state = State::Stopped;
        }
        Ok(result)
    }

    async fn invoke(&mut self, request: Request) -> Result<Value> {
        let prepared = match self.prepare_invocation(request) {
            Ok(prepared) => prepared,
            Err(rejection) => return Ok(rejection),
        };
        let invocation_id = prepared.admission.invocation_id.clone();
        let artifact_digest = prepared.admission.artifact_digest.clone();
        if let Some(response) = self.replayed_invocations.get(&invocation_id) {
            return Ok(response.clone());
        }
        self.in_flight = 1;
        let execution = self
            .runtime
            .execute(
                prepared.air,
                &prepared.admission,
                &self.admitted_digests.release_bytes,
                &self.admitted_digests.provenance_bytes,
            )
            .await
            .map_err(|error| error.to_string());
        self.complete_invocation(invocation_id, artifact_digest, execution)
    }

    async fn probe_in_flight_drain(&mut self) -> Result<Value> {
        let probe_invocation_id = "probe.invocation.1";
        let probe_air = json!({
            "schema_version": "apxm.air.v2",
            "semantic_operations": [],
            "structural_ir": [],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        });
        let probe_artifact_digest = serialized_air_digest(&probe_air)?;
        let prepared = self
            .prepare_invocation(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(Admission {
                    schema_version: ADMISSION_SCHEMA.into(),
                    invocation_id: probe_invocation_id.into(),
                    artifact_digest: probe_artifact_digest,
                    release_digest: self.admitted_digests.release_digest.clone(),
                    port_bindings_digest: self.admitted_digests.port_bindings_digest.clone(),
                    resource_ceiling_digest: self.admitted_digests.resource_ceiling_digest.clone(),
                    provenance_digest: self.admitted_digests.provenance_digest.clone(),
                }),
                air: Some(probe_air),
            })
            .map_err(|rejection| anyhow!("{rejection}"))?;
        self.in_flight = 1;
        let in_flight_before_drain = self.readiness();
        let drain_response = self.handle_non_invoke(Request {
            schema_version: REQUEST_SCHEMA.into(),
            operation: "drain".into(),
            admission: None,
            air: None,
        });
        let invocation_id = prepared.admission.invocation_id.clone();
        let artifact_digest = prepared.admission.artifact_digest.clone();
        let execution = self
            .runtime
            .execute(
                prepared.air,
                &prepared.admission,
                &self.admitted_digests.release_bytes,
                &self.admitted_digests.provenance_bytes,
            )
            .await
            .map_err(|error| error.to_string());
        let completion_response =
            self.complete_invocation(invocation_id, artifact_digest, execution)?;
        let terminal_readiness = self.readiness();
        Ok(json!({
            "schema_version": IN_FLIGHT_DRAIN_PROBE_SCHEMA,
            "semantic_owner": "agents",
            "case": IN_FLIGHT_DRAIN_PROBE,
            "transition": "stop_admission_then_finish_in_flight",
            "in_flight_before_drain": in_flight_before_drain,
            "drain_response": drain_response,
            "completion_response": completion_response,
            "terminal_readiness": terminal_readiness,
        }))
    }

    fn handle_non_invoke(&mut self, request: Request) -> Value {
        match request.operation.as_str() {
            "readiness" => self.readiness_response(),
            "drain" if self.state == State::Ready => {
                self.state = State::Draining;
                self.readiness_response()
            }
            "drain" => self.reject("invalid_transition", "host is not ready to drain"),
            "cancel" if self.state == State::Ready && self.in_flight == 0 => {
                self.terminal("cancelled", "cancelled_before_admission")
            }
            "cancel" => self.reject(
                "cancellation_unconfirmed",
                "in-flight cancellation requires durable host evidence",
            ),
            "shutdown" if self.in_flight == 0 && self.state != State::Stopped => {
                self.terminal("shutdown", "shutdown_terminal")
            }
            "shutdown" if self.state == State::Stopped => {
                self.reject("invalid_transition", "host is already stopped")
            }
            "shutdown" => self.reject(
                "shutdown_unconfirmed",
                "in-flight shutdown requires durable host evidence",
            ),
            "revoke" if self.in_flight == 0 && self.state != State::Stopped => {
                self.admission_revoked = true;
                self.terminal("revoked", "admission_revoked")
            }
            "revoke" if self.state == State::Stopped && self.admission_revoked => {
                self.reject("invalid_transition", "host admission is already revoked")
            }
            "revoke" if self.state == State::Stopped => {
                self.reject("invalid_transition", "host is already stopped")
            }
            "revoke" => self.reject(
                "revocation_unconfirmed",
                "in-flight revocation requires durable host evidence",
            ),
            "restart" if self.state == State::Stopped && self.in_flight == 0 => self.restart(),
            "restart" => self.reject(
                "invalid_transition",
                "host restart requires a stopped terminal state",
            ),
            _ => self.reject(
                "unknown_operation",
                "operation must be readiness, invoke, drain, cancel, shutdown, revoke, or restart",
            ),
        }
    }

    async fn handle_result(&mut self, request: Request) -> Result<Value> {
        if request.schema_version != REQUEST_SCHEMA {
            return Ok(self.reject(
                "invalid_request_schema",
                "exact host request schema is required",
            ));
        }
        if request.operation == "invoke" {
            self.invoke(request).await
        } else {
            Ok(self.handle_non_invoke(request))
        }
    }

    async fn handle(&mut self, request: Request) -> Value {
        match self.handle_result(request).await {
            Ok(response) => response,
            Err(error) => self.reject("replay_persistence_failed", error.to_string()),
        }
    }
}

fn is_hex_digest(digest: &str) -> bool {
    let Some(hex) = digest.strip_prefix(DIGEST_PREFIX) else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_placeholder_digest(digest: &str) -> bool {
    let Some(hex) = digest.strip_prefix(DIGEST_PREFIX) else {
        return true;
    };
    if hex.len() != 64 {
        return true;
    }
    let mut chars = hex.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    chars.all(|value| value == first)
}

fn validate_exact_digest(field: &str, digest: &str) -> Result<()> {
    if !is_hex_digest(digest) {
        bail!("{field} must be an exact sha256:<64 hex> digest");
    }
    if is_placeholder_digest(digest) {
        bail!("{field} must not be a placeholder digest");
    }
    Ok(())
}

fn file_digest(path: &Path) -> Result<String> {
    Ok(bytes_digest(
        &fs::read(path).with_context(|| format!("read {}", path.display()))?,
    ))
}

fn bytes_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn serialized_air_digest(value: &Value) -> Result<String> {
    let air: AirModule = serde_json::from_value(value.clone()).context("parse canonical AIR")?;
    Ok(bytes_digest(&serde_json::to_vec(&air)?))
}

fn canonical_release_manifest_source_path() -> &'static Path {
    Path::new(RELEASE_MANIFEST_SOURCE_PATH)
}

#[cfg(test)]
fn checkout_release_manifest_path() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(RELEASE_MANIFEST_SOURCE_PATH);
    fs::canonicalize(&path).unwrap_or(path)
}

fn is_hex_revision(revision: &str) -> bool {
    revision.len() == 40
        && revision
            .chars()
            .all(|value| value.is_ascii_digit() || ('a'..='f').contains(&value))
}

fn load_startup_input(path: &Path, expected_transport: &str) -> Result<AdmittedDigests> {
    let startup: StartupInput = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read startup input {}", path.display()))?,
    )
    .with_context(|| format!("parse startup input {}", path.display()))?;
    if startup.schema_version != STARTUP_INPUT_SCHEMA {
        bail!("startup input must use schema_version {STARTUP_INPUT_SCHEMA}");
    }
    if startup.semantic_owner != "agents" {
        bail!("startup input semantic_owner must be agents");
    }
    if startup.owner_executable != OWNER_EXECUTABLE {
        bail!("startup input owner_executable must be the canonical reference host");
    }
    if startup.owner_executable_path != OWNER_EXECUTABLE_PATH {
        bail!("startup input owner_executable_path must be the canonical source path");
    }
    if startup.transport_protocol != expected_transport {
        bail!(
            "startup input transport_protocol must match the selected transport: expected {}, got {}",
            expected_transport,
            startup.transport_protocol
        );
    }
    if !startup
        .fail_closed_on
        .iter()
        .map(String::as_str)
        .eq(FAIL_CLOSED_ON.iter().copied())
    {
        bail!("startup input fail_closed_on must publish the exact fail-closed reasons");
    }

    validate_exact_digest(
        "reference_host_release_manifest.digest",
        &startup.reference_host_release_manifest.digest,
    )?;
    validate_exact_digest("release_digest", &startup.release_digest)?;
    validate_exact_digest("port_bindings_digest", &startup.port_bindings_digest)?;
    validate_exact_digest("resource_ceiling_digest", &startup.resource_ceiling_digest)?;
    if !is_hex_revision(&startup.provenance.owner_revision) {
        bail!("startup input provenance.owner_revision must be a lowercase 40-hex revision");
    }
    validate_exact_digest(
        "provenance.descriptor_semantic_digest",
        &startup.provenance.descriptor_semantic_digest,
    )?;
    validate_exact_digest(
        "provenance.descriptor_exact_checksum",
        &startup.provenance.descriptor_exact_checksum,
    )?;
    if startup.provenance.dirty {
        bail!("startup input provenance must prove a clean owner checkout");
    }

    let manifest_path =
        fs::canonicalize(PathBuf::from(&startup.reference_host_release_manifest.path))
            .unwrap_or_else(|_| PathBuf::from(&startup.reference_host_release_manifest.path));
    if !manifest_path.ends_with(canonical_release_manifest_source_path()) {
        bail!(
            "reference-host release manifest path mismatch: expected suffix {}, got {}",
            canonical_release_manifest_source_path().display(),
            manifest_path.display()
        );
    }
    let release_bytes = fs::read(&manifest_path).with_context(|| {
        format!(
            "load reference-host release manifest {}",
            manifest_path.display()
        )
    })?;
    let actual_manifest_digest = format!("sha256:{:x}", Sha256::digest(&release_bytes));
    if actual_manifest_digest != startup.reference_host_release_manifest.digest {
        bail!(
            "reference-host release manifest digest mismatch: expected {}, got {}",
            startup.reference_host_release_manifest.digest,
            actual_manifest_digest
        );
    }
    if startup.release_digest != actual_manifest_digest {
        bail!(
            "release digest must match the exact reference-host release manifest: expected {}, got {}",
            actual_manifest_digest,
            startup.release_digest
        );
    }
    let expected_port_bindings_digest = reference_host_port_bindings_digest();
    if startup.port_bindings_digest != expected_port_bindings_digest {
        bail!(
            "startup port bindings digest is not the exact APXM reference-host binding set: expected {}, got {}",
            expected_port_bindings_digest,
            startup.port_bindings_digest
        );
    }
    let expected_resource_ceiling_digest = reference_host_resource_ceiling_digest();
    if startup.resource_ceiling_digest != expected_resource_ceiling_digest {
        bail!(
            "startup resource ceiling digest is not the exact APXM reference-host ceiling set: expected {}, got {}",
            expected_resource_ceiling_digest,
            startup.resource_ceiling_digest
        );
    }
    let provenance_bytes = serde_json::to_vec(&startup.provenance)?;
    let provenance_digest = bytes_digest(&provenance_bytes);

    Ok(AdmittedDigests {
        release_digest: startup.release_digest,
        port_bindings_digest: startup.port_bindings_digest,
        resource_ceiling_digest: startup.resource_ceiling_digest,
        release_bytes,
        provenance_bytes,
        provenance_digest,
    })
}

fn startup_input_path() -> Result<(PathBuf, Option<String>, Option<PathBuf>)> {
    let mut args = std::env::args().skip(1);
    let Some(flag) = args.next() else {
        return Err(anyhow!(
            "missing required startup input; usage: apxm-reference-host --startup-input <path>"
        ));
    };
    if flag != "--startup-input" {
        return Err(anyhow!(
            "unexpected argument {flag:?}; usage: apxm-reference-host --startup-input <path>"
        ));
    }
    let Some(path) = args.next() else {
        return Err(anyhow!(
            "missing startup input path; usage: apxm-reference-host --startup-input <path>"
        ));
    };
    let mut probe = None;
    let mut unix_socket = None;
    let mut remaining = args;
    while let Some(flag) = remaining.next() {
        match flag.as_str() {
            "--lifecycle-probe" => {
                let Some(case) = remaining.next() else {
                    bail!("missing lifecycle probe case")
                };
                if probe.replace(case).is_some() {
                    bail!("lifecycle probe may be supplied only once")
                }
            }
            "--unix-socket" => {
                let Some(socket_path) = remaining.next() else {
                    bail!("missing Unix socket path")
                };
                if unix_socket.replace(PathBuf::from(socket_path)).is_some() {
                    bail!("--unix-socket may be supplied only once")
                }
            }
            other => return Err(anyhow!("unexpected argument {other:?}")),
        }
    }
    Ok((PathBuf::from(path), probe, unix_socket))
}

async fn response_for_line(host: &mut Host, line: &str) -> Result<String> {
    let response = match serde_json::from_str::<Request>(line) {
        Ok(request) => host.handle(request).await,
        Err(error) => host.reject("invalid_request", error.to_string()),
    };
    Ok(serde_json::to_string(&response)?)
}

#[cfg(unix)]
/// Returns whether a private transport peer belongs to the socket owner.
fn private_transport_peer_is_owner(socket_owner_uid: u32, peer_uid: u32) -> bool {
    peer_uid == socket_owner_uid
}

#[cfg(unix)]
async fn run_unix_socket(mut host: Host, socket_path: &Path) -> Result<()> {
    if !socket_path.is_absolute() {
        bail!("{PRIVATE_TRANSPORT_PROTOCOL} requires an absolute Unix socket path")
    }
    let parent = socket_path
        .parent()
        .ok_or_else(|| anyhow!("private transport socket must have a parent directory"))?;
    let parent_metadata = fs::metadata(parent)
        .with_context(|| format!("inspect private transport directory {}", parent.display()))?;
    if !parent_metadata.is_dir() {
        bail!(
            "private transport parent is not a directory: {}",
            parent.display()
        );
    }
    #[cfg(unix)]
    if parent_metadata.mode() & 0o022 != 0 {
        bail!(
            "private transport parent must not be group/world writable: {}",
            parent.display()
        );
    }
    if let Ok(metadata) = fs::symlink_metadata(socket_path) {
        if !metadata.file_type().is_socket() {
            bail!("refusing to replace non-socket private transport endpoint")
        }
        fs::remove_file(socket_path)
            .with_context(|| format!("remove stale Unix socket {}", socket_path.display()))?;
    }
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("bind private transport {}", socket_path.display()))?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restrict private transport {}", socket_path.display()))?;
    let socket_owner_uid = fs::metadata(socket_path)
        .with_context(|| format!("inspect private transport {}", socket_path.display()))?
        .uid();

    let journal_path = PathBuf::from(format!(
        "{}{}",
        socket_path.display(),
        REPLAY_JOURNAL_SUFFIX
    ));
    let (journal, replayed, last_runtime_evidence) = ReplayJournal::open(&journal_path)?;
    host.replay_journal = Some(journal);
    host.replayed_invocations = replayed;
    host.last_runtime_evidence = last_runtime_evidence;

    loop {
        let (stream, _) = listener.accept().await?;
        let peer_uid = stream
            .peer_cred()
            .with_context(|| "inspect private transport peer credentials")?
            .uid();
        if !private_transport_peer_is_owner(socket_owner_uid, peer_uid) {
            continue;
        }
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = BufReader::new(read_half).lines();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let response = response_for_line(&mut host, &line).await?;
            write_half.write_all(response.as_bytes()).await?;
            write_half.write_all(b"\n").await?;
            write_half.flush().await?;
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let (startup_input, probe, unix_socket) = startup_input_path()?;
    let expected_transport = if unix_socket.is_some() {
        PRIVATE_TRANSPORT_PROTOCOL
    } else {
        TRANSPORT_PROTOCOL
    };
    let admitted_digests = load_startup_input(&startup_input, expected_transport)?;
    let stdin = std::io::stdin();
    let mut host = Host::new(admitted_digests);
    host.mark_ready();
    if let Some(probe) = probe {
        if probe != IN_FLIGHT_DRAIN_PROBE {
            bail!("unsupported lifecycle probe {probe:?}");
        }
        println!(
            "{}",
            serde_json::to_string(&host.probe_in_flight_drain().await?)?
        );
        return Ok(());
    }
    if let Some(socket_path) = unix_socket {
        #[cfg(unix)]
        {
            return run_unix_socket(host, &socket_path).await;
        }
        #[cfg(not(unix))]
        {
            let _ = socket_path;
            bail!("private Unix transport is unavailable on this platform")
        }
    }
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        println!("{}", response_for_line(&mut host, &line).await?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn exact_digest(label: &str) -> String {
        format!("sha256:{:x}", Sha256::digest(label.as_bytes()))
    }

    fn placeholder_digest(hex: char) -> String {
        format!("sha256:{}", hex.to_string().repeat(64))
    }

    fn startup_digests() -> AdmittedDigests {
        let release_bytes = b"reference-release-v1".to_vec();
        let provenance_bytes = b"reference-provenance-v1".to_vec();
        AdmittedDigests {
            release_digest: bytes_digest(&release_bytes),
            port_bindings_digest: reference_host_port_bindings_digest(),
            resource_ceiling_digest: reference_host_resource_ceiling_digest(),
            provenance_digest: bytes_digest(&provenance_bytes),
            release_bytes,
            provenance_bytes,
        }
    }

    fn admission(digests: &AdmittedDigests) -> Admission {
        let air = minimal_air();
        let air_module: AirModule =
            serde_json::from_value(air).expect("minimal AIR deserialization");
        let artifact_bytes = serde_json::to_vec(&air_module).expect("minimal AIR serialization");
        Admission {
            schema_version: ADMISSION_SCHEMA.into(),
            invocation_id: "invocation.1".into(),
            artifact_digest: bytes_digest(&artifact_bytes),
            release_digest: digests.release_digest.clone(),
            port_bindings_digest: digests.port_bindings_digest.clone(),
            resource_ceiling_digest: digests.resource_ceiling_digest.clone(),
            provenance_digest: digests.provenance_digest.clone(),
        }
    }

    fn request(operation: &str) -> Request {
        Request {
            schema_version: REQUEST_SCHEMA.into(),
            operation: operation.into(),
            admission: None,
            air: None,
        }
    }

    fn minimal_air() -> Value {
        json!({
            "schema_version": "apxm.air.v2",
            "semantic_operations": [],
            "structural_ir": [],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        })
    }

    #[tokio::test]
    async fn readiness_and_drain_are_explicit() {
        let mut host = Host::new(startup_digests());
        assert_eq!(
            host.handle(request("readiness")).await["status"],
            "readiness"
        );
        assert_eq!(
            host.handle(request("readiness")).await["readiness"]["state"],
            "starting"
        );
        host.mark_ready();
        assert_eq!(
            host.handle(request("readiness")).await["readiness"]["state"],
            "ready"
        );
        assert_eq!(
            host.handle(request("drain")).await["readiness"]["state"],
            "draining"
        );
        assert_eq!(
            host.handle(request("readiness")).await["readiness"]["state"],
            "draining"
        );
    }

    #[tokio::test]
    async fn cancellation_before_admission_is_terminal_and_recoverable_by_restart() {
        let mut host = Host::new(startup_digests());
        host.mark_ready();
        let response = host.handle(request("cancel")).await;
        assert_eq!(response["status"], "cancelled");
        assert_eq!(response["readiness"]["state"], "stopped");
        assert_eq!(
            host.handle(request("readiness")).await["readiness"]["state"],
            "stopped"
        );
        let restart = host.handle(request("restart")).await;
        assert_eq!(restart["status"], "restarted");
        assert_eq!(restart["recovery"]["status"], "no_runtime_evidence");
        assert_eq!(restart["readiness"]["state"], "ready");
    }

    #[tokio::test]
    async fn in_flight_lifecycle_operations_fail_closed_until_runtime_evidence_exists() {
        let mut host = Host::new(startup_digests());
        host.mark_ready();
        host.in_flight = 1;

        let cancel = host.handle(request("cancel")).await;
        assert_eq!(
            cancel,
            json!({
                "schema_version": HOST_SCHEMA,
                "status": "rejected",
                "error": {
                    "code": "cancellation_unconfirmed",
                    "message": "in-flight cancellation requires durable host evidence",
                },
                "readiness": {
                    "schema_version": READINESS_SCHEMA,
                    "contract_id": READINESS_SCHEMA,
                    "state": "ready",
                    "release_digest": host.admitted_digests.release_digest.clone(),
                    "port_bindings_digest": host.admitted_digests.port_bindings_digest.clone(),
                    "resource_ceiling_digest": host.admitted_digests.resource_ceiling_digest.clone(),
                    "in_flight": 1,
                },
            })
        );

        let shutdown = host.handle(request("shutdown")).await;
        assert_eq!(
            shutdown,
            json!({
                "schema_version": HOST_SCHEMA,
                "status": "rejected",
                "error": {
                    "code": "shutdown_unconfirmed",
                    "message": "in-flight shutdown requires durable host evidence",
                },
                "readiness": {
                    "schema_version": READINESS_SCHEMA,
                    "contract_id": READINESS_SCHEMA,
                    "state": "ready",
                    "release_digest": host.admitted_digests.release_digest.clone(),
                    "port_bindings_digest": host.admitted_digests.port_bindings_digest.clone(),
                    "resource_ceiling_digest": host.admitted_digests.resource_ceiling_digest.clone(),
                    "in_flight": 1,
                },
            })
        );

        let revoke = host.handle(request("revoke")).await;
        assert_eq!(
            revoke,
            json!({
                "schema_version": HOST_SCHEMA,
                "status": "rejected",
                "error": {
                    "code": "revocation_unconfirmed",
                    "message": "in-flight revocation requires durable host evidence",
                },
                "readiness": {
                    "schema_version": READINESS_SCHEMA,
                    "contract_id": READINESS_SCHEMA,
                    "state": "ready",
                    "release_digest": host.admitted_digests.release_digest.clone(),
                    "port_bindings_digest": host.admitted_digests.port_bindings_digest.clone(),
                    "resource_ceiling_digest": host.admitted_digests.resource_ceiling_digest.clone(),
                    "in_flight": 1,
                },
            })
        );
    }

    #[tokio::test]
    async fn missing_or_mismatched_admission_is_rejected() {
        let mut host = Host::new(startup_digests());
        host.mark_ready();
        let response = host.handle(request("invoke")).await;
        assert_eq!(response["status"], "rejected");
        assert_eq!(response["error"]["code"], "missing_admission");
    }

    #[tokio::test]
    async fn draining_host_rejects_new_invocations_before_dispatch() {
        let digests = startup_digests();
        let mut host = Host::new(digests.clone());
        host.mark_ready();
        assert_eq!(
            host.handle(request("drain")).await["readiness"]["state"],
            "draining"
        );

        let response = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission(&digests)),
                air: Some(minimal_air()),
            })
            .await;
        assert_eq!(
            response,
            json!({
                "schema_version": HOST_SCHEMA,
                "status": "rejected",
                "error": {
                    "code": "host_not_accepting",
                    "message": "host state is draining",
                },
                "readiness": {
                    "schema_version": READINESS_SCHEMA,
                    "contract_id": READINESS_SCHEMA,
                    "state": "draining",
                    "release_digest": host.admitted_digests.release_digest.clone(),
                    "port_bindings_digest": host.admitted_digests.port_bindings_digest.clone(),
                    "resource_ceiling_digest": host.admitted_digests.resource_ceiling_digest.clone(),
                    "in_flight": 0,
                },
            })
        );
    }

    #[tokio::test]
    async fn in_flight_drain_probe_finishes_to_terminal_quiescence() {
        let mut host = Host::new(startup_digests());
        host.mark_ready();

        let probe = host
            .probe_in_flight_drain()
            .await
            .expect("in-flight drain probe");

        assert_eq!(probe["schema_version"], IN_FLIGHT_DRAIN_PROBE_SCHEMA);
        assert_eq!(probe["case"], IN_FLIGHT_DRAIN_PROBE);
        assert_eq!(probe["in_flight_before_drain"]["state"], "ready");
        assert_eq!(probe["in_flight_before_drain"]["in_flight"], 1);
        assert_eq!(probe["drain_response"]["status"], "readiness");
        assert_eq!(probe["drain_response"]["readiness"]["state"], "draining");
        assert_eq!(probe["drain_response"]["readiness"]["in_flight"], 1);
        assert_eq!(probe["completion_response"]["status"], "committed");
        assert_eq!(
            probe["completion_response"]["runtime_evidence"]["facts"][2]["fact_kind"],
            "invocation.committed"
        );
        assert_eq!(probe["terminal_readiness"]["state"], "stopped");
        assert_eq!(probe["terminal_readiness"]["in_flight"], 0);
    }

    #[tokio::test]
    async fn admitted_empty_air_reaches_the_canonical_commit_boundary() {
        let digests = startup_digests();
        let mut host = Host::new(digests.clone());
        host.mark_ready();
        let response = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission(&digests)),
                air: Some(minimal_air()),
            })
            .await;
        assert_eq!(response["status"], "committed");
        assert_eq!(response["invocation_id"], "invocation.1");
    }

    #[tokio::test]
    async fn duplicate_invocation_replays_the_same_receipt_without_a_second_commit() {
        let digests = startup_digests();
        let mut host = Host::new(digests.clone());
        host.mark_ready();
        let request = || Request {
            schema_version: REQUEST_SCHEMA.into(),
            operation: "invoke".into(),
            admission: Some(admission(&digests)),
            air: Some(minimal_air()),
        };

        let first = host.handle(request()).await;
        let replay = host.handle(request()).await;
        assert_eq!(first["status"], "committed");
        assert_eq!(replay, first, "replay must return the committed receipt");
        assert_eq!(host.in_flight, 0);
    }

    #[tokio::test]
    async fn replay_conflict_rejects_invalid_admission_before_returning_prior_commit() {
        let digests = startup_digests();
        let mut host = Host::new(digests.clone());
        host.mark_ready();

        let first = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission(&digests)),
                air: Some(minimal_air()),
            })
            .await;
        assert_eq!(first["status"], "committed");
        let committed_evidence = host.last_runtime_evidence.clone();

        let mut conflicting_admission = admission(&digests);
        conflicting_admission.provenance_digest = bytes_digest(b"conflicting-provenance");
        let response = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(conflicting_admission),
                air: Some(minimal_air()),
            })
            .await;

        assert_eq!(response["status"], "rejected");
        assert_eq!(response["error"]["code"], "provenance_mismatch");
        assert_eq!(response["readiness"]["state"], "ready");
        assert_eq!(response["readiness"]["in_flight"], 0);
        assert_eq!(host.last_runtime_evidence, committed_evidence);
    }

    #[tokio::test]
    async fn shutdown_is_terminal_until_explicit_restart() {
        let mut host = Host::new(startup_digests());
        host.mark_ready();
        let shutdown = host.handle(request("shutdown")).await;
        assert_eq!(shutdown["status"], "shutdown");
        assert_eq!(shutdown["readiness"]["state"], "stopped");

        let restarted = host.handle(request("restart")).await;
        assert_eq!(restarted["status"], "restarted");
        assert_eq!(
            restarted["recovery"]["contract"],
            "reconcile_from_runtime_evidence"
        );
        assert_eq!(restarted["readiness"]["state"], "ready");
    }

    #[tokio::test]
    async fn revocation_closes_admission_until_restart() {
        let digests = startup_digests();
        let mut host = Host::new(digests.clone());
        host.mark_ready();

        let revoke = host.handle(request("revoke")).await;
        assert_eq!(revoke["status"], "revoked");
        assert_eq!(revoke["readiness"]["state"], "stopped");

        let rejected = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission(&digests)),
                air: Some(minimal_air()),
            })
            .await;
        assert_eq!(rejected["status"], "rejected");
        assert_eq!(rejected["error"]["code"], "admission_revoked");

        let restarted = host.handle(request("restart")).await;
        assert_eq!(restarted["status"], "restarted");
        assert_eq!(restarted["readiness"]["state"], "ready");

        let committed = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission(&digests)),
                air: Some(minimal_air()),
            })
            .await;
        assert_eq!(committed["status"], "committed");
    }

    #[tokio::test]
    async fn restart_reports_recovery_from_last_runtime_evidence() {
        let digests = startup_digests();
        let mut host = Host::new(digests.clone());
        host.mark_ready();

        let committed = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission(&digests)),
                air: Some(minimal_air()),
            })
            .await;
        let runtime_evidence = committed["runtime_evidence"].clone();
        assert_eq!(committed["status"], "committed");

        let shutdown = host.handle(request("shutdown")).await;
        assert_eq!(shutdown["status"], "shutdown");

        let restarted = host.handle(request("restart")).await;
        assert_eq!(restarted["status"], "restarted");
        assert_eq!(restarted["recovery"]["status"], "reconciled");
        assert_eq!(restarted["recovery"]["runtime_evidence"], runtime_evidence);
    }

    #[test]
    fn startup_input_rejects_placeholder_digests() {
        let temp_dir = tempdir().expect("temp dir");
        let manifest_path = temp_dir.path().join("release-manifest.json");
        fs::write(&manifest_path, "{\"schema_version\":\"test\"}\n").expect("manifest");
        let startup_path = temp_dir.path().join("startup.json");
        fs::write(
            &startup_path,
            serde_json::to_string(&json!({
                "schema_version": STARTUP_INPUT_SCHEMA,
                "semantic_owner": "agents",
                "owner_executable": OWNER_EXECUTABLE,
                "owner_executable_path": OWNER_EXECUTABLE_PATH,
                "transport_protocol": TRANSPORT_PROTOCOL,
                "reference_host_release_manifest": {
                    "path": manifest_path,
                    "digest": file_digest(&manifest_path).expect("manifest digest"),
                },
                "release_digest": placeholder_digest('a'),
                "port_bindings_digest": exact_digest("port-bindings"),
                "resource_ceiling_digest": exact_digest("resource-ceiling"),
                "provenance": {
                    "owner_revision": "a".repeat(40),
                    "descriptor_semantic_digest": exact_digest("descriptor-semantic"),
                    "descriptor_exact_checksum": exact_digest("descriptor-exact"),
                    "dirty": false,
                },
                "fail_closed_on": FAIL_CLOSED_ON,
            }))
            .expect("startup json"),
        )
        .expect("startup file");

        let error = load_startup_input(&startup_path, TRANSPORT_PROTOCOL)
            .expect_err("placeholder digest must fail");
        assert!(
            error
                .to_string()
                .contains("release_digest must not be a placeholder digest"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn startup_input_rejects_dirty_provenance() {
        let temp_dir = tempdir().expect("temp dir");
        let manifest_path = checkout_release_manifest_path();
        let startup_path = temp_dir.path().join("startup.json");
        fs::write(
            &startup_path,
            serde_json::to_string(&json!({
                "schema_version": STARTUP_INPUT_SCHEMA,
                "semantic_owner": "agents",
                "owner_executable": OWNER_EXECUTABLE,
                "owner_executable_path": OWNER_EXECUTABLE_PATH,
                "transport_protocol": TRANSPORT_PROTOCOL,
                "reference_host_release_manifest": {
                    "path": manifest_path,
                    "digest": file_digest(&manifest_path).expect("manifest digest"),
                },
                "release_digest": exact_digest("release"),
                "port_bindings_digest": exact_digest("port-bindings"),
                "resource_ceiling_digest": exact_digest("resource-ceiling"),
                "provenance": {
                    "owner_revision": "a".repeat(40),
                    "descriptor_semantic_digest": exact_digest("descriptor-semantic"),
                    "descriptor_exact_checksum": exact_digest("descriptor-exact"),
                    "dirty": true,
                },
                "fail_closed_on": FAIL_CLOSED_ON,
            }))
            .expect("startup json"),
        )
        .expect("startup file");

        let error = load_startup_input(&startup_path, TRANSPORT_PROTOCOL)
            .expect_err("dirty startup input must fail");
        assert!(
            error
                .to_string()
                .contains("startup input provenance must prove a clean owner checkout"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn startup_input_accepts_relocated_release_manifest_path_with_exact_digest() {
        let temp_dir = tempdir().expect("temp dir");
        let relocated_manifest = temp_dir.path().join(RELEASE_MANIFEST_SOURCE_PATH);
        fs::create_dir_all(relocated_manifest.parent().expect("manifest parent"))
            .expect("create relocated manifest parent");
        fs::copy(checkout_release_manifest_path(), &relocated_manifest)
            .expect("copy relocated manifest");
        let startup_path = temp_dir.path().join("startup.json");
        fs::write(
            &startup_path,
            serde_json::to_string(&json!({
                "schema_version": STARTUP_INPUT_SCHEMA,
                "semantic_owner": "agents",
                "owner_executable": OWNER_EXECUTABLE,
                "owner_executable_path": OWNER_EXECUTABLE_PATH,
                "transport_protocol": TRANSPORT_PROTOCOL,
                "reference_host_release_manifest": {
                    "path": relocated_manifest,
                    "digest": file_digest(&relocated_manifest).expect("manifest digest"),
                },
                "release_digest": file_digest(&relocated_manifest).expect("manifest digest"),
                "port_bindings_digest": reference_host_port_bindings_digest(),
                "resource_ceiling_digest": reference_host_resource_ceiling_digest(),
                "provenance": {
                    "owner_revision": "a".repeat(40),
                    "descriptor_semantic_digest": exact_digest("descriptor-semantic"),
                    "descriptor_exact_checksum": exact_digest("descriptor-exact"),
                    "dirty": false,
                },
                "fail_closed_on": FAIL_CLOSED_ON,
            }))
            .expect("startup json"),
        )
        .expect("startup file");

        let admitted =
            load_startup_input(&startup_path, TRANSPORT_PROTOCOL).expect("relocated startup input");
        assert_eq!(
            admitted.release_digest,
            file_digest(&relocated_manifest).expect("manifest digest")
        );
        assert_eq!(
            admitted.port_bindings_digest,
            reference_host_port_bindings_digest()
        );
        assert_eq!(
            admitted.resource_ceiling_digest,
            reference_host_resource_ceiling_digest()
        );
    }

    #[test]
    fn startup_input_rejects_relocated_release_manifest_digest_mismatch() {
        let temp_dir = tempdir().expect("temp dir");
        let relocated_manifest = temp_dir.path().join(RELEASE_MANIFEST_SOURCE_PATH);
        fs::create_dir_all(relocated_manifest.parent().expect("manifest parent"))
            .expect("create relocated manifest parent");
        fs::copy(checkout_release_manifest_path(), &relocated_manifest)
            .expect("copy relocated manifest");
        let startup_path = temp_dir.path().join("startup.json");
        fs::write(
            &startup_path,
            serde_json::to_string(&json!({
                "schema_version": STARTUP_INPUT_SCHEMA,
                "semantic_owner": "agents",
                "owner_executable": OWNER_EXECUTABLE,
                "owner_executable_path": OWNER_EXECUTABLE_PATH,
                "transport_protocol": TRANSPORT_PROTOCOL,
                "reference_host_release_manifest": {
                    "path": relocated_manifest,
                    "digest": exact_digest("mismatch"),
                },
                "release_digest": exact_digest("release"),
                "port_bindings_digest": exact_digest("port-bindings"),
                "resource_ceiling_digest": exact_digest("resource-ceiling"),
                "provenance": {
                    "owner_revision": "a".repeat(40),
                    "descriptor_semantic_digest": exact_digest("descriptor-semantic"),
                    "descriptor_exact_checksum": exact_digest("descriptor-exact"),
                    "dirty": false,
                },
                "fail_closed_on": FAIL_CLOSED_ON,
            }))
            .expect("startup json"),
        )
        .expect("startup file");

        let error = load_startup_input(&startup_path, TRANSPORT_PROTOCOL)
            .expect_err("mismatched manifest digest must fail");
        assert!(
            error
                .to_string()
                .contains("reference-host release manifest digest mismatch"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn startup_input_rejects_missing_relocated_release_manifest_path() {
        let temp_dir = tempdir().expect("temp dir");
        let missing_manifest = temp_dir.path().join(RELEASE_MANIFEST_SOURCE_PATH);
        fs::create_dir_all(missing_manifest.parent().expect("manifest parent"))
            .expect("create relocated manifest parent");
        let startup_path = temp_dir.path().join("startup.json");
        fs::write(
            &startup_path,
            serde_json::to_string(&json!({
                "schema_version": STARTUP_INPUT_SCHEMA,
                "semantic_owner": "agents",
                "owner_executable": OWNER_EXECUTABLE,
                "owner_executable_path": OWNER_EXECUTABLE_PATH,
                "transport_protocol": TRANSPORT_PROTOCOL,
                "reference_host_release_manifest": {
                    "path": missing_manifest,
                    "digest": exact_digest("missing"),
                },
                "release_digest": exact_digest("release"),
                "port_bindings_digest": exact_digest("port-bindings"),
                "resource_ceiling_digest": exact_digest("resource-ceiling"),
                "provenance": {
                    "owner_revision": "a".repeat(40),
                    "descriptor_semantic_digest": exact_digest("descriptor-semantic"),
                    "descriptor_exact_checksum": exact_digest("descriptor-exact"),
                    "dirty": false,
                },
                "fail_closed_on": FAIL_CLOSED_ON,
            }))
            .expect("startup json"),
        )
        .expect("startup file");

        let error = load_startup_input(&startup_path, TRANSPORT_PROTOCOL)
            .expect_err("missing manifest path must fail");
        assert!(
            error
                .to_string()
                .contains("load reference-host release manifest"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn startup_input_rejects_noncanonical_release_manifest_path() {
        let temp_dir = tempdir().expect("temp dir");
        let wrong_manifest = temp_dir.path().join("release-manifest.json");
        fs::write(&wrong_manifest, "{\"schema_version\":\"test\"}\n").expect("manifest");
        let startup_path = temp_dir.path().join("startup.json");
        fs::write(
            &startup_path,
            serde_json::to_string(&json!({
                "schema_version": STARTUP_INPUT_SCHEMA,
                "semantic_owner": "agents",
                "owner_executable": OWNER_EXECUTABLE,
                "owner_executable_path": OWNER_EXECUTABLE_PATH,
                "transport_protocol": TRANSPORT_PROTOCOL,
                "reference_host_release_manifest": {
                    "path": wrong_manifest,
                    "digest": file_digest(&wrong_manifest).expect("manifest digest"),
                },
                "release_digest": exact_digest("release"),
                "port_bindings_digest": exact_digest("port-bindings"),
                "resource_ceiling_digest": exact_digest("resource-ceiling"),
                "provenance": {
                    "owner_revision": "a".repeat(40),
                    "descriptor_semantic_digest": exact_digest("descriptor-semantic"),
                    "descriptor_exact_checksum": exact_digest("descriptor-exact"),
                    "dirty": false,
                },
                "fail_closed_on": FAIL_CLOSED_ON,
            }))
            .expect("startup json"),
        )
        .expect("startup file");

        let error = load_startup_input(&startup_path, TRANSPORT_PROTOCOL)
            .expect_err("noncanonical path must fail");
        assert!(
            error
                .to_string()
                .contains("reference-host release manifest path mismatch"),
            "unexpected error: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_transport_requires_peer_uid_to_match_socket_owner() {
        assert!(private_transport_peer_is_owner(1000, 1000));
        assert!(!private_transport_peer_is_owner(1000, 1001));
    }
}
