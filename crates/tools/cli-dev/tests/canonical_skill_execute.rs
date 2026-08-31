//! Live CLI coverage for the skill discovery capabilities: the shipped
//! `execute-canonical` path listing and searching a discovery root, then
//! loading one skill body, through admitted `capability.invoke` nodes.
//!
//! These three ids were once allowlisted with no handler behind them, which is
//! why they were removed. The claim that they are back for real is only worth
//! anything if a skill is discoverable and readable through the same admitted
//! path any other capability takes, in the shipped binary, against a real
//! discovery root — which is exactly what this test does. It reads `.agents/skills`, the
//! repository's own project-tier root, so no fixture stands in for the thing
//! being demonstrated.

use std::path::PathBuf;

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

/// The local discovery roots are working-directory relative, so the CLI runs
/// from the repository root exactly as `dekk agents execute-canonical` does.
fn execute_skill_fixture() -> std::process::Output {
    let mut command = apxm_dev_bin::command();
    command.current_dir(repository_root());
    command.args([
        "--json",
        "execute-canonical",
        fixture("canonical-skill-execute.air.json")
            .to_str()
            .expect("AIR fixture path"),
        "--invocation-admission",
        fixture("canonical-skill-execute.invocation-admission.json")
            .to_str()
            .expect("admission fixture path"),
        "--release",
        fixture("canonical-execute.release.json")
            .to_str()
            .expect("release fixture path"),
        "--provenance",
        fixture("canonical-execute.provenance.json")
            .to_str()
            .expect("provenance fixture path"),
    ]);
    command.output().expect("execute canonical CLI")
}

fn envelope_content(value: &Value) -> String {
    // The local execution projection may carry the capability Value as a
    // JSON-encoded string; the capability contract itself is the envelope
    // object. Accept both transport representations while asserting the same
    // trust boundary.
    let wire = match value {
        Value::String(encoded) => {
            serde_json::from_str(encoded).expect("decode untrusted skill envelope")
        }
        other => other.clone(),
    };
    assert_eq!(wire["kind"], "untrusted_content");
    assert_eq!(wire["trust"], "untrusted");
    wire["items"][0]["content"]
        .as_str()
        .expect("untrusted skill envelope content")
        .to_owned()
}

#[test]
fn an_authored_capability_invoke_lists_and_reads_a_real_skill() {
    let output = execute_skill_fixture();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("canonical result JSON");
    let committed_content = envelope_content(&result);
    assert!(
        committed_content.contains("name: context") && committed_content.contains("# APXM Context"),
        "the typed committed Session Output contains the skill body: {committed_content}"
    );
}
