# Runtime TODOs

Derived from gap analysis and verified
against current source code as of 2026-03-17.

---

## P0: Critical (blocks hierarchical AAM vision)

- [x] **Gap 8 — AAM transitions wired into all handlers.**
  Implemented in `61d408f`: all handlers now produce AAM state transitions via `ctx.aam.set_belief()`. Branch, claim, err, exc, guard, inv, llm (ASK/THINK), loop_end, loop_start, pause, print, reflect, resume, switch, verify all record transitions. `effects.rs` rewritten with explicit per-op entries (no catch-all wildcard), Fence corrected to pure barrier, Reflect corrected to write Episodic.
  Files: `crates/apxm-runtime/src/executor/handlers/*.rs`, `crates/apxm-runtime/src/aam/effects.rs`

- [x] **Gap 8b — Two divergent AAM types (spec vs runtime).** Resolved: canonical Goal/GoalId/GoalStatus defined in apxm-ais::aam, re-exported via apxm-core::types::goal, imported by apxm-runtime.

- [x] **Gap 4 — Flat goals (no parent-child hierarchy).**
  Resolved: `GoalTree` struct added to `AamState` with parent-child index (`HashMap<GoalId, Vec<GoalId>>`). `CompletionPolicy` enum (AllChildren, AnyChild, Manual) with `propagate_completion()`. `Aam::add_child_goal()` and `Aam::children_of()` methods. GoalTree included in checkpoint snapshot/restore. 3 unit tests.
  Files: `crates/apxm-runtime/src/aam/mod.rs`

- [x] **Gap 3 — No per-task AAM scoping.**
  Resolved: `ScopePolicy` enum extended with `Snapshot` variant (Inherit/Isolate/Snapshot/Filter). `Aam::child_scope(&ScopeSpec)` creates child AAMs with correct semantics: Inherit shares the parent `Arc` (bidirectional writes), Isolate starts empty, Snapshot copies then diverges, Filter copies a subset. `ExecutionContext::child()` now delegates through `child_with_scope(ScopeSpec::default())` which preserves backward-compatible Inherit semantics. 5 unit tests (inherit shares state, isolate starts empty, snapshot copies then diverges, filter inherits subset, mixed policies).
  Files: `crates/apxm-runtime/src/aam/mod.rs`, `crates/apxm-runtime/src/executor/context.rs`

---

## P1: Important (needed for production-quality hierarchical execution)

- [x] **Gap 9 — Goal priority not connected to scheduler priority.**
  Resolved: `Aam::active_goal_priority()` and `Aam::goal_priority_by_description()` bridge the AAM goal system to the scheduler. `SchedulerState::apply_goal_priorities(&Aam)` projects goal priorities onto node scheduler priorities using `max(compile_time, goal_priority)`. Called from `DataflowScheduler::execute()` after state construction. Nodes with `goal_id` attribute get the matching goal's priority; default-priority nodes without `goal_id` get the top active goal's priority. 9 tests cover boost, no-downgrade, inactive-goal filtering, and empty-AAM cases.

- [x] **Gap 7 — Tool discovery pipeline partially connected.**
  Resolved: `UserToolCapability::execute()` now dispatches via `ProcessSandbox`. Tools in `~/.apxm/tools.json` include `command`, `args`, `timeout_ms` fields. Subprocess receives JSON args on stdin, stdout parsed as JSON Value. Timeout and error handling included.
  Files: `crates/apxm-driver/src/runtime/capabilities.rs`

- [ ] **Gap 1 — Fixed 4-level hierarchy, no recursive composition.**
  The hierarchy is Agent -> Flow -> Task -> Node, exactly four levels. `PlanStep` is a flat list with string dependencies. `ApxmGraph::merge()` flattens into a single DAG. Flows cannot structurally contain sub-flows.
  Current: flat 4-level hierarchy. Needed: recursive `WorkflowNode` that can contain sub-workflows (unifying INV, FLOW_CALL, and PLAN inner-plan behind one abstraction).
  Files: `crates/apxm-core/src/plan.rs`, `crates/apxm-core/src/types/execution/agent.rs`

- [ ] **Gap 2 — DAG modification is expand-only (no condensation).**
  `DagSplicer` supports expanding a node into a sub-DAG. No `replace_subdag()` or `condense_subdag()` exists. `SchedulerState` uses `Arc<Node>` (immutable) with no node removal API.
  Current: append-only DAG modification. Needed: bidirectional graph modification (condense a sub-DAG into a single capability node).
  _(Compiler-side counterpart: `CondenseOps` pass tracked in `docs/implementation/compiler/TODO.md`)_
  Files: `crates/apxm-runtime/src/scheduler/splicing.rs`, `crates/apxm-runtime/src/scheduler/state.rs`

- [x] **Gap 5 — Self-organization operations implemented.** -- Resolved: AISOperationType::SpawnAgent and RegisterCapability exist as enum variants with full handlers and dispatcher routing.

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

- [x] **Gap 8d — `effects.rs` aligned with handler behavior.**
  Fixed in `61d408f`: Fence effect corrected to empty (pure barrier), Reflect corrected to read+write Episodic, all ops have explicit entries (no wildcard catch-all). Handlers now match their declared effects.
  Files: `crates/apxm-runtime/src/aam/effects.rs`, `crates/apxm-runtime/src/executor/handlers/fence.rs`, `crates/apxm-runtime/src/executor/handlers/reflect.rs`
