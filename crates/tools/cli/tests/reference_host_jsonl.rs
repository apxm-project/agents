//! Live JSONL lifecycle coverage for the reference-host binary.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn exact_digest(label: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(label.as_bytes()))
}

fn release_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json")
}

fn release_manifest_digest() -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(fs::read(release_manifest_path()).expect("read release manifest"))
    )
}

fn write_startup_input() -> (TempDir, PathBuf, String, String, String) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let startup_path = temp_dir.path().join("startup-input.json");
    let release_digest = exact_digest("reference-host-release");
    let port_bindings_digest = exact_digest("reference-host-port-bindings");
    let resource_ceiling_digest = exact_digest("reference-host-resource-ceiling");
    fs::write(
        &startup_path,
        serde_json::to_string(&json!({
            "schema_version": "apxm.reference-host-startup-input.v1",
            "reference_host_release_manifest": {
                "path": release_manifest_path(),
                "digest": release_manifest_digest(),
            },
            "release_digest": release_digest,
            "port_bindings_digest": port_bindings_digest,
            "resource_ceiling_digest": resource_ceiling_digest,
        }))
        .expect("startup json"),
    )
    .expect("write startup input");
    (
        temp_dir,
        startup_path,
        release_digest,
        port_bindings_digest,
        resource_ceiling_digest,
    )
}

struct HostProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    _temp_dir: TempDir,
    release_digest: String,
    port_bindings_digest: String,
    resource_ceiling_digest: String,
}

impl HostProcess {
    fn spawn() -> Self {
        let (temp_dir, startup_path, release_digest, port_bindings_digest, resource_ceiling_digest) =
            write_startup_input();
        let mut child = Command::new(env!("CARGO_BIN_EXE_apxm-reference-host"))
            .arg("--startup-input")
            .arg(&startup_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn reference host");
        let stdin = child.stdin.take().expect("host stdin");
        let stdout = BufReader::new(child.stdout.take().expect("host stdout"));
        Self {
            child,
            stdin,
            stdout,
            _temp_dir: temp_dir,
            release_digest,
            port_bindings_digest,
            resource_ceiling_digest,
        }
    }

    fn request(&mut self, value: Value) -> Value {
        writeln!(self.stdin, "{value}").expect("write request");
        self.stdin.flush().expect("flush request");

        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read response");
        serde_json::from_str(line.trim()).expect("response json")
    }

    fn shutdown(mut self) {
        drop(self.stdin);
        let status = self.child.wait().expect("wait for host exit");
        assert!(status.success(), "host exited with {status}");
    }
}

fn request(operation: &str) -> Value {
    json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": operation,
    })
}

fn admission(host: &HostProcess) -> Value {
    json!({
        "schema_version": "apxm.invocation-admission.v1",
        "invocation_id": "invocation.1",
        "artifact_digest": host.release_digest.clone(),
        "release_digest": host.release_digest.clone(),
        "port_bindings_digest": host.port_bindings_digest.clone(),
        "resource_ceiling_digest": host.resource_ceiling_digest.clone(),
        "provenance_digest": host.release_digest.clone()
    })
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

#[test]
fn cancellation_before_admission_is_terminal_and_restartable() {
    let mut host = HostProcess::spawn();

    let cancelled = host.request(request("cancel"));
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["reason"], "cancelled_before_admission");
    assert_eq!(cancelled["readiness"]["state"], "stopped");
    assert_eq!(
        cancelled["readiness"]["release_digest"],
        host.release_digest.clone()
    );

    let restarted = host.request(request("restart"));
    assert_eq!(restarted["status"], "restarted");
    assert_eq!(restarted["recovery"]["status"], "no_runtime_evidence");
    assert_eq!(restarted["readiness"]["state"], "ready");

    host.shutdown();
}

#[test]
fn revoke_closes_admission_until_explicit_restart() {
    let mut host = HostProcess::spawn();

    assert_eq!(host.request(request("readiness"))["state"], "ready");

    let revoked = host.request(request("revoke"));
    assert_eq!(revoked["status"], "revoked");
    assert_eq!(revoked["reason"], "admission_revoked");
    assert_eq!(revoked["readiness"]["state"], "stopped");

    let rejected = host.request(json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": "invoke",
        "admission": admission(&host),
        "air": minimal_air(),
    }));
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["error"]["code"], "admission_revoked");
    assert_eq!(rejected["readiness"]["state"], "stopped");

    let restarted = host.request(request("restart"));
    assert_eq!(restarted["status"], "restarted");
    assert_eq!(restarted["readiness"]["state"], "ready");

    let committed = host.request(json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": "invoke",
        "admission": admission(&host),
        "air": minimal_air(),
    }));
    assert_eq!(committed["status"], "committed");

    host.shutdown();
}

#[test]
fn shutdown_restart_reports_runtime_evidence_recovery() {
    let mut host = HostProcess::spawn();

    let committed = host.request(json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": "invoke",
        "admission": admission(&host),
        "air": minimal_air(),
    }));
    assert_eq!(committed["status"], "committed");
    let runtime_evidence = committed["runtime_evidence"].clone();

    let shutdown = host.request(request("shutdown"));
    assert_eq!(shutdown["status"], "shutdown");
    assert_eq!(shutdown["reason"], "shutdown_terminal");
    assert_eq!(shutdown["readiness"]["state"], "stopped");

    let restarted = host.request(request("restart"));
    assert_eq!(restarted["status"], "restarted");
    assert_eq!(restarted["recovery"]["status"], "reconciled");
    assert_eq!(restarted["recovery"]["runtime_evidence"], runtime_evidence);
    assert_eq!(restarted["readiness"]["state"], "ready");

    host.shutdown();
}

#[test]
fn draining_host_fails_closed_before_dispatch() {
    let mut host = HostProcess::spawn();

    let drain = host.request(request("drain"));
    assert_eq!(drain["state"], "draining");
    assert_eq!(drain["release_digest"], host.release_digest.clone());
    assert_eq!(
        drain["port_bindings_digest"],
        host.port_bindings_digest.clone()
    );
    assert_eq!(
        drain["resource_ceiling_digest"],
        host.resource_ceiling_digest.clone()
    );

    let rejected = host.request(json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": "invoke",
        "admission": admission(&host),
        "air": minimal_air(),
    }));
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["error"]["code"], "host_not_accepting");
    assert_eq!(rejected["readiness"]["state"], "draining");
    assert_eq!(
        rejected["readiness"]["release_digest"],
        host.release_digest.clone()
    );
    assert_eq!(
        rejected["readiness"]["port_bindings_digest"],
        host.port_bindings_digest.clone()
    );
    assert_eq!(
        rejected["readiness"]["resource_ceiling_digest"],
        host.resource_ceiling_digest.clone()
    );

    host.shutdown();
}
