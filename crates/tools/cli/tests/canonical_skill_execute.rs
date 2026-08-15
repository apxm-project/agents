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

/// The local discovery roots are working-directory relative, so the CLI runs
/// from the repository root exactly as `dekk agents execute-canonical` does.
fn execute_skill_fixture() -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apxm"));
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

fn node_outcome<'a>(result: &'a Value, node_id: &str) -> &'a Value {
    result["results"]["node_outcomes"]
        .as_array()
        .expect("node outcomes array")
        .iter()
        .find(|outcome| outcome["node_id"] == node_id)
        .unwrap_or_else(|| panic!("no node outcome for {node_id}: {result}"))
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
    assert_eq!(result["status"], "completed");
    assert_eq!(result["commit"]["status"], "committed");

    let listed = node_outcome(&result, "fixture.capability.list_skills");
    assert_eq!(listed["kind"], "capability.invoke");
    assert_eq!(
        listed["outcome"]["status"], "completed",
        "an admitted Tool(\"list_skills\") reaches ListSkillsCapability: {result}"
    );
    let cards = listed["outcome"]["result"]
        .as_str()
        .expect("capability result text");
    assert!(
        cards.contains("\"skill_id\": \"context\""),
        "the project root publishes a card per checked-in skill: {cards}"
    );
    // Discovery is metadata-only. The listing advertises `context`; it must not
    // have loaded its body, which is what `activates_on_listing: false` means.
    assert!(
        !cards.contains("# APXM Context"),
        "listing a root must not load any instruction body: {cards}"
    );

    let searched = node_outcome(&result, "fixture.capability.search_skills");
    assert_eq!(searched["kind"], "capability.invoke");
    assert_eq!(
        searched["outcome"]["status"], "completed",
        "an admitted Tool(\"search_skills\") reaches SearchSkillsCapability: {result}"
    );
    let search_cards = searched["outcome"]["result"]
        .as_str()
        .expect("capability result text");
    assert!(
        search_cards.contains("\"skill_id\": \"context\""),
        "searching by metadata returns the matching skill card: {search_cards}"
    );
    assert!(
        !search_cards.contains("# APXM Context"),
        "searching skills must remain metadata-only: {search_cards}"
    );

    let body = node_outcome(&result, "fixture.capability.read_skill");
    assert_eq!(
        body["outcome"]["status"], "completed",
        "an admitted Tool(\"read_skill\") reaches ReadSkillCapability: {result}"
    );
    let body = body["outcome"]["result"]
        .as_str()
        .expect("capability result text");
    assert!(
        body.contains("name: context"),
        "reading a skill returns its instruction document: {body}"
    );
    assert!(
        body.contains("# APXM Context"),
        "reading a skill returns the body listing withheld: {body}"
    );
}
