//! Live CLI coverage for a host-fulfilled Capability through the shipped
//! `execute-canonical` path.
//!
//! The fixture program invokes two `host:` Capabilities. APXM executes neither:
//! it publishes a request, parks, and settles the node when the host answers
//! with `capability_fulfill` (ADR-0025). Here the command is the host, so the
//! whole seam — request, park, settle, resume, terminal commit — runs end to
//! end against the shipped binary.

use std::path::PathBuf;

use apxm_program::{ExecutableArtifact, air::AirModule};
use apxm_runtime_service::materials_for_artifact;
use serde_json::Value;

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

fn fixture_artifact_bytes() -> Vec<u8> {
    let raw = std::fs::read(fixture("canonical-host-capability-execute.air.json"))
        .expect("the host capability fixture AIR");
    let air: AirModule = serde_json::from_slice(&raw).expect("fixture AIR decodes");
    ExecutableArtifact::from_air(&air)
        .expect("the fixture seals into an artifact")
        .encode()
        .expect("the fixture artifact encodes")
}

/// The checked-in Invocation Admission is the exact one the fixture's materials
/// bind. Editing the fixture AIR without regenerating the admission fails here
/// rather than as an opaque digest mismatch at execution.
#[test]
fn the_admission_fixture_binds_the_fixture_it_names() {
    let release = std::fs::read(fixture("canonical-execute.release.json")).expect("release");
    let provenance =
        std::fs::read(fixture("canonical-execute.provenance.json")).expect("provenance");
    let materials = materials_for_artifact(
        &fixture_artifact_bytes(),
        "invocation.canonical-host-capability-fixture.1",
        release,
        provenance,
    );
    let expected = serde_json::to_value(&materials.admission).expect("the admission serializes");
    let published: Value = serde_json::from_slice(
        &std::fs::read(fixture(
            "canonical-host-capability-execute.invocation-admission.json",
        ))
        .expect("the published admission"),
    )
    .expect("the published admission is JSON");
    assert_eq!(
        published,
        expected,
        "regenerate the admission fixture:\n{}",
        serde_json::to_string_pretty(&expected).expect("pretty")
    );
}

#[test]
fn a_host_fulfilled_fixture_is_requested_settled_and_committed() {
    let mut command = apxm_dev_bin::command();
    command.current_dir(repository_root());
    command.args([
        "--json",
        "execute-canonical",
        fixture("canonical-host-capability-execute.air.json")
            .to_str()
            .expect("AIR path"),
        "--invocation-admission",
        fixture("canonical-host-capability-execute.invocation-admission.json")
            .to_str()
            .expect("admission path"),
        "--release",
        fixture("canonical-execute.release.json")
            .to_str()
            .expect("release path"),
        "--provenance",
        fixture("canonical-execute.provenance.json")
            .to_str()
            .expect("provenance path"),
    ]);
    let output = command.output().expect("execute canonical CLI");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the host-fulfilled fixture must settle and commit\nstdout: {stdout}\nstderr: {stderr}"
    );
    let committed: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("committed output is not JSON ({error}): {stdout}"));
    assert_eq!(
        committed,
        Value::String(
            "{\"note\":\"the second request is issued only after the first settles\"}".to_owned()
        ),
        "the committed Session Output is the settlement the host handed back"
    );
}
