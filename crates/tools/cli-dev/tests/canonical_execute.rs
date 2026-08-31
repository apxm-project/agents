//! Live CLI coverage for exact Invocation Admission at the canonical runtime boundary.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

#[path = "apxm_dev_bin.rs"]
mod apxm_dev_bin;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("tools/tests/fixtures")
        .join(name)
}

fn command(release: &std::path::Path) -> Command {
    let mut command = apxm_dev_bin::command();
    command.args([
        "--json",
        "execute-canonical",
        fixture("canonical-execute.air.json")
            .to_str()
            .expect("AIR fixture path"),
        "--invocation-admission",
        fixture("canonical-execute.invocation-admission.json")
            .to_str()
            .expect("admission fixture path"),
        "--release",
        release.to_str().expect("release fixture path"),
        "--provenance",
        fixture("canonical-execute.provenance.json")
            .to_str()
            .expect("provenance fixture path"),
    ]);
    command
}

#[test]
fn exact_invocation_admission_executes_the_canonical_fixture() {
    let output = command(&fixture("canonical-execute.release.json"))
        .output()
        .expect("execute canonical CLI");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("canonical result JSON");
    assert_eq!(
        result, "fixture.instance",
        "the command returns the typed committed Session Output content"
    );
}

#[test]
fn release_byte_drift_is_rejected_before_execution() {
    let temp = tempfile::tempdir().expect("temporary release directory");
    let tampered_release = temp.path().join("release.json");
    fs::write(&tampered_release, b"tampered release bytes").expect("write tampered release");
    let output = command(&tampered_release)
        .output()
        .expect("execute canonical CLI");
    assert!(!output.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("release digest mismatch"),
        "diagnostic: {diagnostic}"
    );
}
