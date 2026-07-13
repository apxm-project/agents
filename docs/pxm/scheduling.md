# Scheduling in A-PXM

APXM scheduling starts from a compiled dataflow graph. A node can run when its
typed inputs are available, its capability grants are valid, and its declared
effects do not conflict with other admitted work.

## Scheduling Rules

- **Data dependencies are explicit.** Edges carry values or futures; downstream
  work waits on the values it actually needs.
- **Effects are visible.** Memory writes, tool calls, and messages are AIS
  operations with declared effects, so the scheduler can separate independent
  work from ordered work.
- **Latency budgets are part of the program.** `ASK`, `THINK`, and `REASON`
  express different expected costs. The scheduler can overlap longer reasoning
  work with short independent operations.
- **Authority gates precede execution.** Capability grants and permission checks
  are evaluated before mutating operations run.
- **Host topology is not runtime topology.** Organization policy can admit,
  deny, or route a request, but the runtime sees concrete targets and typed
  operations.

## Runtime Shape

The runtime maintains a ready set over graph nodes instead of advancing through
global phases. Completion of a typed dependency can release its consumers
immediately; unrelated work does not wait for a phase barrier. Among runnable
nodes, the workflow runner changes priority only when a current, versioned
compiler summary proves the affected operations reorderable; otherwise it keeps
deterministic declaration order. Dependency, effect, approval, grant, and
replay ordering remain unchanged.

Async operations remain represented by futures. Work waiting on permission,
host input, or an external event parks without being reported as complete.
Run/session evidence records the actual scheduling and usage outcome. Backend
selection remains a separate concern driven by registered routes and their
declared capabilities; the scheduler does not infer a provider, model, endpoint,
or deployment profile.

## Prompt and Context Planning

LLM nodes use one typed plan for prompt roles, context assembly, tool-result
budgets, conversation compaction, model-call reservation, and observed usage
reconciliation. Mandatory system/user/tool/control inputs and dependency-only
values cannot be silently reordered, merged, or dropped as interchangeable
strings.

Context planning is enabled only when the caller supplies a planning policy.
The runtime does not insert a tokenizer, context profile, budget, or scope when
that policy is absent. A plan that cannot satisfy its typed role and budget
constraints fails through the runtime error surface instead of falling back to
an implicit profile.

## Checkpoints, Memoization, and Replay

Checkpoint placement follows typed effect and replay evidence. A precompiled
workflow artifact requires a current versioned summary. If that summary requires
a checkpoint before a node, workflow admission fails because the workflow
runner does not invent or inject compiler-owned checkpoint barriers. An explicit
`CHECKPOINT` node persists only a scheduler frontier that the scheduler marks
replayable and quiescent outside the checkpoint itself. A checkpoint is not an
assertion that a prior side effect may be skipped: partial replay can reuse an
effect only when its owning authority supplies verified durable evidence for
that exact execution, graph, node, invocation, grant, approval, request digest,
and committed outcome.

Memoization is admitted only when the compiled operation carries the
compiler-stamped eligibility that the operation handler consumes. The workflow
runner does not infer memoization eligibility or deduplicate whole workflow
steps. It also does not combine ready steps into a request batch: LLM
concurrency bounds individual requests. Cache keys include the identities needed
to prevent semantic or authority aliasing. Native, script-local, or Link v1
effects do not become replay-authoritative merely because an output was cached
or checkpointed.

## Evidence and Tests

The implementation lives under `crates/runtime/engine/src/scheduler/`, with
typed context planning under `crates/runtime/engine/src/context_stack/` and
effect replay validation in the scheduler/executor replay paths. Compiler
priority and legality inputs are described in the
[compiler pipeline](../compiler/pipeline.md). Repository tests cover ready-set
release, critical-path ordering, producer-finished-before-dependent-ready trace
ordering, checkpoint admission rejection, memoization rejection, and
accepted/rejected replay boundaries.

For implementation details, see
[compiler pipeline](../compiler/pipeline.md) and the runtime crate README.
