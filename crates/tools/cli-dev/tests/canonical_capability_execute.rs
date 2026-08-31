//! Live CLI coverage for the admitted Capability port: the shipped
//! `execute-canonical` path running a real tool, and denying one that the local
//! composition root does not admit.

use std::path::PathBuf;

use apxm_program::{ExecutableArtifact, air::AirModule};
use serde_json::Value;
use tempfile::{TempDir, tempdir};

#[path = "apxm_dev_bin.rs"]
mod apxm_dev_bin;

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
    let mut command = apxm_dev_bin::command();
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

fn execute_output_fixture() -> std::process::Output {
    execute_fixture(
        &fixture("canonical-capability-output.air.json"),
        &fixture("canonical-capability-output.invocation-admission.json"),
        None,
    )
}

fn execute_denied_capability_fixture() -> std::process::Output {
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

    apxm_cli_dev::commands::agent::agent_build(package.path(), true).expect("build policy package");
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
    let authored_air: AirModule =
        serde_json::from_slice(&air_bytes).expect("parse authored-ask AIR");
    let authored_artifact =
        ExecutableArtifact::from_air(&authored_air).expect("seal authored-ask artifact");
    admission["artifact_digest"] = serde_json::json!(authored_artifact.artifact_digest);
    std::fs::write(
        &admission_path,
        serde_json::to_vec_pretty(&admission).expect("serialize authored-ask admission"),
    )
    .expect("write authored-ask admission");

    (temp, air_path, admission_path)
}

#[test]
fn an_authored_capability_invoke_runs_a_real_tool() {
    let denied_write = repository_root().join(".apxm/canonical-capability-denied-write.txt");
    let _ = std::fs::remove_file(&denied_write);

    let output = execute_output_fixture();
    assert!(
        output.status.success(),
        "status: {}; stdout: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("canonical result JSON");
    // The command returns only the bytes read through the committed V2
    // output reference. It does not reconstruct a legacy execution wrapper
    // containing node outcomes or copy a last-output sidecar.
    assert_eq!(result, "5", "typed committed output content: {result}");
    assert!(
        !denied_write.exists(),
        "the denied capability still fails before the write implementation receives its arguments"
    );
}

#[test]
fn unadmitted_capability_fails_closed_without_output() {
    let denied_write = repository_root().join(".apxm/canonical-capability-denied-write.txt");
    let _ = std::fs::remove_file(&denied_write);

    let output = execute_denied_capability_fixture();
    assert!(!output.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("Runtime Service committed no output reference"),
        "a terminal denied capability has no committed output to read: {diagnostic}"
    );
    assert!(!denied_write.exists());
}

#[test]
fn package_ask_policy_fails_closed_without_output() {
    let package = build_policy_package(
        "{ decision = \"ask\", reason = \"Package policy requires approval before reading.\" }",
    );
    let output = execute_fixture(
        &fixture("canonical-capability-execute.air.json"),
        &fixture("canonical-capability-execute.invocation-admission.json"),
        Some(package.path()),
    );
    assert!(!output.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("Runtime Service committed no output reference"),
        "an Ask-denied terminal invocation has no committed output to read: {diagnostic}"
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
