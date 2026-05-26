# `CALL_SKILL` server-isolation test contract

This document is the **test contract** for `CALL_SKILL`. Each test
below must land **in the same PR** that introduces the op
(see [`call-skill.md`](call-skill.md)). The implementer is
reviewer-rejected if any of these gates is missing.

Tests live alongside the existing admission tests at
`crates/tools/apxm-server/src/tests/`. Recommended new module name:
`tests/call_skill_isolation.rs`, registered from
`tests.rs` next to `skills_admission`.

## Gate 1 — path-shaped `skill_id` is rejected before resolution

```rust
#[tokio::test]
async fn call_skill_rejects_path_shaped_id() {
    // Parent skill author writes a CALL_SKILL op with skill_id = "../escape"
    // (or "foo/bar", or "../../etc/passwd"). The runtime MUST reject at
    // `validate_skill_id` before the resolver runs.
    //
    // Expected: typed `InvalidSkillId` error in the parent's failure;
    // no filesystem access attempted; no panic.
}
```

Coverage targets:
- `"../escape"` (parent-relative traversal)
- `"foo/bar"` (path separator)
- `"~/skill"` (home expansion attempt)
- `""` (empty string)
- `"skill\x00with-null"` (control characters)
- `" skill"` / `"skill "` (whitespace padding)

## Gate 2 — version that does not exist is rejected

```rust
#[tokio::test]
async fn call_skill_rejects_missing_version() {
    // Parent calls "apxm-orient@99.0.0". `SkillLibrary.find_executable`
    // resolves the id but no .apxmobj matches the version.
    //
    // Expected: typed `SkillVersionNotFound("apxm-orient", "99.0.0")`;
    // parent execution failed; provenance records the attempted
    // resolution.
}
```

## Gate 3 — capability widening is rejected

```rust
#[tokio::test]
async fn call_skill_rejects_capability_widening() {
    // Parent has only `read_only`. Child declares
    // `required_capabilities = ["network"]`.
    //
    // Expected: typed `CapabilityWiden`; rejection happens BEFORE the
    // child's entry DAG dispatches; no partial side effects.
}
```

## Gate 4 — depth limit is enforced

```rust
#[tokio::test]
async fn call_skill_enforces_max_depth() {
    // Chain CALL_SKILL N times where N > MAX_CALL_SKILL_DEPTH.
    // The (max_depth + 1)-th call MUST fail with
    // `CallSkillDepthExceeded(max_depth + 1, max_depth)`.
}
```

This is independent of `MAX_FLOW_CALL_DEPTH = 100`. Use the
recommended v1 value `MAX_CALL_SKILL_DEPTH = 8` (see
[`call-skill.md`](call-skill.md)).

## Gate 5 — output namespacing (T3.2)

```rust
#[tokio::test]
async fn call_skill_namespaces_child_outputs() {
    // Parent DAG has two CALL_SKILL nodes, both calling the same
    // skill, both declaring output `result`.
    //
    // Expected: parent's `node_output_map` has both children's
    // outputs under their respective CALL_SKILL node ids; no
    // collision; no last-write-wins.
}
```

## Gate 6 — provenance records the resolved triple

```rust
#[tokio::test]
async fn call_skill_records_resolved_provenance() {
    // Parent calls "apxm-orient" (no version pin). Server resolves
    // to "apxm-orient@0.2.0" with artifact_hash = "blake3:abc..".
    //
    // Expected: parent's provenance log has a `call_skill` entry
    // with `resolved.skill_id`, `resolved.version`,
    // `resolved.artifact_hash` populated. Replaying the parent
    // against a SkillLibrary snapshot pinned to that hash
    // succeeds deterministically.
}
```

## Gate 7 — child failure surfaces typed

```rust
#[tokio::test]
async fn call_skill_surfaces_child_failure() {
    // Child skill's entry DAG raises a typed error mid-execution.
    //
    // Expected: parent receives `ChildExecutionFailed(child_id,
    // inner)`; child's execution_id is recorded; child's partial
    // node_output_map is NOT exposed to the parent's
    // node_output_map (failure semantics, not partial outputs).
}
```

## Gate 8 — `scope_id` propagation

```rust
#[tokio::test]
async fn call_skill_propagates_scope_id() {
    // Parent execution has scope_id = Some("scope-abc"). Child
    // execution started via CALL_SKILL MUST inherit the same
    // scope_id (set by Track A.1 sub-item 2 on
    // SkillExecutionProvenance).
    //
    // Expected: child's ExecutionRecord.scope_id == parent's
    // scope_id; child events carry the same scope on the event
    // stream.
}
```

## Negative / regression

Existing isolation guarantees must remain intact. These are
**not new** but should be in the same PR's regression run:

- Server still refuses to accept raw AIR from a client; only
  `/v1/execute` accepts a server-built request, and
  `/v1/skills/{id}/execute` resolves by manifest id.
- Symlink escapes from a pack root still fail in
  `find_manifest_dirs`.
- Session root under `~/.apxm/sessions/` cannot be redirected by
  a client-supplied path.

## Why these gates ship with A.2 and not after

The plan body said "BEFORE T3.3 ships, not after." The reason:
each of these gates encodes a property the runtime must hold
*from the first commit that introduces CALL_SKILL*. Adding them
later means there is a window in `main` where the op exists but
no test enforces the invariants — exactly the kind of window
that produced past dual-naming and silent-fallback incidents in
this codebase (`feedback_attribute_dual_naming`,
`feedback_no_legacy_no_fallback`). The PR that adds CALL_SKILL is
also the PR that promises these gates hold.
