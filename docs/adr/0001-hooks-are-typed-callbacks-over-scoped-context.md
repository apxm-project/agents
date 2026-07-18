---
status: superseded
date: 2026-07-15
superseded_by: ADR-0010
---

# Hooks are typed callbacks over scoped context

> Superseded by [ADR-0010](0010-agent-program-source-owns-context-hooks-and-conversational-loops.md).
> This file preserves the rejected historical rationale only; it is not a
> target contract or implementation source.

APXM will use one program-authored Hook mechanism across agent, Turn, Node,
model, and Capability lifecycle events. A Hook is an asynchronous callback with
the semantic shape `HookCallback<E> = (HookContext<E>) -> HookResult<E>`.
Frontends may provide event-specific registration sugar, but Python,
TypeScript, AIR, artifacts, and the runtime must preserve one versioned
callback contract.

The callback receives a scoped `HookContext`, not the frontend's
`ConversationalAgent` instance and not the runtime's internal
`ExecutionContext`. The Hook Context contains a read-only Agent Reference,
Turn and target metadata, the current immutable Program Context Snapshot,
admitted Skill and Capability views, budgets, cancellation, trace correlation,
and explicitly supported helper operations. It excludes credentials, raw
grants, scheduler internals, mutable graph state, and ambient authority.

Program Context behaves like a local variable in an Agent Program but is
immutable and versioned. Hooks and program operations accumulate it through
typed Context Deltas. Parallel branches observe snapshots and reconcile changes
only through explicit deterministic Context Merges. Model Context and
Capability Context are least-privilege projections of Program Context;
Execution Context remains runtime-private; Memory changes only through explicit
persistence intents.

Hook events remain specialized because each lifecycle point permits different
effects. Generic Node Hooks may observe and a pre-node Hook may gate, but they
may not rewrite arbitrary node inputs or outputs. Model and Capability Hooks
may return only their event-specific typed transformations. A pure observer
cannot mutate semantics. A controlling Hook fails closed by default, while an
observer failure is recorded and execution may continue under explicit policy.

Hook-originated model and Capability calls remain budgeted, traced, cancelled,
and authorized through the normal APXM chokepoints. Version 1 permits only
read-only Capability calls from Hooks. Nested model calls do not re-enter the
active lifecycle callback chain, preventing recursion such as
`pre_ask -> ask -> pre_ask`.

This decision is normative design, not a claim that the complete contract is
already implemented. The canonical contract and implementation gaps are in
[`../agents/hook-and-context-contract.md`](../agents/hook-and-context-contract.md).

## Considered options

### Pass the whole agent object

Rejected. The frontend object does not exist after compilation, cannot retain
object identity across Python, TypeScript, AIR, and isolated handlers, and
would expose mutable state or runtime authority that cannot be serialized,
audited, replayed, or safely attenuated.

### Use one untyped callback payload and arbitrary return value

Rejected. It is superficially simple but makes event legality, context
transformations, compatibility, and fail-closed behavior runtime conventions
instead of compiler-checkable contracts.

### Build separate Hook systems for Turns, Nodes, models, and Capabilities

Rejected. Separate systems would duplicate registration, packaging, dispatch,
ordering, tracing, and failure behavior and would drift between frontends.

### Treat all Hooks as middleware

Rejected. Runtime middleware and Execution Observers remain useful internal
composition mechanisms, but an author-visible lifecycle callback requires a
portable artifact contract and event-specific input and output types.

## Consequences

- `agents` owns the semantic contract, frontend lowering, artifact binding,
  runtime application, and conformance tests.
- Python and TypeScript must expose equivalent typed callback ergonomics and
  lower to the same compiler-owned graph representation.
- The existing unversioned Hook payload and decision surface must be migrated
  rather than treated as the canonical contract.
- Current scheduler `ExecutionHook` observers remain internal and must be named
  Execution Observers in user-facing documentation.
- Product surfaces may author and display these constructs but must consume the APXM
  contract rather than create product-specific Hook or context semantics.
