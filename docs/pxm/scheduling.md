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

The runtime uses a work-ready queue over graph nodes, tracks futures for async
operations, records run/session evidence, and parks work that is waiting on
external events or permission. Backend selection is a separate concern handled
by registered backend routes.

For implementation details, see
[compiler pipeline](../compiler/pipeline.md) and the runtime crate README.
