# Runtime TODOs

Derived from gap analysis and verified
against current source code as of 2026-03-17.

---

## P0: Critical (blocks hierarchical AAM vision)

- [ ] **Gap 8 — AAM transitions missing from most handlers.**
  Most AIS handlers produce no AAM state transition.
  Handlers that DO call `ctx.aam.*`: `llm.rs` (REASON only — beliefs + goals via `process_structured_output`), `plan.rs` (beliefs + goals), `umem.rs` (beliefs), `qmem.rs` (beliefs), `communicate.rs` (beliefs), `flow_call.rs` (beliefs), `update_goal.rs` (goals).
  Handlers that do NOT touch AAM: `llm.rs` (ASK, THINK — return plain `Value::String`, no AAM call), `reflect.rs`, `verify.rs`, `inv.rs`, `fence.rs`, `branch.rs`, `jump.rs`, `loop_start.rs`, `loop_end.rs`, `merge.rs`, `wait_all.rs`, `switch.rs`, `try_catch.rs`, `err.rs`, `exc.rs`, `print.rs`, `return_op.rs`, `const_str.rs`, `claim.rs`, `guard.rs`, `pause.rs`, `resume.rs`.
  Current: 7 of 32 ops produce transitions (22%). Needed: all ops that have declared effects in `effects.rs` must actually perform them (e.g., FENCE declares writes to B+STM+LTM+G but does nothing; REFLECT declares read of Episodic but never queries it; ASK declares reads from B but ignores the AAM; VERIFY declares read of Beliefs but ignores the AAM).
  Files: `crates/apxm-runtime/src/executor/handlers/*.rs`, `crates/apxm-runtime/src/aam/effects.rs`

- [ ] **Gap 8b — Two divergent AAM types (spec vs runtime).**
  `apxm-ais::aam::Goal` has `parent_id: Option<String>`, `GoalStatus::{Pending, Active, Completed, Failed, Cancelled}`, `priority: i32`.
  `apxm-runtime::aam::Goal` has no `parent_id`, `GoalStatus::{Active, Completed, Cancelled}` (missing Pending/Failed), `priority: u32`.
  These must be unified into a single canonical type or explicitly bridged.
  Current: two incompatible Goal types with different fields and status enums. Needed: single source of truth.
  Files: `crates/apxm-ais/src/aam.rs`, `crates/apxm-runtime/src/aam/mod.rs`

- [ ] **Gap 4 — Flat goals (no parent-child hierarchy).**
  Runtime `Goal` has no `parent_id` field. `AamState.goals` is a `PriorityQueue<GoalId, u32>` — flat, no tree structure. PLAN produces `Vec<PlanStep>` with string-based `dependencies`, not a recursive goal tree. No completion propagation from child to parent goals.
  Current: flat priority queue. Needed: `GoalTree` with parent-child links, `CompletionPolicy` (AllChildren, AnyChild, Manual, Condition), and automatic propagation.
  Files: `crates/apxm-runtime/src/aam/mod.rs`, `crates/apxm-core/src/plan.rs`

- [ ] **Gap 3 — No per-task AAM scoping.**
  `ExecutionContext::child()` clones the same `Arc<RwLock<AamState>>` — parent and child share identical flat state. A UMEM in flow A is immediately visible to flow B. No `ScopeId`, `ScopeSpec`, or belief namespacing exists anywhere in the runtime.
  Current: global shared AAM. Needed: `ScopeSpec { beliefs: ScopePolicy, capabilities: ScopePolicy, goal: Option<GoalSpec> }` with Inherit/Isolate/Filter policies, and scoped child AAM creation.
  Files: `crates/apxm-runtime/src/executor/context.rs` (line 174-193), `crates/apxm-runtime/src/aam/mod.rs`

---

## P1: Important (needed for production-quality hierarchical execution)

- [ ] **Gap 9 — Goal priority not connected to scheduler priority.**
  Scheduler uses `node.metadata.priority` (set at compile time in `Priority::from_u8`). AAM `Goal.priority` is a separate system never consulted by the scheduler. No mechanism for PLAN to link a spawned sub-DAG back to a goal.
  Current: two disconnected priority systems. Needed: goal-aware scheduling that projects `Goal.priority` onto node scheduling priority.
  Files: `crates/apxm-runtime/src/scheduler/state.rs` (line 121), `crates/apxm-runtime/src/scheduler/queue.rs`, `crates/apxm-runtime/src/aam/mod.rs`

- [ ] **Gap 7 — Tool discovery pipeline is disconnected.**
  `apxm tools register` writes to `~/.apxm/tools.json`. Driver startup calls `register_standard_tools()` which loads only built-in tools (bash, search_web, etc. from `apxm-tools`). The runtime never reads `tools.json`. `AamCheckpoint` excludes capabilities (only serializes beliefs + goals), so capabilities are lost on resume.
  Current: CLI tool registration and runtime capability system are separate code paths. Needed: driver reads `tools.json` at startup, capabilities included in checkpoint/restore.
  Files: `crates/apxm-driver/src/runtime/capabilities.rs`, `crates/apxm-tools/src/lib.rs`, `crates/apxm-runtime/src/aam/mod.rs` (line 274-279, 391-395)

- [ ] **Gap 1 — Fixed 4-level hierarchy, no recursive composition.**
  The hierarchy is Agent -> Flow -> Task -> Node, exactly four levels. `PlanStep` is a flat list with string dependencies. `ApxmGraph::merge()` flattens into a single DAG. Flows cannot structurally contain sub-flows.
  Current: flat 4-level hierarchy. Needed: recursive `WorkflowNode` that can contain sub-workflows (unifying INV, FLOW_CALL, and PLAN inner-plan behind one abstraction).
  Files: `crates/apxm-core/src/plan.rs`, `crates/apxm-core/src/types/execution/agent.rs`

- [ ] **Gap 2 — DAG modification is expand-only (no condensation).**
  `DagSplicer` supports expanding a node into a sub-DAG. No `replace_subdag()` or `condense_subdag()` exists. `SchedulerState` uses `Arc<Node>` (immutable) with no node removal API.
  Current: append-only DAG modification. Needed: bidirectional graph modification (condense a sub-DAG into a single capability node).
  _(Compiler-side counterpart: `CondenseOps` pass tracked in `docs/implementation/compiler/TODO.md`)_
  Files: `crates/apxm-runtime/src/scheduler/splicing.rs`, `crates/apxm-runtime/src/scheduler/state.rs`

- [ ] **Gap 5 — No self-organization control plane operations.**
  No SPAWN_AGENT or REGISTER_CAPABILITY AIS operations exist. `create_task` and `compile_task_dag` exist only as server HTTP endpoints (`apxm-server/src/main.rs` lines 603/1585), not as registered runtime capabilities that agents can invoke from within a workflow.
  Current: no AIS ops for agent self-modification. Needed: AIS operations that let an agent spawn sub-agents and register capabilities at runtime.
  Files: `crates/apxm-runtime/src/executor/dispatcher.rs`, `crates/apxm-server/src/main.rs`

---

## P2: Nice to Have (future-proofing, polish)

- [ ] **Gap 10 — Latency tiers are compile-time constants.**
  `estimated_latency` is set by the compiler and baked into the artifact. Not configurable per model backend at runtime. No model capability probing to determine if a multi-step workflow can be condensed into a single call.
  Current: compile-time constants. Needed: runtime-configurable latency tiers per model backend, profile-guided adaptation.
  Files: `crates/apxm-compiler/src/codegen/artifact.rs` (line 148), `crates/apxm-core/src/types/execution/node.rs` (line 29)

- [ ] **Gap 6 — No exploded workflow format (runtime impact).**
  Workflows are monolithic JSON blobs. The scheduler and executor have no concept of directory-based workflow state. Runtime would need per-sub-task directories for prompts, tools, and data.
  _(CLI and compiler implementation tracked in `docs/implementation/compiler/TODO.md`: `apxm init`, `apxm decompile`, directory compilation)_
  Files: `crates/apxm-runtime/src/executor/engine.rs`, `crates/apxm-graph/src/`

- [ ] **Gap 8c — `enter_operation`/`exit_operation` call stack is dead code.**
  `Aam::enter_operation()` and `Aam::exit_operation()` maintain a `call_stack: Vec<CallFrame>`, but no handler ever calls them. The exception handler lookup (`current_exception_handler`) depends on this stack but is also unused.
  Current: dead code. Needed: either wire into the dispatcher (call enter/exit around each handler dispatch) or remove.
  Files: `crates/apxm-runtime/src/aam/mod.rs` (lines 101-127)

- [ ] **Gap 8d — `effects.rs` declarations don't match actual handler behavior.**
  `operation_effects(Fence)` declares writes to Beliefs+STM+LTM+Goals, but `fence::execute()` is a pure passthrough (no AAM interaction). `operation_effects(Reflect)` declares read of Episodic, but `reflect::execute()` never queries episodic memory via the AAM. These mismatches can cause the compiler's reordering analysis to be overly conservative.
  Current: declared effects diverge from actual effects. Needed: align declarations with actual behavior (either update handlers to perform declared effects, or correct the declarations).
  Files: `crates/apxm-runtime/src/aam/effects.rs`, `crates/apxm-runtime/src/executor/handlers/fence.rs`, `crates/apxm-runtime/src/executor/handlers/reflect.rs`
