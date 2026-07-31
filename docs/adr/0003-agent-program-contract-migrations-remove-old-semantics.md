---
status: accepted
date: 2026-07-15
owner: APXM agents
amended_by: ADR-0014
amended: 2026-07-22
---

# Agent Program contract migrations remove old semantics

ADR-0014 preserves this full-replacement decision while replacing its standard
`ConversationalAgent` target with generic installable Agent Program APIs and
repository-local conversational/Gao examples.

APXM Agent Program semantic migrations are full replacements. The current
unversioned Hook payload, broad decision union, implicit context behavior,
direct Gao loop, and any superseded artifact schema are implementation evidence
only. They are not supported compatibility contracts for the target release.

`apxm.frontend-graph.v1`, `apxm.air.v1`, `apxm.executable-artifact.v1`,
`apxm.runtime-evidence.v1`, the generic installable Python/TypeScript
`AgentProgram` surfaces, the two closed AIS operation families, and generic
`LoopIterationCompleted` evidence will replace every prior semantic path. All
first-party Python and TypeScript examples, artifacts, handlers, runtime
dispatch, generated declarations, tests, and product adapters move to the
target contract in one APXM Compatibility Set. Server, Studio, CLI, and other
cross-plane consumers use current-owner generated bindings under the
[current event/runtime ownership](0018-event-readiness-and-local-scheduling-are-agents-semantics.md).
Conversational and Gao examples use only packed generic APIs. Admission rejects
old or unknown semantic versions after cutover.

APXM will not ship:

- two Hook or Program Context engines;
- an adapter interpreting a pre-canonical Hook result as a canonical v1 result;
- a Gao direct-loop fallback or agent-id-specific runtime branch;
- dual artifact loading semantics selected by a feature flag;
- alias events, fields, package APIs, or environment variables for old callers;
- mixed old/new frontend, compiler, artifact, and runtime versions.

The pinned current source may be used to extract behavior vectors. One-time
fixture or artifact conversion tools may run in isolated migration workflows,
but they are not linked into the supported runtime and are deleted or archived
after the cutover gate. Historical run and audit evidence is transformed to a
target representation or retained as a digest-bound export; executable legacy
semantics are not retained to read it.

Implementation runs through the parallel
  [Agent Program full-replacement portfolio](../agents/agent-program-full-replacement-portfolio.md),
whose normative semantic lane is the composition and AIR replacement plan.

## Failure and rollback

An old or unknown contract/artifact fails explicitly before execution. It is
never guessed, silently upgraded, or executed with target semantics.

Before cutover, target candidates run in isolated test environments. Production
rollback restores one complete previously released APXM Compatibility Set and
matching product/data snapshot before the declared point of no return. It does
not enable an old semantic engine inside the target release. After target
artifacts or effects make restoration unsafe, recovery is forward-only.

## Considered options

### Preserve old and new semantic versions in one runtime

Rejected. It doubles compiler/runtime behavior, makes conformance conditional,
and leaves the deletion of the old model optional.

### Translate old artifacts during admission

Rejected as a supported runtime feature. Translation may exist only as an
isolated one-time migration tool with explicit input/output versions and
reconciliation evidence.

### Replace all first-party consumers and reject old semantics at cutover

Accepted. It keeps one frontend/compiler/artifact/runtime contract and makes
the breaking change explicit.

## Consequences

- Contract and public API changes are intentionally breaking.
- Python and TypeScript move together and remain semantically equivalent.
- Conversational and Gao repository examples are release-blocking generic-API
  conformance fixtures, not compatibility exceptions or package contracts.
- Distribution remains governed by coordinator ADR-0001. The compiler bridge
  participates in the same full replacement and one APXM v1 Compatibility
  Set.
- Release evidence must prove both target conformance and absence of old paths.
