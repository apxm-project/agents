//! Vector-backed JSONL parity coverage for the reference-host binary.

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

use apxm_cli::canonical_execute::{
    reference_host_port_bindings_digest, reference_host_resource_ceiling_digest,
};
use apxm_program::air::AirModule;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn exact_digest(label: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(label.as_bytes()))
}

fn load_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("read json fixture")).expect("parse json")
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

fn invoke_vector_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("contracts/reference-host/vectors/apxm.reference-host.invoke-parity.v1.json")
}

fn lifecycle_vector_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("contracts/reference-host/vectors/apxm.reference-host.lifecycle-parity.v1.json")
}

fn startup_input_vector_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("contracts/reference-host/vectors/apxm.reference-host-startup-input.v1.json")
}

fn release_manifest() -> Value {
    load_json(&release_manifest_path())
}

fn invoke_vector() -> Value {
    load_json(&invoke_vector_path())
}

fn lifecycle_vector() -> Value {
    load_json(&lifecycle_vector_path())
}

#[test]
fn startup_input_vector_covers_stdio_and_private_unix_stream_profiles() {
    let vector = load_json(&startup_input_vector_path());
    let cases = vector.as_array().expect("startup-input vector cases");
    let valid_profiles: BTreeSet<&str> = cases
        .iter()
        .filter(|case| case["expected_valid"] == true)
        .map(|case| {
            case["input"]["transport_protocol"]
                .as_str()
                .expect("valid startup-input transport protocol")
        })
        .collect();

    assert_eq!(
        valid_profiles,
        BTreeSet::from(["jsonl-stdin-stdout", "jsonl-unix-stream"])
    );
}

fn live_required_cases() -> Vec<String> {
    release_manifest()["executable_parity_evidence"]["live_required_cases"]
        .as_array()
        .expect("live_required_cases array")
        .iter()
        .map(|entry| entry.as_str().expect("live case name").to_owned())
        .collect()
}

fn vector_case_names(vector: &Value) -> Vec<String> {
    vector["cases"]
        .as_array()
        .expect("vector cases array")
        .iter()
        .map(|case| case["name"].as_str().expect("case name").to_owned())
        .collect()
}

fn case_by_name<'a>(vector: &'a Value, name: &str) -> &'a Value {
    vector["cases"]
        .as_array()
        .expect("vector cases array")
        .iter()
        .find(|case| case["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("missing vector case {name}"))
}

fn fixture<'a>(vector: &'a Value, name: &str) -> &'a Value {
    &vector["fixtures"][name]
}

#[derive(Serialize)]
struct StartupProvenance {
    owner_revision: String,
    descriptor_semantic_digest: String,
    descriptor_exact_checksum: String,
    dirty: bool,
}

fn bytes_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn write_startup_input(
    transport_protocol: &str,
) -> (TempDir, PathBuf, String, String, String, String) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let startup_path = temp_dir.path().join("startup-input.json");
    let release_digest = release_manifest_digest();
    let port_bindings_digest = reference_host_port_bindings_digest();
    let resource_ceiling_digest = reference_host_resource_ceiling_digest();
    let provenance = StartupProvenance {
        owner_revision: "a".repeat(40),
        descriptor_semantic_digest: exact_digest("descriptor-semantic"),
        descriptor_exact_checksum: exact_digest("descriptor-exact"),
        dirty: false,
    };
    let provenance_bytes = serde_json::to_vec(&provenance).expect("provenance json");
    let provenance_digest = bytes_digest(&provenance_bytes);
    fs::write(
        &startup_path,
        serde_json::to_string(&json!({
            "schema_version": "apxm.reference-host-startup-input.v1",
            "semantic_owner": "agents",
            "owner_executable": "apxm-reference-host",
            "owner_executable_path": "crates/tools/cli/src/bin/reference_host.rs",
            "transport_protocol": transport_protocol,
            "reference_host_release_manifest": {
                "path": release_manifest_path(),
                "digest": release_manifest_digest(),
            },
            "release_digest": release_digest,
            "port_bindings_digest": port_bindings_digest,
            "resource_ceiling_digest": resource_ceiling_digest,
            "provenance": provenance,
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
        provenance_digest,
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
    provenance_digest: String,
}

impl HostProcess {
    fn spawn() -> Self {
        let (
            temp_dir,
            startup_path,
            release_digest,
            port_bindings_digest,
            resource_ceiling_digest,
            provenance_digest,
        ) = write_startup_input("jsonl-stdin-stdout");
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
            provenance_digest,
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

#[cfg(unix)]
struct PrivateHostProcess {
    child: Child,
    stream: BufReader<UnixStream>,
    _temp_dir: TempDir,
    startup_path: PathBuf,
    socket_path: PathBuf,
    release_digest: String,
    port_bindings_digest: String,
    resource_ceiling_digest: String,
    provenance_digest: String,
}

#[cfg(unix)]
impl PrivateHostProcess {
    fn spawn() -> Self {
        let (
            temp_dir,
            startup_path,
            release_digest,
            port_bindings_digest,
            resource_ceiling_digest,
            provenance_digest,
        ) = write_startup_input("jsonl-unix-stream");
        let socket_path = temp_dir.path().join("private.sock");
        let mut child = Command::new(env!("CARGO_BIN_EXE_apxm-reference-host"))
            .arg("--startup-input")
            .arg(&startup_path)
            .arg("--unix-socket")
            .arg(&socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn private reference host");

        let deadline = Instant::now() + Duration::from_secs(5);
        let stream = loop {
            if let Ok(metadata) = fs::metadata(&socket_path)
                && metadata.permissions().mode() & 0o777 == 0o600
            {
                match UnixStream::connect(&socket_path) {
                    Ok(stream) => break stream,
                    Err(error) if Instant::now() < deadline => {
                        let _ = error;
                    }
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("connect private reference host socket: {error}");
                    }
                }
            }

            if let Ok(Some(status)) = child.try_wait() {
                let _ = child.wait();
                panic!("private reference host exited before socket readiness: {status}");
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("private reference host socket did not become owner-only");
            }
            thread::sleep(Duration::from_millis(10));
        };

        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set private socket read timeout");
        Self {
            child,
            stream: BufReader::new(stream),
            _temp_dir: temp_dir,
            startup_path,
            socket_path,
            release_digest,
            port_bindings_digest,
            resource_ceiling_digest,
            provenance_digest,
        }
    }

    fn request(&mut self, value: &Value) -> Value {
        writeln!(self.stream.get_mut(), "{value}").expect("write private socket request");
        self.stream
            .get_mut()
            .flush()
            .expect("flush private socket request");

        let mut line = String::new();
        self.stream
            .read_line(&mut line)
            .expect("read private socket response");
        serde_json::from_str(line.trim()).expect("private socket response json")
    }

    fn restart(&mut self) {
        let _ = self.stream.get_mut().shutdown(Shutdown::Both);
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut child = Command::new(env!("CARGO_BIN_EXE_apxm-reference-host"))
            .arg("--startup-input")
            .arg(&self.startup_path)
            .arg("--unix-socket")
            .arg(&self.socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("restart private reference host");
        let deadline = Instant::now() + Duration::from_secs(5);
        let stream = loop {
            if let Ok(metadata) = fs::metadata(&self.socket_path)
                && metadata.permissions().mode() & 0o777 == 0o600
            {
                match UnixStream::connect(&self.socket_path) {
                    Ok(stream) => break stream,
                    Err(error) if Instant::now() < deadline => {
                        let _ = error;
                    }
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("reconnect private reference host socket: {error}");
                    }
                }
            }
            if let Ok(Some(status)) = child.try_wait() {
                let _ = child.wait();
                panic!("private reference host exited during restart: {status}");
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("private reference host socket did not recover after restart");
            }
            thread::sleep(Duration::from_millis(10));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set restarted socket read timeout");
        self.child = child;
        self.stream = BufReader::new(stream);
    }
}

#[cfg(unix)]
impl Drop for PrivateHostProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
fn run_private_host_until_exit(startup_path: &Path, socket_path: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_apxm-reference-host"))
        .arg("--startup-input")
        .arg(startup_path)
        .arg("--unix-socket")
        .arg(socket_path)
        .stdin(Stdio::null())
        .output()
        .expect("run private reference host")
}

#[cfg(unix)]
fn replay_record(invocation_id: &str, response: &Value) -> String {
    serde_json::to_string(&json!({
        "invocation_id": invocation_id,
        "response": response,
    }))
    .expect("serialize replay record")
}

fn request(operation: &str) -> Value {
    json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": operation,
    })
}

fn materialize_admission_with_digests(
    fixture: &Value,
    air: &Value,
    release_digest: &str,
    port_bindings_digest: &str,
    resource_ceiling_digest: &str,
    provenance_digest: &str,
) -> Value {
    let mut admission = fixture.clone();
    let provenance_matches_fixture_artifact =
        admission["provenance_digest"] == admission["artifact_digest"];
    let air: AirModule = serde_json::from_value(air.clone()).expect("admitted AIR fixture");
    let artifact_bytes = serde_json::to_vec(&air).expect("admitted AIR serialization");
    admission["artifact_digest"] = Value::String(bytes_digest(&artifact_bytes));
    admission["release_digest"] = Value::String(release_digest.to_owned());
    admission["port_bindings_digest"] = Value::String(port_bindings_digest.to_owned());
    admission["resource_ceiling_digest"] = Value::String(resource_ceiling_digest.to_owned());
    if provenance_matches_fixture_artifact {
        admission["provenance_digest"] = Value::String(provenance_digest.to_owned());
    }
    admission
}

#[cfg(unix)]
#[test]
fn private_unix_transport_is_owner_only_jsonl_and_host_response() {
    let mut host = PrivateHostProcess::spawn();

    let permissions = fs::metadata(&host.socket_path)
        .expect("private socket metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(permissions, 0o600, "private socket must be owner-only");

    let response = host.request(&request("readiness"));
    assert_eq!(response["schema_version"], "apxm.runtime.host-response.v1");
    assert_eq!(response["status"], "readiness");
    assert_eq!(response["readiness"]["state"], "ready");
}

#[cfg(unix)]
#[test]
fn private_unix_transport_rejects_stdio_startup_attestation() {
    let (temp_dir, startup_path, _, _, _, _) = write_startup_input("jsonl-stdin-stdout");
    let socket_path = temp_dir.path().join("private.sock");
    let output = Command::new(env!("CARGO_BIN_EXE_apxm-reference-host"))
        .arg("--startup-input")
        .arg(&startup_path)
        .arg("--unix-socket")
        .arg(&socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .expect("run mismatched private transport host");
    assert!(!output.status.success());
    assert!(
        !socket_path.exists(),
        "mismatched transport must fail before bind"
    );
}

#[cfg(unix)]
#[test]
fn private_unix_transport_rejects_conflicting_replay_receipts() {
    let (temp_dir, startup_path, _, _, _, _) = write_startup_input("jsonl-unix-stream");
    let socket_path = temp_dir.path().join("private.sock");
    let journal_path = PathBuf::from(format!("{}{}", socket_path.display(), ".replay.jsonl"));
    let first = json!({
        "schema_version": "apxm.runtime.host-response.v1",
        "status": "committed",
        "invocation_id": "invocation.1",
        "result": {"value": "first"},
        "runtime_evidence": null,
    });
    let conflicting = json!({
        "schema_version": "apxm.runtime.host-response.v1",
        "status": "committed",
        "invocation_id": "invocation.1",
        "result": {"value": "different"},
        "runtime_evidence": null,
    });
    fs::write(
        &journal_path,
        format!(
            "{}\n{}\n",
            replay_record("invocation.1", &first),
            replay_record("invocation.1", &conflicting)
        ),
    )
    .expect("write conflicting replay journal");
    fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o600))
        .expect("restrict conflicting replay journal");

    let output = run_private_host_until_exit(&startup_path, &socket_path);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("replay journal contains a conflicting invocation receipt"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn private_unix_transport_rejects_invalid_replay_receipt() {
    let (temp_dir, startup_path, _, _, _, _) = write_startup_input("jsonl-unix-stream");
    let socket_path = temp_dir.path().join("private.sock");
    let journal_path = PathBuf::from(format!("{}{}", socket_path.display(), ".replay.jsonl"));
    let invalid = json!({
        "schema_version": "apxm.runtime.host-response.v1",
        "status": "rejected",
        "invocation_id": "invocation.1",
    });
    fs::write(
        &journal_path,
        format!("{}\n", replay_record("invocation.1", &invalid)),
    )
    .expect("write invalid replay journal");
    fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o600))
        .expect("restrict invalid replay journal");

    let output = run_private_host_until_exit(&startup_path, &socket_path);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("replay journal contains an invalid invocation receipt"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn private_unix_transport_recovers_torn_final_replay_line() {
    let vector = invoke_vector();
    let mut host = PrivateHostProcess::spawn();
    let air = fixture(&vector, "minimal_valid_air").clone();
    let invoke = json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": "invoke",
        "admission": materialize_admission_with_digests(
            fixture(&vector, "valid_exact_admission"),
            &air,
            &host.release_digest,
            &host.port_bindings_digest,
            &host.resource_ceiling_digest,
            &host.provenance_digest,
        ),
        "air": air,
    });
    let first = host.request(&invoke);
    assert_eq!(first["status"], "committed");

    let journal_path = PathBuf::from(format!("{}{}", host.socket_path.display(), ".replay.jsonl"));
    let mut journal = fs::OpenOptions::new()
        .append(true)
        .open(&journal_path)
        .expect("open replay journal for torn tail");
    journal
        .write_all(br#"{"invocation_id":"invocation.torn","response":{"status":"committed"}"#)
        .expect("append torn replay line");
    journal.flush().expect("flush torn replay line");
    drop(journal);

    host.restart();
    let recovered = host.request(&invoke);
    assert_eq!(
        recovered, first,
        "complete receipt must survive torn tail recovery"
    );
    let journal_contents = fs::read_to_string(&journal_path).expect("read recovered journal");
    assert_eq!(journal_contents.lines().count(), 1);
    assert!(!journal_contents.contains("invocation.torn"));
}

#[cfg(unix)]
#[test]
fn private_unix_transport_rejects_group_or_world_writable_parent() {
    let (temp_dir, startup_path, _, _, _, _) = write_startup_input("jsonl-unix-stream");
    fs::set_permissions(temp_dir.path(), fs::Permissions::from_mode(0o777))
        .expect("make private transport parent writable");
    let socket_path = temp_dir.path().join("private.sock");

    let output = run_private_host_until_exit(&startup_path, &socket_path);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("private transport parent must not be group/world writable"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn private_unix_transport_rejects_non_owner_replay_journal_permissions() {
    let (temp_dir, startup_path, _, _, _, _) = write_startup_input("jsonl-unix-stream");
    let socket_path = temp_dir.path().join("private.sock");
    let journal_path = PathBuf::from(format!("{}{}", socket_path.display(), ".replay.jsonl"));
    fs::write(&journal_path, "").expect("create replay journal");
    fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o644))
        .expect("make replay journal readable by peers");

    let output = run_private_host_until_exit(&startup_path, &socket_path);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("replay journal must be owner-only"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn private_unix_transport_commits_admitted_invoke_and_replays_exact_receipt() {
    let vector = invoke_vector();
    let mut host = PrivateHostProcess::spawn();
    let air = fixture(&vector, "minimal_valid_air").clone();
    let invoke = json!({
        "schema_version": "apxm.runtime.host-request.v1",
        "operation": "invoke",
        "admission": materialize_admission_with_digests(
            fixture(&vector, "valid_exact_admission"),
            &air,
            &host.release_digest,
            &host.port_bindings_digest,
            &host.resource_ceiling_digest,
            &host.provenance_digest,
        ),
        "air": air,
    });

    let first = host.request(&invoke);
    assert_eq!(first["status"], "committed");
    assert_eq!(first["invocation_id"], "invocation.1");
    assert_eq!(
        terminal_fact(&first["runtime_evidence"])["commit_sequence"],
        1
    );
    let journal_path = PathBuf::from(format!("{}{}", host.socket_path.display(), ".replay.jsonl"));
    let journal_permissions = fs::metadata(&journal_path)
        .expect("replay journal metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        journal_permissions, 0o600,
        "replay journal must be owner-only"
    );
    assert!(
        fs::read_to_string(&journal_path)
            .expect("read replay journal")
            .contains("invocation.1"),
        "committed receipt must be durably journaled"
    );

    let replay = host.request(&invoke);
    assert_eq!(
        replay, first,
        "duplicate invocation must replay its exact receipt"
    );
    assert_eq!(
        terminal_fact(&replay["runtime_evidence"])["commit_sequence"],
        1
    );

    host.restart();
    let recovered = host.request(&invoke);
    assert_eq!(
        recovered, first,
        "duplicate invocation must replay its exact receipt after process restart"
    );

    let readiness = host.request(&request("readiness"));
    assert_eq!(readiness["readiness"]["state"], "ready");
    assert_eq!(readiness["readiness"]["in_flight"], 0);
}

fn readiness(host: &mut HostProcess) -> Value {
    host.request(&request("readiness"))
}

fn materialize_admission(host: &HostProcess, fixture: &Value, air: &Value) -> Value {
    materialize_admission_with_digests(
        fixture,
        air,
        &host.release_digest,
        &host.port_bindings_digest,
        &host.resource_ceiling_digest,
        &host.provenance_digest,
    )
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
        let air_fixture_name = case["air_fixture"].as_str().expect("air fixture");
        let air = fixture(&vector, air_fixture_name).clone();
        let admission_air = if case_name == "invalid_air_failure" {
            fixture(&vector, "minimal_valid_air")
        } else {
            &air
        };
        let response = host.request(&json!({
            "schema_version": "apxm.runtime.host-request.v1",
            "operation": "invoke",
            "admission": materialize_admission(
                &host,
                fixture(&vector, case["admission_fixture"].as_str().expect("admission fixture")),
                admission_air,
            ),
            "air": air,
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
                    readiness(&mut host)["readiness"]["state"],
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
                let (_temp_dir, startup_path, _, _, _, _) =
                    write_startup_input("jsonl-stdin-stdout");
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
                assert_eq!(probe["drain_response"]["readiness"]["state"], "draining");
                assert_eq!(probe["drain_response"]["readiness"]["in_flight"], 1);
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
                        fixture(&invoke, "minimal_valid_air"),
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
                        fixture(&invoke, "minimal_valid_air"),
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
                assert_eq!(drain["readiness"]["state"], "draining");
                let host_not_accepting = host.request(&json!({
                    "schema_version": "apxm.runtime.host-request.v1",
                    "operation": "invoke",
                    "admission": materialize_admission(
                        &host,
                        fixture(&invoke, "valid_exact_admission"),
                        fixture(&invoke, "minimal_valid_air"),
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
