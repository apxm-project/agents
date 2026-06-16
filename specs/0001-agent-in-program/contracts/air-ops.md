# Contract: AIR ops & attributes

New/changed AIR surface. Adding an op MUST update the op-count / tablegen parity
guard (research.md). All attributes travel inside the artifact (AIR-portable).

## `REGISTER_HOOK` (new op)
Registers one author hook into the per-artifact `HookRegistry`.
- Attributes: `HOOK_EVENT` (one of the lifecycle events), `HOOK_MATCH` (glob,
  default `*`), `HOOK_MODE` (`observe`|`gate`), `PYTHON_HOOK_HANDLER_ID`.
- Validation: unknown event → error; `gate` on a non-`pre_*` event → error;
  missing handler id → error. (Mirrors REGISTER_CAPABILITY shape.)
- Runtime: belief-style registration node; the real binding lives in the hooks
  sidecar, resolved at load into `ExecutionContext.hook_registry`.

## Turn loop (recv) — converted semantics
The in-graph loop reuses the AUTONOMOUS `recv` form but with two changes:
- **Park, not poll:** the recv node returns `OperationParked{wait_key=session_recv_key}`
  instead of HTTP long-polling; woken by the server turn-input endpoint.
- **Author turn body:** per wake, the loop runs the **author turn flow** (a
  sibling func), not the hardcoded Rust `run_agent_turn`. State carries across
  re-arms via spliced token connections.
- Attributes: `recv_once=false` (re-arm), `MAX_ITERATIONS` (event budget),
  reserved turn-input param name. AIR validation MUST accept a recv/turn-input
  entry + loop-back marker without tripping the acyclic-DAG check.

## Dataflow `system_prompt` (changed)
`ASK` MAY take a named input operand (e.g. `__system`) carrying the system prompt
as a dataflow value; when present, `resolve_system_prompt` uses it instead of the
static attr. Enables in-program context injection (FR-005/FR-009).

## `qmem` recent-window (changed)
`query_memory` gains a `recall_mode=recent` + `recent=N` attribute returning the
last-N session entries in temporal order (the transcript-as-memory window).

## Multi-flow module (changed emitter)
The frontend emits N `func.func` in one module; the `Agent.flow` dotted name MUST
survive name sanitization so `reconstruct_agents_from_artifact` registers each
flow under the right agent (DELEGATE resolves by name). Runtime/artifact already
support multi-DAG; the carrier fix is the work.

## Compaction primitives (composition, no new op)
Compaction is authored in-program from existing primitives. The shipped path is
a `post_turn` hook that uses `ctx.count_tokens` to decide when to compact,
`ctx.ask` to produce the author-defined summary, and `ctx.umem` to fold the
summary into the author-owned key pinned by `CompactionPolicy.summary_key`.
`count_tokens` MUST be registered in `register_standard_tools` for non-server
runtimes (parity). An optional future host-independent form may lower to an
AIR-only `count_tokens` → `guard`/`switch` → `ask` → `umem` graph; `branch()`
labels are NOT routed by the scheduler.
