//! Live JSONL lifecycle coverage for the reference-host binary.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct HostProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl HostProcess {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_apxm-reference-host"))
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

fn admission() -> Value {
    json!({
        "schema_version": "apxm.invocation-admission.v1",
        "invocation_id": "invocation.1",
        "artifact_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "release_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "port_bindings_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "resource_ceiling_digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "provenance_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
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

fn lifecycle_case(name: &str) -> Value {
    let vector_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("contracts/reference-host/vectors/apxm.reference-host.lifecycle-parity.v1.json");
    let vector: Value =
        serde_json::from_slice(&fs::read(vector_path).expect("read lifecycle vector"))
            .expect("parse lifecycle vector");
    vector["cases"]
        .as_array()
        .expect("lifecycle cases")
        .iter()
        .find(|case| case["name"] == name)
        .cloned()
        .expect("lifecycle case")
}

fn expected_host_response(name: &str) -> Value {
    lifecycle_case(name)["expected"]["expected_host_response"].clone()
}

#[test]
fn cancellation_before_admission_matches_published_shape() {
    let mut host = HostProcess::spawn();

    let cancelled = host.request(request("cancel"));
    assert_eq!(
        cancelled,
        expected_host_response("cancellation_before_admission")
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
    assert_eq!(
        revoked,
        expected_host_response("revocation_before_dispatch")
    );

    let rejected = host.request(json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": "invoke",
        "admission": admission(),
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
        "admission": admission(),
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
        "admission": admission(),
        "air": minimal_air(),
    }));
    assert_eq!(committed["status"], "committed");
    let runtime_evidence = committed["runtime_evidence"].clone();

    let shutdown = host.request(request("shutdown"));
    assert_eq!(
        shutdown,
        expected_host_response("explicit_shutdown_terminal_state")
    );

    let restarted = host.request(request("restart"));
    let mut expected = expected_host_response("restart_recovery_from_runtime_evidence");
    expected["recovery"]["runtime_evidence"] = runtime_evidence.clone();
    assert_eq!(restarted, expected);

    host.shutdown();
}

#[test]
fn draining_host_fails_closed_before_dispatch() {
    let mut host = HostProcess::spawn();

    let drain = host.request(request("drain"));
    assert_eq!(drain["state"], "draining");

    let rejected = host.request(json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": "invoke",
        "admission": admission(),
        "air": minimal_air(),
    }));
    assert_eq!(
        rejected,
        json!({
            "schema_version": "apxm.runtime.host.v1",
            "status": "rejected",
            "error": {
                "code": "host_not_accepting",
                "message": "host state is draining"
            },
            "readiness": {
                "schema_version": "apxm.runtime.host.v1",
                "contract_id": "apxm.runtime.host.v1",
                "state": "draining",
                "release_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "port_bindings_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "resource_ceiling_digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "in_flight": 0
            }
        })
    );

    host.shutdown();
}
