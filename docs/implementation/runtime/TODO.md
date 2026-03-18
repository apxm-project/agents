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
