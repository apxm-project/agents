//! Product-neutral APXM reference host over JSONL stdin/stdout.
//!
//! The host owns lifecycle admission and invokes the existing canonical driver
//! only after exact release, port, resource-ceiling, and provenance checks.
//! There is no HTTP compatibility route, runtime discovery, or fallback.

use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use apxm_program::air::AirModule;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use apxm_cli::canonical_execute::execute_canonical_air;

const HOST_SCHEMA: &str = "apxm.runtime.host.v1";
const REQUEST_SCHEMA: &str = "apxm.runtime.host-request.v1";
const ADMISSION_SCHEMA: &str = "apxm.invocation-admission.v1";
const STARTUP_INPUT_SCHEMA: &str = "apxm.reference-host-startup-input.v1";
const IN_FLIGHT_DRAIN_PROBE: &str = "drain_shutdown_after_in_flight_completion";
const IN_FLIGHT_DRAIN_PROBE_SCHEMA: &str = "apxm.reference-host.lifecycle-probe.v1";
const DIGEST_PREFIX: &str = "sha256:";
const OWNER_EXECUTABLE: &str = "apxm-reference-host";
const OWNER_EXECUTABLE_PATH: &str = "crates/tools/cli/src/bin/reference_host.rs";
const RELEASE_MANIFEST_SOURCE_PATH: &str =
    "contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json";
const TRANSPORT_PROTOCOL: &str = "jsonl-stdin-stdout";
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    schema_version: String,
    invocation_id: String,
    artifact_digest: String,
    release_digest: String,
    port_bindings_digest: String,
    resource_ceiling_digest: String,
    provenance_digest: String,
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

#[derive(Clone, Debug, Deserialize)]
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
}

struct Host {
    state: State,
    in_flight: u64,
    admission_revoked: bool,
    last_runtime_evidence: Option<Value>,
    admitted_digests: AdmittedDigests,
}

struct PreparedInvocation {
    invocation_id: String,
    air: AirModule,
}

impl Host {
    fn new(admitted_digests: AdmittedDigests) -> Self {
        Self {
            state: State::Starting,
            in_flight: 0,
            admission_revoked: false,
            last_runtime_evidence: None,
            admitted_digests,
        }
    }

    fn mark_ready(&mut self) {
        if self.state == State::Starting {
            self.state = State::Ready;
        }
    }

    fn readiness(&self) -> Value {
        json!({
            "schema_version": HOST_SCHEMA,
            "contract_id": HOST_SCHEMA,
            "state": self.state.as_str(),
            "release_digest": self.admitted_digests.release_digest.clone(),
            "port_bindings_digest": self.admitted_digests.port_bindings_digest.clone(),
            "resource_ceiling_digest": self.admitted_digests.resource_ceiling_digest.clone(),
            "in_flight": self.in_flight,
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
        if admission.schema_version != ADMISSION_SCHEMA {
            return Err(self.reject(
                "invalid_admission_schema",
                "exact apxm.invocation-admission.v1 is required",
            ));
        }
        if admission.invocation_id.is_empty() {
            return Err(self.reject("missing_invocation_id", "invocation_id is required"));
        }
        if admission.artifact_digest != self.admitted_digests.release_digest
            || admission.provenance_digest != admission.artifact_digest
        {
            return Err(self.reject(
                "provenance_mismatch",
                "artifact and provenance digests must match the admitted release",
            ));
        }
        if admission.release_digest != self.admitted_digests.release_digest {
            return Err(self.reject("release_mismatch", "release digest is not admitted"));
        }
        if admission.port_bindings_digest != self.admitted_digests.port_bindings_digest {
            return Err(self.reject(
                "port_bindings_mismatch",
                "port binding digest is not admitted",
            ));
        }
        if admission.resource_ceiling_digest != self.admitted_digests.resource_ceiling_digest {
            return Err(self.reject(
                "resource_ceiling_mismatch",
                "resource ceiling digest is not admitted",
            ));
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
            return Err(self.reject("missing_air", "canonical apxm.air.v1 is required"));
        };
        let air: AirModule = match serde_json::from_value(air_value) {
            Ok(air) => air,
            Err(error) => return Err(self.reject("invalid_air", error.to_string())),
        };
        if !air.verify().is_accepted() {
            return Err(self.reject("invalid_air", "canonical AIR verification failed"));
        }
        self.in_flight = 1;
        Ok(PreparedInvocation {
            invocation_id: admission.invocation_id,
            air,
        })
    }

    fn complete_invocation(
        &mut self,
        invocation_id: String,
        execution: std::result::Result<Value, String>,
    ) -> Value {
        let result = match execution {
            Ok(output) => {
                let runtime_evidence =
                    self.runtime_evidence(&invocation_id, "invocation.committed");
                self.last_runtime_evidence = Some(runtime_evidence.clone());
                json!({
                    "schema_version": HOST_SCHEMA,
                    "status": "committed",
                    "invocation_id": invocation_id,
                    "result": output,
                    "runtime_evidence": runtime_evidence,
                })
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
        result
    }

    async fn invoke(&mut self, request: Request) -> Value {
        let prepared = match self.prepare_invocation(request) {
            Ok(prepared) => prepared,
            Err(rejection) => return rejection,
        };
        let execution = execute_canonical_air(prepared.air)
            .await
            .map_err(|error| error.to_string());
        self.complete_invocation(prepared.invocation_id, execution)
    }

    async fn probe_in_flight_drain(&mut self) -> Result<Value> {
        let probe_invocation_id = "probe.invocation.1";
        let prepared = self
            .prepare_invocation(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(Admission {
                    schema_version: ADMISSION_SCHEMA.into(),
                    invocation_id: probe_invocation_id.into(),
                    artifact_digest: self.admitted_digests.release_digest.clone(),
                    release_digest: self.admitted_digests.release_digest.clone(),
                    port_bindings_digest: self.admitted_digests.port_bindings_digest.clone(),
                    resource_ceiling_digest: self.admitted_digests.resource_ceiling_digest.clone(),
                    provenance_digest: self.admitted_digests.release_digest.clone(),
                }),
                air: Some(json!({
                    "schema_version": "apxm.air.v1",
                    "semantic_operations": [],
                    "structural_ir": [],
                    "context_flow": [],
                    "source_map": {
                        "schema_version": "apxm.source-map.v1",
                        "source_language": "python",
                        "node_spans": [],
                        "region_annotations": []
                    }
                })),
            })
            .map_err(|rejection| anyhow!("{rejection}"))?;
        let in_flight_before_drain = self.readiness();
        let drain_response = self.handle_non_invoke(Request {
            schema_version: REQUEST_SCHEMA.into(),
            operation: "drain".into(),
            admission: None,
            air: None,
        });
        let execution = execute_canonical_air(prepared.air)
            .await
            .map_err(|error| error.to_string());
        let completion_response = self.complete_invocation(prepared.invocation_id, execution);
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
            "readiness" => self.readiness(),
            "drain" if self.state == State::Ready => {
                self.state = State::Draining;
                self.readiness()
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

    async fn handle(&mut self, request: Request) -> Value {
        if request.schema_version != REQUEST_SCHEMA {
            return self.reject(
                "invalid_request_schema",
                "exact host request schema is required",
            );
        }
        if request.operation == "invoke" {
            self.invoke(request).await
        } else {
            self.handle_non_invoke(request)
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
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(path).with_context(|| format!("read {}", path.display()))?)
    ))
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

fn load_startup_input(path: &Path) -> Result<AdmittedDigests> {
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
    if startup.transport_protocol != TRANSPORT_PROTOCOL {
        bail!("startup input transport_protocol must remain jsonl-stdin-stdout");
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
    let actual_manifest_digest = file_digest(&manifest_path).with_context(|| {
        format!(
            "load reference-host release manifest {}",
            manifest_path.display()
        )
    })?;
    if actual_manifest_digest != startup.reference_host_release_manifest.digest {
        bail!(
            "reference-host release manifest digest mismatch: expected {}, got {}",
            startup.reference_host_release_manifest.digest,
            actual_manifest_digest
        );
    }

    Ok(AdmittedDigests {
        release_digest: startup.release_digest,
        port_bindings_digest: startup.port_bindings_digest,
        resource_ceiling_digest: startup.resource_ceiling_digest,
    })
}

fn startup_input_path() -> Result<(PathBuf, Option<String>)> {
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
    let probe = match (args.next(), args.next()) {
        (None, None) => None,
        (Some(flag), Some(case)) if flag == "--lifecycle-probe" => Some(case),
        (Some(other), _) => return Err(anyhow!("unexpected argument {other:?}")),
        (None, Some(_)) => return Err(anyhow!("unexpected trailing argument")),
    };
    Ok((PathBuf::from(path), probe))
}

#[tokio::main]
async fn main() -> Result<()> {
    let (startup_input, probe) = startup_input_path()?;
    let admitted_digests = load_startup_input(&startup_input)?;
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
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => host.handle(request).await,
            Err(error) => host.reject("invalid_request", error.to_string()),
        };
        println!("{}", serde_json::to_string(&response)?);
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
        AdmittedDigests {
            release_digest: exact_digest("release"),
            port_bindings_digest: exact_digest("port-bindings"),
            resource_ceiling_digest: exact_digest("resource-ceiling"),
        }
    }

    fn admission(digests: &AdmittedDigests) -> Admission {
        Admission {
            schema_version: ADMISSION_SCHEMA.into(),
            invocation_id: "invocation.1".into(),
            artifact_digest: digests.release_digest.clone(),
            release_digest: digests.release_digest.clone(),
            port_bindings_digest: digests.port_bindings_digest.clone(),
            resource_ceiling_digest: digests.resource_ceiling_digest.clone(),
            provenance_digest: digests.release_digest.clone(),
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
            "schema_version": "apxm.air.v1",
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
        assert_eq!(host.handle(request("readiness")).await["state"], "starting");
        host.mark_ready();
        assert_eq!(host.handle(request("readiness")).await["state"], "ready");
        assert_eq!(host.handle(request("drain")).await["state"], "draining");
        assert_eq!(host.handle(request("readiness")).await["state"], "draining");
    }

    #[tokio::test]
    async fn cancellation_before_admission_is_terminal_and_recoverable_by_restart() {
        let mut host = Host::new(startup_digests());
        host.mark_ready();
        let response = host.handle(request("cancel")).await;
        assert_eq!(response["status"], "cancelled");
        assert_eq!(response["readiness"]["state"], "stopped");
        assert_eq!(host.handle(request("readiness")).await["state"], "stopped");
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
                    "schema_version": HOST_SCHEMA,
                    "contract_id": HOST_SCHEMA,
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
                    "schema_version": HOST_SCHEMA,
                    "contract_id": HOST_SCHEMA,
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
                    "schema_version": HOST_SCHEMA,
                    "contract_id": HOST_SCHEMA,
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
        assert_eq!(host.handle(request("drain")).await["state"], "draining");

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
                    "schema_version": HOST_SCHEMA,
                    "contract_id": HOST_SCHEMA,
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
        assert_eq!(probe["drain_response"]["state"], "draining");
        assert_eq!(probe["drain_response"]["in_flight"], 1);
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

        let error = load_startup_input(&startup_path).expect_err("placeholder digest must fail");
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

        let error = load_startup_input(&startup_path).expect_err("dirty startup input must fail");
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

        let admitted = load_startup_input(&startup_path).expect("relocated startup input");
        assert_eq!(admitted.release_digest, exact_digest("release"));
        assert_eq!(admitted.port_bindings_digest, exact_digest("port-bindings"));
        assert_eq!(
            admitted.resource_ceiling_digest,
            exact_digest("resource-ceiling")
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

        let error =
            load_startup_input(&startup_path).expect_err("mismatched manifest digest must fail");
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

        let error = load_startup_input(&startup_path).expect_err("missing manifest path must fail");
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

        let error = load_startup_input(&startup_path).expect_err("noncanonical path must fail");
        assert!(
            error
                .to_string()
                .contains("reference-host release manifest path mismatch"),
            "unexpected error: {error}"
        );
    }
}
