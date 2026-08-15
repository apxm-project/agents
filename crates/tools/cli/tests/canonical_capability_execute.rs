//! Live CLI coverage for the admitted Capability port: the shipped
//! `execute-canonical` path running a real tool, and denying one that the local
//! composition root does not admit.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

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
fn execute_capability_fixture() -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apxm_dev"));
    command.current_dir(repository_root());
    command.args([
        "--json",
        "execute-canonical",
        fixture("canonical-capability-execute.air.json")
            .to_str()
            .expect("AIR fixture path"),
        "--invocation-admission",
        fixture("canonical-capability-execute.invocation-admission.json")
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
