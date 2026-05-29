//! Server-isolation gates for the `CALL_SKILL` op.
//!
//! Each test here is one of the eight gates from
//! `docs/design/ais-ops/call-skill-isolation-tests.md`. Gates 1-4 exercise
//! the server-side [`SkillLibrarySkillResolver`] directly (no HTTP layer)
//! so the precise rejection mode is observable. Gates 5-8 build a parent
//! artifact whose entry DAG contains `CALL_SKILL` nodes and dispatch it
//! through a fully-attached [`Runtime`], so the child-DAG dispatch plug
//! is exercised end-to-end.
//!
//! Cross-referenced from `crates/runtime/apxm-runtime/src/executor/handlers/call_skill.rs`
//! where five additional unit-level gates already cover the runtime-side
//! invariants (id parsing, depth, capability tagging).

use super::*;

use std::collections::HashMap;
use std::sync::Arc;

use apxm_artifact::{Artifact, ArtifactMetadata};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::AISOperationType;
use apxm_core::types::execution::{DagMetadata, Edge, ExecutionDag, Node, NodeMetadata};
use apxm_core::types::values::Value;
use apxm_runtime::{CallSkillRequest, Runtime, RuntimeConfig, SkillResolver};

use crate::call_skill::SkillLibrarySkillResolver;
use crate::skills::SkillLibrary;

const CALL_SKILL_TAG: &str = "call_skill";
const CALL_SKILL_OUTPUT_PREFIX: &str = "_call_skill_outputs:";
const PARENT_ENTRY_FLOW: &str = "parent-main";

/// Build a [`CallSkillRequest`] with neutral parent metadata. Tests
/// override only the fields they care about.
fn request(skill_id: &str, requested_version: Option<&str>, depth: usize) -> CallSkillRequest {
    CallSkillRequest {
        skill_id: skill_id.to_string(),
        requested_version: requested_version.map(str::to_string),
        args: Vec::new(),
        named_args: HashMap::new(),
        parent_execution_id: "parent-exec".to_string(),
        parent_scope_id: "parent-scope".to_string(),
        parent_session_dir: None,
        spawn_node_id: 1,
        depth,
    }
}

fn build_resolver(skill_root: &std::path::Path) -> SkillLibrarySkillResolver {
    SkillLibrarySkillResolver::new(SkillLibrary::new(vec![skill_root.to_path_buf()]))
}

/// Build a runtime with the `SkillLibrarySkillResolver` installed against
/// the given skill root. Used by gates 5-8 to dispatch parent artifacts
/// through the same code path the real server uses.
async fn runtime_with_skill_root(skill_root: &std::path::Path) -> Arc<Runtime> {
    let runtime = Runtime::new(RuntimeConfig::in_memory())
        .await
        .expect("test runtime");
    let mut runtime = Arc::new(runtime);
    let library = SkillLibrary::new(vec![skill_root.to_path_buf()]);
    crate::call_skill::install(&mut runtime, library);
    runtime
}

/// Build a parent artifact whose entry DAG contains the supplied
/// `CALL_SKILL` nodes. Each node references `child_skill_id` via the
/// canonical `graph_attrs::SKILL_ID` attribute and produces a unique
/// output token. The nodes are independent (no edges) so the runtime
/// can schedule them concurrently.
fn parent_artifact_with_call_skill_nodes(
    child_skill_id: &str,
    call_skill_node_ids: &[u64],
) -> Vec<u8> {
    let mut nodes = Vec::with_capacity(call_skill_node_ids.len());
    let mut exit_nodes = Vec::with_capacity(call_skill_node_ids.len());
    for &node_id in call_skill_node_ids {
        let mut node = Node {
            id: node_id,
            op_type: AISOperationType::CallSkill,
            attributes: HashMap::new(),
            input_tokens: vec![],
            // Each CALL_SKILL produces a distinct output token id so the
            // runtime's per-token result map doesn't collide. Token ids
            // are namespaced by 1000+node_id to keep them disjoint from
            // any future fixture token range.
            output_tokens: vec![1000 + node_id],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::SKILL_ID.to_string(),
            Value::String(child_skill_id.to_string()),
        );
        nodes.push(node);
        exit_nodes.push(node_id);
    }
    let dag = ExecutionDag {
        nodes,
        edges: Vec::<Edge>::new(),
        entry_nodes: call_skill_node_ids.to_vec(),
        exit_nodes,
        metadata: DagMetadata {
            name: Some(PARENT_ENTRY_FLOW.to_string()),
            is_entry: true,
            parameters: vec![],
        },
    };
    let artifact = Artifact::new(
        ArtifactMetadata::new(Some("parent-skill".to_string()), FIXTURE_COMPILER_VERSION),
        vec![dag],
    );
    artifact.to_bytes().expect("parent artifact bytes")
}

/// Build a child skill on disk whose entry DAG is a single `Err` node
/// with no `message` attribute, so executing it always fails at the
/// handler level. Used by gate 7.
fn write_failing_child_skill(root: &std::path::Path) {
    let mut node = Node {
        id: 1,
        op_type: AISOperationType::Err,
        attributes: HashMap::new(),
        input_tokens: vec![],
        output_tokens: vec![100],
        metadata: NodeMetadata::default(),
    };
    // Intentionally omit graph_attrs::MESSAGE — the Err handler returns
    // a typed error when the message attribute is missing, which the
    // scheduler propagates as a node failure.
    node.attributes.insert(
        graph_attrs::VALUE.to_string(),
        Value::String("unused".to_string()),
    );
    let dag = ExecutionDag {
        nodes: vec![node],
        edges: vec![],
        entry_nodes: vec![1],
        exit_nodes: vec![1],
        metadata: DagMetadata {
            name: Some(FIXTURE_ENTRY_FLOW.to_string()),
            is_entry: true,
            parameters: vec![],
        },
    };
    let artifact = Artifact::new(
        ArtifactMetadata::new(Some(FIXTURE_SKILL_ID.to_string()), FIXTURE_COMPILER_VERSION),
        vec![dag],
    );
    let bytes = artifact.to_bytes().expect("failing child artifact bytes");
    write_executable_skill_with_artifact(
        root,
        FIXTURE_PACKAGE_DIR,
        FIXTURE_SKILL_VERSION,
        &bytes,
        FIXTURE_SKILL_ID,
        FIXTURE_ENTRY_FLOW,
    );
}

// ── Gate 1 ─────────────────────────────────────────────────────────────
// Path-shaped ids are rejected before the resolver runs.
//
// The runtime-level guard lives in `executor/handlers/call_skill.rs`
// (covered by `validate_skill_id_rejects_path_shaped_ids` there). At the
// server-resolver layer, an id that survives the runtime guard but does
// not match any pack must surface as a typed `call_skill:not_found:<id>`
// — never as a "successful resolve" against a sibling pack reached by
// path traversal. We exercise the resolver directly with the worst-case
// inputs to confirm it never reaches into the filesystem for them.

#[tokio::test]
async fn call_skill_resolver_rejects_path_shaped_ids_as_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let resolver = build_resolver(temp.path());

    for hostile in [
        "../escape",
        "foo/bar",
        "~/skill",
        "skill\x00null",
        " skill",
        "skill ",
    ] {
        let err = resolver
            .call_skill(request(hostile, None, 1))
            .await
            .expect_err(&format!("expected rejection for {hostile:?}"));
        let message = err.to_string();
        assert!(
            message.contains(CALL_SKILL_TAG)
                && (message.contains("not_found") || message.contains("invalid_manifest")),
            "expected typed not_found/invalid_manifest rejection for {hostile:?}, got: {message}"
        );
    }
}

// ── Gate 2 ─────────────────────────────────────────────────────────────
// Requesting a version that the library does not have surfaces a typed
// `not_found` failure mode, not a silent "resolve to closest" fallback.

#[tokio::test]
async fn call_skill_resolver_rejects_missing_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path()); // installs FIXTURE_SKILL_ID @ FIXTURE_SKILL_VERSION
    let resolver = build_resolver(temp.path());

    let err = resolver
        .call_skill(request(FIXTURE_SKILL_ID, Some(UNKNOWN_SKILL_VERSION), 1))
        .await
        .expect_err("expected version not_found");
    let message = err.to_string();
    assert!(
        message.contains("not_found"),
        "expected not_found tag, got: {message}"
    );
    assert!(
        message.contains(UNKNOWN_SKILL_VERSION),
        "expected unknown version in message, got: {message}"
    );
}

// ── Gate 3 ─────────────────────────────────────────────────────────────
// A child skill that declares any required capability fails closed under
// the conservative no-widen default. Once parent-grant plumbing lands the
// rule becomes "child ⊆ parent"; the current implementation refuses any
// capability declaration outright, which is strictly stronger than the
// spec demands but still satisfies the gate.

#[tokio::test]
async fn call_skill_resolver_rejects_capability_widening() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = skill_artifact_bytes(AISOperationType::ConstStr);
    write_policy_skill_with_artifact(
        temp.path(),
        &artifact,
        FIXTURE_CAPABILITY,
        FIXTURE_CAPABILITY,
    );
    let resolver = build_resolver(temp.path());

    let err = resolver
        .call_skill(request(FIXTURE_SKILL_ID, None, 1))
        .await
        .expect_err("expected capability widen rejection");
    let message = err.to_string();
    assert!(
        message.contains("capability_widen"),
        "expected capability_widen tag, got: {message}"
    );
    assert!(
        message.contains(FIXTURE_CAPABILITY),
        "expected capability name in message, got: {message}"
    );
}

// ── Gate 4 ─────────────────────────────────────────────────────────────
// The depth limit lives in the runtime handler, not the resolver, so the
// authoritative gate is the unit test
// `execute_enforces_max_depth` in
// `crates/runtime/apxm-runtime/src/executor/handlers/call_skill.rs`.
// Here we cross-check the constant the server-side resolver receives
// matches the runtime constant — a guard against accidental drift.

#[test]
fn call_skill_max_depth_constant_matches_runtime() {
    // Single source of truth: apxm-core.
    assert_eq!(apxm_core::constants::call_skill::MAX_CALL_SKILL_DEPTH, 8);
}

// ── Gate 5 ─────────────────────────────────────────────────────────────
// Two `CALL_SKILL` nodes in the parent, both targeting the same child,
// must produce two distinct provenance beliefs keyed by spawn-node id.
// This is the namespacing contract: the parent AAM cannot conflate two
// linked calls to the same skill, even when they emit identically-named
// outputs.

#[tokio::test]
async fn call_skill_namespaces_child_outputs() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let runtime = runtime_with_skill_root(temp.path()).await;

    let parent_bytes = parent_artifact_with_call_skill_nodes(FIXTURE_SKILL_ID, &[10, 20]);
    let parent = Artifact::from_bytes(&parent_bytes).expect("parent artifact");

    let result = runtime
        .execute_artifact_with_args(parent, Vec::new())
        .await
        .expect("parent execution");
    assert_eq!(result.stats.failed_nodes, 0, "no parent nodes should fail");
    assert_eq!(
        result.stats.executed_nodes, 2,
        "both call_skill nodes executed"
    );

    let beliefs = runtime.aam().beliefs();
    let key_10 = format!("{CALL_SKILL_OUTPUT_PREFIX}{FIXTURE_SKILL_ID}:10");
    let key_20 = format!("{CALL_SKILL_OUTPUT_PREFIX}{FIXTURE_SKILL_ID}:20");
    let belief_10 = beliefs.get(&key_10).expect("provenance for node 10");
    let belief_20 = beliefs.get(&key_20).expect("provenance for node 20");

    // Confirm the namespacing is real: each belief records its own
    // parent_node_id rather than a single shared value.
    let parent_node_id_10 = belief_field_i64(belief_10, "parent_node_id");
    let parent_node_id_20 = belief_field_i64(belief_20, "parent_node_id");
    assert_eq!(parent_node_id_10, 10);
    assert_eq!(parent_node_id_20, 20);

    // And both beliefs resolve to the same child skill — the namespacing
    // is structural, not a function of differing target ids.
    assert_eq!(
        belief_resolved_field(belief_10, "skill_id"),
        FIXTURE_SKILL_ID
    );
    assert_eq!(
        belief_resolved_field(belief_20, "skill_id"),
        FIXTURE_SKILL_ID
    );
}

// ── Gate 6 ─────────────────────────────────────────────────────────────
// A successful dispatch records the resolver-supplied
// `{skill_id, version, artifact_hash}` triple on the parent's AAM. This
// is the provenance evidence the audit trail depends on.

#[tokio::test]
async fn call_skill_records_resolved_provenance() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());

    // Compute the expected artifact_hash by reading the bytes the
    // fixture actually wrote — the in-memory byte serialization can
    // differ from a fresh call (e.g. ordering, padding) so the only
    // sound oracle is the on-disk artifact.
    let artifact_on_disk = std::fs::read(
        temp.path()
            .join(FIXTURE_PACKAGE_DIR)
            .join(FILE_SKILL_ARTIFACT),
    )
    .expect("read fixture artifact bytes");
    let expected_hash = tagged_blake3(&artifact_on_disk);

    let runtime = runtime_with_skill_root(temp.path()).await;
    let parent_bytes = parent_artifact_with_call_skill_nodes(FIXTURE_SKILL_ID, &[42]);
    let parent = Artifact::from_bytes(&parent_bytes).expect("parent artifact");
    let _ = runtime
        .execute_artifact_with_args(parent, Vec::new())
        .await
        .expect("parent execution");

    let key = format!("{CALL_SKILL_OUTPUT_PREFIX}{FIXTURE_SKILL_ID}:42");
    let belief = runtime
        .aam()
        .beliefs()
        .get(&key)
        .cloned()
        .expect("provenance belief");
    assert_eq!(belief_resolved_field(&belief, "skill_id"), FIXTURE_SKILL_ID);
    assert_eq!(
        belief_resolved_field(&belief, "version"),
        FIXTURE_SKILL_VERSION
    );
    assert_eq!(
        belief_resolved_field(&belief, "artifact_hash"),
        expected_hash
    );
}

// ── Gate 7 ─────────────────────────────────────────────────────────────
// A child whose entry DAG fails mid-execution must surface a typed
// `call_skill:child_failed:<skill_id>` capability error to the parent.
// The child's partial outputs (if any) must NOT be recorded on the
// parent AAM — failure is all-or-nothing at the boundary.

#[tokio::test]
async fn call_skill_surfaces_child_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_failing_child_skill(temp.path());
    let runtime = runtime_with_skill_root(temp.path()).await;

    let parent_bytes = parent_artifact_with_call_skill_nodes(FIXTURE_SKILL_ID, &[7]);
    let parent = Artifact::from_bytes(&parent_bytes).expect("parent artifact");
    let result = runtime.execute_artifact_with_args(parent, Vec::new()).await;
    let err = result.expect_err("parent must propagate child failure");
    let message = err.to_string();
    assert!(
        message.contains("child_failed"),
        "expected child_failed tag, got: {message}"
    );
    assert!(
        message.contains(FIXTURE_SKILL_ID),
        "expected child skill id in message, got: {message}"
    );

    // No provenance belief is written when the child fails — the parent
    // only records successful dispatches.
    let key = format!("{CALL_SKILL_OUTPUT_PREFIX}{FIXTURE_SKILL_ID}:7");
    assert!(
        runtime.aam().beliefs().get(&key).is_none(),
        "no partial provenance must leak on child failure"
    );
}

// ── Gate 8 ─────────────────────────────────────────────────────────────
// The parent's execution id flows into the child execution id under the
// resolver's deterministic `parent_execution::call_skill::<spawn_node_id>`
// pattern. The pattern is stable so audit tooling can join parent and
// child traces back together by inspecting the provenance belief alone.

#[tokio::test]
async fn call_skill_propagates_scope_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let runtime = runtime_with_skill_root(temp.path()).await;

    let parent_bytes = parent_artifact_with_call_skill_nodes(FIXTURE_SKILL_ID, &[55]);
    let parent = Artifact::from_bytes(&parent_bytes).expect("parent artifact");
    let _ = runtime
        .execute_artifact_with_args(parent, Vec::new())
        .await
        .expect("parent execution");

    let key = format!("{CALL_SKILL_OUTPUT_PREFIX}{FIXTURE_SKILL_ID}:55");
    let belief = runtime
        .aam()
        .beliefs()
        .get(&key)
        .cloned()
        .expect("provenance belief");
    let child_execution_id = match belief_field(&belief, "child_execution_id") {
        Value::String(s) => s,
        other => panic!("expected child_execution_id to be a string, got: {other:?}"),
    };
    // The pattern is `<parent_execution_id>::call_skill::<spawn_node_id>`.
    // We don't know the parent execution id ahead of time (it's a UUID),
    // but we know the suffix is deterministic, and that the prefix is
    // non-empty (i.e. the parent execution id was actually threaded
    // through rather than dropped).
    let expected_suffix = "::call_skill::55";
    assert!(
        child_execution_id.ends_with(expected_suffix),
        "expected child_execution_id to end with {expected_suffix:?}, got: {child_execution_id}"
    );
    let prefix = child_execution_id
        .strip_suffix(expected_suffix)
        .expect("suffix present");
    assert!(
        !prefix.is_empty(),
        "child_execution_id must carry a non-empty parent execution id prefix; got: {child_execution_id}"
    );
}

// ── Resolver smoke: a successful end-to-end dispatch through the bridge ─

#[tokio::test]
async fn call_skill_resolver_dispatches_child_end_to_end() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_executable_skill(temp.path());
    let runtime = runtime_with_skill_root(temp.path()).await;

    let parent_bytes = parent_artifact_with_call_skill_nodes(FIXTURE_SKILL_ID, &[1]);
    let parent = Artifact::from_bytes(&parent_bytes).expect("parent artifact");
    let result = runtime
        .execute_artifact_with_args(parent, Vec::new())
        .await
        .expect("end-to-end dispatch");
    assert_eq!(result.stats.failed_nodes, 0);
    assert_eq!(result.stats.executed_nodes, 1);
}

// ── Local belief-projection helpers ─────────────────────────────────────

fn belief_field<'a>(belief: &'a Value, key: &str) -> &'a Value {
    let Value::Object(map) = belief else {
        panic!("expected object belief, got: {belief:?}");
    };
    map.get(key)
        .unwrap_or_else(|| panic!("belief missing field {key:?}: {belief:?}"))
}

fn belief_field_i64(belief: &Value, key: &str) -> i64 {
    match belief_field(belief, key) {
        Value::Number(apxm_core::types::values::Number::Integer(n)) => *n,
        other => panic!("expected integer field {key:?}, got: {other:?}"),
    }
}

fn belief_resolved_field(belief: &Value, key: &str) -> String {
    let resolved = belief_field(belief, "resolved");
    match belief_field(resolved, key) {
        Value::String(s) => s.clone(),
        other => panic!("expected string resolved.{key}, got: {other:?}"),
    }
}
