//! Vector-backed JSONL parity coverage for the reference-host binary.

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
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
            "semantic_owner": "agents",
            "owner_executable": "apxm-reference-host",
            "owner_executable_path": "crates/tools/cli/src/bin/reference_host.rs",
            "transport_protocol": "jsonl-stdin-stdout",
            "reference_host_release_manifest": {
                "path": release_manifest_path(),
                "digest": release_manifest_digest(),
            },
            "release_digest": release_digest,
            "port_bindings_digest": port_bindings_digest,
            "resource_ceiling_digest": resource_ceiling_digest,
            "provenance": {
                "owner_revision": "a".repeat(40),
                "descriptor_semantic_digest": exact_digest("descriptor-semantic"),
                "descriptor_exact_checksum": exact_digest("descriptor-exact"),
                "dirty": false
            },
            "fail_closed_on": ["missing", "placeholder", "dirty", "mismatched", "implicit-default"],
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

    fn request(&mut self, value: &Value) -> Value {
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

fn readiness(host: &mut HostProcess) -> Value {
    host.request(&request("readiness"))
}

fn materialize_admission(host: &HostProcess, fixture: &Value) -> Value {
    let mut admission = fixture.clone();
    let provenance_matches_artifact =
        admission["provenance_digest"] == admission["artifact_digest"];
    admission["artifact_digest"] = Value::String(host.release_digest.clone());
    admission["release_digest"] = Value::String(host.release_digest.clone());
    admission["port_bindings_digest"] = Value::String(host.port_bindings_digest.clone());
    admission["resource_ceiling_digest"] = Value::String(host.resource_ceiling_digest.clone());
    if provenance_matches_artifact {
        admission["provenance_digest"] = Value::String(host.release_digest.clone());
    }
    admission
}

fn materialize_expected_response(host: &HostProcess, expected: &Value) -> Value {
    let mut materialized = expected.clone();
    if let Some(readiness) = materialized
        .get_mut("readiness")
        .and_then(Value::as_object_mut)
    {
        readiness.insert(
            "release_digest".into(),
            Value::String(host.release_digest.clone()),
        );
        readiness.insert(
            "port_bindings_digest".into(),
            Value::String(host.port_bindings_digest.clone()),
        );
        readiness.insert(
            "resource_ceiling_digest".into(),
            Value::String(host.resource_ceiling_digest.clone()),
        );
    }
    materialized
}

fn assert_json_subset(actual: &Value, expected: &Value, path: &str) {
    match expected {
        Value::Object(expected_map) => {
            let actual_map = actual
                .as_object()
                .unwrap_or_else(|| panic!("{path} must be an object"));
            for (key, expected_value) in expected_map {
                let child_path = format!("{path}.{key}");
                let actual_value = actual_map
                    .get(key)
                    .unwrap_or_else(|| panic!("{child_path} is missing"));
                assert_json_subset(actual_value, expected_value, &child_path);
            }
        }
        Value::Array(expected_items) => {
            let actual_items = actual
                .as_array()
                .unwrap_or_else(|| panic!("{path} must be an array"));
            assert_eq!(
                actual_items.len(),
                expected_items.len(),
                "{path} length mismatch"
            );
            for (index, (actual_item, expected_item)) in
                actual_items.iter().zip(expected_items.iter()).enumerate()
            {
                assert_json_subset(actual_item, expected_item, &format!("{path}[{index}]"));
            }
        }
        _ => assert_eq!(actual, expected, "{path} mismatch"),
    }
}

fn assert_no_runtime_evidence(response: &Value) {
    if let Some(runtime_evidence) = response.get("runtime_evidence") {
        assert!(
            runtime_evidence.is_null(),
            "runtime_evidence must be null or absent"
        );
    }
}

fn terminal_fact(runtime_evidence: &Value) -> &Value {
    runtime_evidence["facts"]
        .as_array()
        .expect("runtime_evidence facts")
        .last()
        .expect("runtime_evidence terminal fact")
}

fn live_case_names_for(vector: &Value) -> Vec<String> {
    let live_cases: BTreeSet<_> = live_required_cases().into_iter().collect();
    vector_case_names(vector)
        .into_iter()
        .filter(|name| live_cases.contains(name))
        .collect()
}

fn assert_case_partition() {
    let invoke_vector = invoke_vector();
    let lifecycle_vector = lifecycle_vector();
    let all_cases = [
        vector_case_names(&invoke_vector),
        vector_case_names(&lifecycle_vector),
    ]
    .concat();
    let live_cases = live_required_cases();
    assert_eq!(
        live_cases, all_cases,
        "live executable cases drifted from the vectors"
    );
}

#[test]
fn executable_parity_manifest_covers_every_published_case() {
    assert_case_partition();
}

#[test]
fn invoke_parity_vectors_execute_live_cases() {
    assert_case_partition();
    let vector = invoke_vector();
    let executed_cases = live_case_names_for(&vector);
    assert_eq!(
        executed_cases,
        vec![
            "positive_commit_minimal_air".to_owned(),
            "negative_admission_provenance_rejection".to_owned(),
            "invalid_air_failure".to_owned(),
        ]
    );

    for case_name in executed_cases {
        let case = case_by_name(&vector, &case_name);
        let expected = &case["expected"];
        let mut host = HostProcess::spawn();
        let response = host.request(&json!({
            "schema_version": "apxm.runtime.host-request.v1",
            "operation": "invoke",
            "admission": materialize_admission(
                &host,
                fixture(&vector, case["admission_fixture"].as_str().expect("admission fixture")),
            ),
            "air": fixture(&vector, case["air_fixture"].as_str().expect("air fixture")).clone(),
        }));

        match expected["parity_outcome"]
            .as_str()
            .expect("invoke parity outcome")
        {
            "committed_return" => {
                assert_eq!(response["status"], "committed");
                let terminal = terminal_fact(&response["runtime_evidence"]);
                assert_eq!(
                    terminal["fact_kind"],
                    expected["runtime_evidence_terminal_kind"]
                );
                assert_eq!(terminal["commit_sequence"], expected["commit_sequence"]);
                assert_eq!(
                    readiness(&mut host)["state"],
                    expected["reference_host_state_after"]
                );
            }
            "rejected_before_commit" => {
                assert_eq!(response["status"], "rejected");
                assert_eq!(response["error"]["code"], expected["error_code"]);
                assert_no_runtime_evidence(&response);
                assert_eq!(
                    response["readiness"]["state"],
                    expected["reference_host_state_after"]
                );
            }
            other => panic!("unexpected invoke parity outcome {other}"),
        }

        host.shutdown();
    }
}

#[test]
fn lifecycle_parity_vectors_execute_live_cases() {
    assert_case_partition();
    let vector = lifecycle_vector();
    let executed_cases = live_case_names_for(&vector);
    assert_eq!(
        executed_cases,
        vec![
            "cancellation_before_admission".to_owned(),
            "drain_shutdown_after_in_flight_completion".to_owned(),
            "explicit_shutdown_terminal_state".to_owned(),
            "restart_recovery_from_runtime_evidence".to_owned(),
            "revocation_before_dispatch".to_owned(),
            "boundary_fail_closed".to_owned(),
        ]
    );

    for case_name in executed_cases {
        let case = case_by_name(&vector, &case_name);
        let expected = &case["expected"];
        match case_name.as_str() {
            "cancellation_before_admission" => {
                let mut host = HostProcess::spawn();
                let cancelled = host.request(&request("cancel"));
                assert_json_subset(
                    &cancelled,
                    &materialize_expected_response(&host, &expected["expected_host_response"]),
                    "cancelled",
                );
                assert_no_runtime_evidence(&cancelled);
                let restarted = host.request(&request("restart"));
                assert_eq!(restarted["readiness"]["state"], expected["restart_state"]);
                host.shutdown();
            }
            "drain_shutdown_after_in_flight_completion" => {
                let (_temp_dir, startup_path, _, _, _) = write_startup_input();
                let output = Command::new(env!("CARGO_BIN_EXE_apxm-reference-host"))
                    .arg("--startup-input")
                    .arg(startup_path)
                    .arg("--lifecycle-probe")
                    .arg("drain_shutdown_after_in_flight_completion")
                    .output()
                    .expect("run in-flight drain probe");
                assert!(
                    output.status.success(),
                    "in-flight drain probe exited with {}",
                    output.status
                );
                let probe: Value =
                    serde_json::from_slice(&output.stdout).expect("parse in-flight drain probe");
                assert_eq!(
                    probe["schema_version"],
                    "apxm.reference-host.lifecycle-probe.v1"
                );
                assert_eq!(probe["case"], "drain_shutdown_after_in_flight_completion");
                assert_eq!(probe["transition"], "stop_admission_then_finish_in_flight");
                assert_eq!(probe["in_flight_before_drain"]["state"], "ready");
                assert_eq!(probe["in_flight_before_drain"]["in_flight"], 1);
                assert_eq!(probe["drain_response"]["state"], "draining");
                assert_eq!(probe["drain_response"]["in_flight"], 1);
                assert_eq!(probe["completion_response"]["status"], "committed");
                assert_eq!(
                    terminal_fact(&probe["completion_response"]["runtime_evidence"])["fact_kind"],
                    expected["runtime_evidence_terminal_kind"]
                );
                assert_eq!(probe["terminal_readiness"]["state"], "stopped");
                assert_eq!(probe["terminal_readiness"]["in_flight"], 0);
            }
            "explicit_shutdown_terminal_state" => {
                let mut host = HostProcess::spawn();
                let shutdown = host.request(&request("shutdown"));
                assert_json_subset(
                    &shutdown,
                    &materialize_expected_response(&host, &expected["expected_host_response"]),
                    "shutdown",
                );
                assert_no_runtime_evidence(&shutdown);
                host.shutdown();
            }
            "restart_recovery_from_runtime_evidence" => {
                let invoke = invoke_vector();
                let mut host = HostProcess::spawn();
                let committed = host.request(&json!({
                    "schema_version": "apxm.runtime.host-request.v1",
                    "operation": "invoke",
                    "admission": materialize_admission(
                        &host,
                        fixture(&invoke, "valid_exact_admission"),
                    ),
                    "air": fixture(&invoke, "minimal_valid_air").clone(),
                }));
                assert_eq!(committed["status"], "committed");
                let shutdown = host.request(&request("shutdown"));
                assert_eq!(shutdown["status"], "shutdown");
                let restarted = host.request(&request("restart"));
                assert_json_subset(
                    &restarted,
                    &materialize_expected_response(&host, &expected["expected_host_response"]),
                    "restarted",
                );
                assert_eq!(
                    restarted["recovery"]["runtime_evidence"]["schema_version"],
                    expected["recovery_source"]
                );
                assert_eq!(
                    terminal_fact(&restarted["recovery"]["runtime_evidence"])["fact_kind"],
                    expected["runtime_evidence_terminal_kind"]
                );
                host.shutdown();
            }
            "revocation_before_dispatch" => {
                let invoke = invoke_vector();
                let mut host = HostProcess::spawn();
                let revoked = host.request(&request("revoke"));
                assert_json_subset(
                    &revoked,
                    &materialize_expected_response(&host, &expected["expected_host_response"]),
                    "revoked",
                );
                assert_no_runtime_evidence(&revoked);
                let rejected = host.request(&json!({
                    "schema_version": "apxm.runtime.host-request.v1",
                    "operation": "invoke",
                    "admission": materialize_admission(
                        &host,
                        fixture(&invoke, "valid_exact_admission"),
                    ),
                    "air": fixture(&invoke, "minimal_valid_air").clone(),
                }));
                assert_eq!(rejected["status"], "rejected");
                assert_eq!(rejected["error"]["code"], "admission_revoked");
                assert_eq!(rejected["readiness"]["state"], "stopped");
                host.shutdown();
            }
            "boundary_fail_closed" => {
                let invoke = invoke_vector();
                let mut seen = BTreeSet::new();

                let mut host = HostProcess::spawn();
                let missing_admission = host.request(&json!({
                    "schema_version": "apxm.runtime.host-request.v1",
                    "operation": "invoke",
                    "air": fixture(&invoke, "minimal_valid_air").clone(),
                }));
                assert_eq!(missing_admission["status"], "rejected");
                assert_no_runtime_evidence(&missing_admission);
                seen.insert(
                    missing_admission["error"]["code"]
                        .as_str()
                        .expect("missing admission code")
                        .to_owned(),
                );
                host.shutdown();

                let mut host = HostProcess::spawn();
                let invalid_schema = host.request(&json!({
                    "schema_version": "apxm.runtime.host-request.v0",
                    "operation": "readiness",
                }));
                assert_eq!(invalid_schema["status"], "rejected");
                assert_no_runtime_evidence(&invalid_schema);
                seen.insert(
                    invalid_schema["error"]["code"]
                        .as_str()
                        .expect("invalid schema code")
                        .to_owned(),
                );
                host.shutdown();

                let mut host = HostProcess::spawn();
                let unknown_operation = host.request(&request("unknown"));
                assert_eq!(unknown_operation["status"], "rejected");
                assert_no_runtime_evidence(&unknown_operation);
                seen.insert(
                    unknown_operation["error"]["code"]
                        .as_str()
                        .expect("unknown operation code")
                        .to_owned(),
                );
                host.shutdown();

                let mut host = HostProcess::spawn();
                let drain = host.request(&request("drain"));
                assert_eq!(drain["state"], "draining");
                let host_not_accepting = host.request(&json!({
                    "schema_version": "apxm.runtime.host-request.v1",
                    "operation": "invoke",
                    "admission": materialize_admission(
                        &host,
                        fixture(&invoke, "valid_exact_admission"),
                    ),
                    "air": fixture(&invoke, "minimal_valid_air").clone(),
                }));
                assert_eq!(host_not_accepting["status"], "rejected");
                assert_no_runtime_evidence(&host_not_accepting);
                seen.insert(
                    host_not_accepting["error"]["code"]
                        .as_str()
                        .expect("host_not_accepting code")
                        .to_owned(),
                );
                host.shutdown();

                let expected_rejections: BTreeSet<_> = expected["boundary_rejections"]
                    .as_array()
                    .expect("boundary_rejections array")
                    .iter()
                    .map(|entry| entry.as_str().expect("boundary rejection").to_owned())
                    .collect();
                assert_eq!(seen, expected_rejections);
            }
            other => panic!("unexpected live lifecycle case {other}"),
        }
    }
}
