# AIS op: `CALL_SKILL`

Status: **design spec**. The op is the keystone of cross-artifact
linking. This document is the implementation contract — the runtime
handler, capability admission, depth limit, provenance schema, and
failure modes below are normative.

For background on why this op exists at all, see
[`../skill-library-model.md`](../skill-library-model.md). For the
parallel `FLOW_CALL` (intra-artifact dispatch), see the existing op
definition at `apxm-core/src/ais_ops.td`.

## Purpose

`CALL_SKILL` invokes another skill *by manifest identity* —
`<skill_id>` or `<skill_id>@<version>` — rather than by raw
`.apxmobj` path. It is the classical-library analogue of an
unresolved symbol that the linker / loader binds at link or load
time. In APXM v1, binding is **lazy**: the runtime resolves the id
through the live `SkillLibrary` at execution time, and records the
resolved `(skill_id, version, artifact_hash)` triple in the
parent's provenance.

## Attributes

| Name | Type | Required | Description |
|---|---|---|---|
| `skill_id` | string | yes | `"apxm-orient"` or `"apxm-orient@0.2.0"`. Must match the canonical skill-id pattern (kebab-case, no path separators). |
| `args` | list[string] | no | Positional `args` to forward as the child skill's entry-flow input vector. Defaults to `[]`. |
| `input_names` | list[string] | no | Optional name vector. When present, the runtime maps the parent's named outputs in `input_names` order onto the child's positional `args`. |

`skill_id` is validated against the canonical skill-id regex before
any resolution attempt. Any of: `/`, `..`, `~`, control characters,
leading or trailing whitespace, empty string → rejected with
`InvalidSkillId(skill_id)` before the resolver runs.

## Resolution semantics

1. **Parse** `skill_id` into `(id, Option<version>)`.
2. **Resolve** through `SkillLibrary.find_executable(id, version)`.
   - `version = None` → `@latest` (highest SemVer with a present
     `.apxmobj`).
   - `version = Some(v)` → exact match; fails with
     `SkillVersionNotFound(id, v)` if absent.
3. **Admit** the resolved skill against the parent's capability
   grants — see "Capability admission" below.
4. **Dispatch** the child's entry DAG with the forwarded `args`,
   under a new execution context whose `parent_execution_id`,
   `parent_skill_id`, and `scope_id` are propagated from the parent
   (see [Track A.1 — provenance scope_id](../skill-library-model.md#provenance)).
5. **Return** the child's `node_output_map` namespaced under the
   `CALL_SKILL` op's instance id, so multiple linked calls in the
   same parent DAG cannot collide.

There is no separate "link" artifact step. Recording the resolved
`(skill_id, version, artifact_hash)` in the parent's provenance
gives reproducibility without a static-link archive format.

## Capability admission

The child skill's `required_capabilities` (declared in
`skill.toml`) must be a **subset** of the parent skill's effective
capability grant. The parent cannot widen its grant by calling a
child that requests a broader capability — this is the no-widen
invariant covered by `CapabilityPolicy::admits()`
(`apxm-skill/src/lib.rs`).

Concretely:

| Parent grant | Child declares | Outcome |
|---|---|---|
| `read_only` | `read_only` | ✓ admit |
| `sandboxed` | `read_only` | ✓ admit (narrower) |
| `sandboxed` | `sandboxed` | ✓ admit |
| `read_only` | `sandboxed` | ✗ reject (`CapabilityWiden`) |
| `Broader { admits: {"network"} }` | `Broader { admits: {"network"} }` | ✓ admit |
| `Broader { admits: {"network"} }` | `Broader { admits: {"fs_write", "network"} }` | ✗ reject |

Rejection happens **before** the child's entry DAG dispatches. The
parent's execution is failed with a typed `CapabilityWiden` error
that names the requested capability and the parent's current
grant. No partial-execution side effects.

## Depth limit

`MAX_CALL_SKILL_DEPTH` is a separate constant from
`MAX_FLOW_CALL_DEPTH` (which gates intra-artifact `FLOW_CALL`).
Recommended v1 value: **8**. Rationale: intra-artifact `FLOW_CALL`
chains can be deep because they're authored against a single
manifest. Cross-skill calls compose independent contracts, and a
depth of 8 is the largest call tree that has shown up in real
agent traces without being a sign of an authoring mistake.

Exceeding the limit fails the offending `CALL_SKILL` with
`CallSkillDepthExceeded(current_depth, max_depth)`.

## Provenance schema

Every `CALL_SKILL` invocation appends a record to the parent's
provenance log:

```json
{
  "op": "call_skill",
  "instance_id": "<unique-within-parent-dag>",
  "skill_id": "apxm-orient",
  "requested_version": null,
  "resolved": {
    "skill_id": "apxm-orient",
    "version": "0.2.0",
    "artifact_hash": "blake3:<hex>"
  },
  "child_execution_id": "<uuid>",
  "scope_id": "<inherited from parent>",
  "started_at": "2026-05-26T13:42:11.103Z",
  "finished_at": "2026-05-26T13:42:11.847Z",
  "outcome": "ok"
}
```

`resolved` is what gives v1 reproducibility without a static-link
artifact: a parent execution can be replayed against a known
`SkillLibrary` snapshot by pinning the resolved hash, even if the
sibling pack catalog has moved on.

## Failure modes

Every failure is a typed error; no panics, no silent fallbacks.

| Error | Trigger |
|---|---|
| `InvalidSkillId(s)` | Path-shaped id, `..`-bearing id, empty, control chars. |
| `SkillNotFound(id)` | No pack provides the id. |
| `SkillVersionNotFound(id, v)` | Id resolved but no `.apxmobj` matches the requested version. |
| `CapabilityWiden(requested, granted)` | Child requires a capability the parent has not been granted. |
| `CallSkillDepthExceeded(d, max)` | Nested call exceeds `MAX_CALL_SKILL_DEPTH`. |
| `ChildExecutionFailed(child_id, inner)` | Child's entry DAG raised; inner error is the child's failure shape. |

The parent's `node_output_map` does not receive a partial result
for any of these — the `CALL_SKILL` node fails the parent DAG (or
is routed to whatever recovery edge the author wired).

## Output namespacing (T3.2)

A `CALL_SKILL` instance's outputs land under
`outputs.<call_skill_node_id>.<child_output_name>` in the
parent's `node_output_map`. Two `CALL_SKILL` nodes in the same
parent DAG with the same `skill_id` cannot collide. This is the
T3.2 contract — see `FLOW_CALL` for the in-artifact analogue.

## Implementation checklist

When implementing `CALL_SKILL`, the following must all land in the
same PR:

- `apxm-core/src/ais_ops.td` — new op definition with the three
  attributes. Required: `dekk apxm build-dialect && dekk apxm
  codegen`.
- `apxm-ais/src/operations/definitions.rs` — `CallSkill` variant.
- `apxm-runtime/src/executor/handlers/call_skill.rs` — new
  handler implementing the seven steps above.
- `apxm-server/src/skills.rs` — depth-counter plumbing on the
  invocation path; provenance writer extended with the resolved
  triple.
- `apxm-core/src/constants.rs` — `MAX_CALL_SKILL_DEPTH = 8`.
- `apxm-server/tests/` — server-isolation regression cover
  (Track A.3): path-shaped id, `../`-bearing id, missing version,
  capability superset all fail clean.
- `apxm-frontend/python/apxm/` — `call_skill(skill_id, ...)`
  binding emitted by codegen.

## Out of scope (v1)

- **Static linking.** Lazy resolution + provenance pinning gives
  reproducibility without a multi-object archive format. A static
  linker phase only makes sense if profiling shows resolution
  cost matters.
- **Cyclic call detection beyond the depth limit.** The depth
  limit catches cycles by exhaustion. A graph-cycle detector
  could be added if real workloads start hitting depth from
  cycles.
- **Caller-supplied capability grants.** The child's
  `required_capabilities` are checked against the parent's
  effective grant. A parent cannot pass a *different* grant to
  the child; if you need that, model it as two separate skills
  with the appropriate grants set in their own manifests.
