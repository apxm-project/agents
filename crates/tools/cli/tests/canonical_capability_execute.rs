//! Live CLI coverage for the admitted Capability port: the shipped
//! `execute-canonical` path running a real tool, and denying one that the local
//! composition root does not admit.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::{TempDir, tempdir};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

fn fixture(name: &str) -> PathBuf {
    repository_root().join("tools/tests/fixtures").join(name)
}

/// The fixture AIR authors a repository-relative path, so the CLI runs from the
/// repository root exactly as `dekk agents execute-canonical` does.
fn execute_fixture(
    input: &std::path::Path,
    admission: &std::path::Path,
    package: Option<&std::path::Path>,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apxm"));
    command.current_dir(repository_root());
    command.args([
        "--json",
        "execute-canonical",
        input.to_str().expect("AIR path"),
        "--invocation-admission",
        admission.to_str().expect("admission path"),
        "--release",
        fixture("canonical-execute.release.json")
            .to_str()
            .expect("release fixture path"),
        "--provenance",
        fixture("canonical-execute.provenance.json")
            .to_str()
            .expect("provenance fixture path"),
    ]);
    if let Some(package) = package {
        command.args(["--package", package.to_str().expect("package path")]);
    }
    command.output().expect("execute canonical CLI")
}

fn execute_capability_fixture() -> std::process::Output {
    execute_fixture(
        &fixture("canonical-capability-execute.air.json"),
        &fixture("canonical-capability-execute.invocation-admission.json"),
        None,
    )
}

fn build_policy_package(permission: &str) -> TempDir {
    let package = tempdir().expect("policy package directory");
    std::fs::write(
        package.path().join("agent.toml"),
        format!(
            "id = \"canonical-policy-test\"\n\
             version = \"0.1.0\"\n\
             schema_version = \"apxm.agent\"\n\
             display_name = \"Canonical Policy Test\"\n\
             description = \"Temporary package for canonical permission tests.\"\n\
             domain = \"general\"\n\
             kind = \"agent\"\n\
             license = \"MIT\"\n\
             [permissions]\n\
             read = {permission}\n"
        ),
    )
    .expect("write policy package manifest");

    let output = Command::new(env!("CARGO_BIN_EXE_apxm"))
        .args([
            "agent",
            "build",
            package.path().to_str().expect("package path"),
        ])
        .output()
        .expect("build policy package");
    assert!(
        output.status.success(),
        "policy package build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    package
}

fn authored_ask_fixture() -> (TempDir, PathBuf, PathBuf) {
    let temp = tempdir().expect("temporary authored-ask fixture directory");
    let air_path = temp.path().join("authored-ask.air.json");
    let admission_path = temp.path().join("authored-ask.admission.json");

    let mut air: Value = serde_json::from_slice(
        &std::fs::read(fixture("canonical-capability-execute.air.json"))
            .expect("read canonical AIR fixture"),
    )
    .expect("parse canonical AIR fixture");
    air["capability_permission_requests"] = serde_json::json!({
        "read": {
            "decision": "ask",
            "reason": "The authored program requires confirmation before reading."
        }
    });
    let air_bytes = serde_json::to_vec_pretty(&air).expect("serialize authored-ask AIR");
    std::fs::write(&air_path, &air_bytes).expect("write authored-ask AIR");

    let mut admission: Value = serde_json::from_slice(
        &std::fs::read(fixture(
            "canonical-capability-execute.invocation-admission.json",
        ))
        .expect("read canonical admission fixture"),
    )
    .expect("parse canonical admission fixture");
    admission["artifact_digest"] =
        serde_json::json!(format!("sha256:{:x}", Sha256::digest(&air_bytes)));
    std::fs::write(
        &admission_path,
        serde_json::to_vec_pretty(&admission).expect("serialize authored-ask admission"),
    )
    .expect("write authored-ask admission");

    (temp, air_path, admission_path)
}

fn node_outcome<'a>(result: &'a Value, node_id: &str) -> &'a Value {
    result["results"]["node_outcomes"]
        .as_array()
        .expect("node outcomes array")
        .iter()
        .find(|outcome| outcome["node_id"] == node_id)
        .unwrap_or_else(|| panic!("no node outcome for {node_id}: {result}"))
}

#[test]
fn an_authored_capability_invoke_runs_a_real_tool() {
    let denied_write = repository_root().join(".apxm/canonical-capability-denied-write.txt");
    let _ = std::fs::remove_file(&denied_write);

    let output = execute_capability_fixture();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("canonical result JSON");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["commit"]["status"], "committed");

    let read = node_outcome(&result, "fixture.capability.read");
    assert_eq!(read["kind"], "capability.invoke");
    assert_eq!(
        read["outcome"]["status"], "completed",
        "an admitted Tool(\"read\") reaches ReadCapability: {result}"
    );
    assert!(
        read["outcome"]["result"]
            .as_str()
            .expect("capability result text")
            .contains("canonical capability fixture payload"),
        "the capability returns the fixture file's contents: {result}"
    );

    let denied = node_outcome(&result, "fixture.capability.denied");
    assert_eq!(
        denied["outcome"]["status"], "failed",
        "an unadmitted capability fails closed: {result}"
    );
    let message = denied["outcome"]["message"]
        .as_str()
        .expect("failure message");
    assert!(
        message.contains("is deny") && message.contains("by the package layer"),
        "the refusal is the resolved permission decision and names the layer that gave it, so \
         the shipped path is enforcing the lattice rather than only the port interceptor: \
         {result}"
    );
    assert!(
        message.contains("read-only capability surface"),
        "the decision still carries the reason the local root refuses: {result}"
    );
    assert!(
        !denied_write.exists(),
        "the denial happens before the write implementation receives its arguments"
    );
}

#[test]
fn package_ask_policy_reaches_canonical_capability_admission() {
    let package = build_policy_package(
        "{ decision = \"ask\", reason = \"Package policy requires approval before reading.\" }",
    );
    let output = execute_fixture(
        &fixture("canonical-capability-execute.air.json"),
        &fixture("canonical-capability-execute.invocation-admission.json"),
        Some(package.path()),
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("canonical result JSON");
    let read = node_outcome(&result, "fixture.capability.read");
    assert_eq!(read["outcome"]["status"], "failed");
    let message = read["outcome"]["message"]
        .as_str()
        .expect("package policy refusal message");
    assert!(
        message.contains("is ask") && message.contains("by the package layer"),
        "the live admission carries the package Ask decision: {result}"
    );
    assert!(
        message.contains("Package policy requires approval before reading."),
        "the package policy reason reaches the capability outcome: {result}"
    );
}

#[test]
fn package_policy_cannot_widen_an_authored_capability_request() {
    let package = build_policy_package("\"allow\"");
    let (_fixtures, air, admission) = authored_ask_fixture();
    let output = execute_fixture(&air, &admission, Some(package.path()));
    assert!(
        !output.status.success(),
        "package widening must fail closed"
    );
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("may only tighten it, not widen it to allow"),
        "the canonical command rejects package widening before execution: {diagnostic}"
    );
}
