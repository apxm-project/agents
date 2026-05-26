//! Server-isolation gates for the `CALL_SKILL` op.
//!
//! Each test here is one of the eight gates from
//! `docs/design/ais-ops/call-skill-isolation-tests.md`. Gates 1-4 exercise
//! the server-side [`SkillLibrarySkillResolver`] directly (no HTTP layer)
//! so the precise rejection mode is observable. Gates 5-8 require the
//! child-DAG dispatch plug that is intentionally *not* installed yet
//! (the resolver fails closed with `child_dispatch_unwired`); those gates
//! are tracked as `#[ignore]` placeholders so the missing infrastructure
//! is visible in the test report rather than silently absent.
//!
//! Cross-referenced from `crates/runtime/apxm-runtime/src/executor/handlers/call_skill.rs`
//! where five additional unit-level gates already cover the runtime-side
//! invariants (id parsing, depth, capability tagging).

use super::*;

use std::collections::HashMap;

use apxm_runtime::{CallSkillRequest, SkillResolver};

use crate::call_skill::SkillLibrarySkillResolver;
use crate::skills::SkillLibrary;

const CALL_SKILL_TAG: &str = "call_skill";

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

    for hostile in ["../escape", "foo/bar", "~/skill", "skill\x00null", " skill", "skill "] {
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
    write_policy_skill_with_artifact(temp.path(), &artifact, FIXTURE_CAPABILITY, FIXTURE_CAPABILITY);
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

// ── Gates 5-8 (pending child-dispatch wiring) ──────────────────────────
//
// These gates require the child-skill dispatcher plug to be installed on
// the server-side resolver. Today the resolver explicitly refuses to
// fabricate a synthetic dispatch path; it returns
// `call_skill:child_dispatch_unwired:<id>` after a successful resolve +
// admit. When the plug lands the `#[ignore]` markers come off and the
// bodies fill in.

#[tokio::test]
#[ignore = "child-dispatch plug not yet installed; tracked in call-skill.md Step 6"]
async fn call_skill_namespaces_child_outputs() {
    // Two CALL_SKILL nodes in the parent, both targeting the same child,
    // both declaring output `result`. Parent's node_output_map must have
    // both children's outputs namespaced under their respective spawn_node_id.
}

#[tokio::test]
#[ignore = "child-dispatch plug not yet installed; tracked in call-skill.md Step 6"]
async fn call_skill_records_resolved_provenance() {
    // Confirm the resolver records `resolved.{skill_id,version,artifact_hash}`
    // in the parent's AAM provenance after a successful end-to-end dispatch.
}

#[tokio::test]
#[ignore = "child-dispatch plug not yet installed; tracked in call-skill.md Step 6"]
async fn call_skill_surfaces_child_failure() {
    // Child entry DAG raises a typed RuntimeError mid-execution. Parent
    // must receive a typed `child_failed:<child_id>` and the child's
    // partial node_output_map must NOT leak into the parent.
}

#[tokio::test]
#[ignore = "child-dispatch plug not yet installed; tracked in call-skill.md Step 6"]
async fn call_skill_propagates_scope_id() {
    // Parent scope_id propagates to the child execution's
    // SkillExecutionProvenance.scope_id and to every event the child
    // emits on the shared event stream.
}

// ── Resolver smoke: the "unwired" failure is observable and typed ──────

#[tokio::test]
async fn call_skill_resolver_returns_child_dispatch_unwired_after_admit() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Capability-free skill so admission succeeds and we reach the
    // dispatch step.
    write_executable_skill(temp.path());
    let resolver = build_resolver(temp.path());

    let err = resolver
        .call_skill(request(FIXTURE_SKILL_ID, None, 1))
        .await
        .expect_err("expected child_dispatch_unwired");
    let message = err.to_string();
    assert!(
        message.contains("child_dispatch_unwired"),
        "expected child_dispatch_unwired tag, got: {message}"
    );
    assert!(
        message.contains(FIXTURE_SKILL_ID),
        "expected skill id in message, got: {message}"
    );
    assert!(
        message.contains(FIXTURE_SKILL_VERSION),
        "expected resolved version in message, got: {message}"
    );
    assert!(
        message.contains("blake3:"),
        "expected resolved artifact_hash in message, got: {message}"
    );
}
