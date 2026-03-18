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

- [x] **Gap 1 — Fixed 4-level hierarchy, no recursive composition.**
  Resolved: `WorkflowNode` enum added to `apxm-core::types::execution` with four variants: `Operation(Node)` (leaf), `FlowCall` (named flow reference), `Invocation` (external tool/capability), and `SubWorkflow` (recursive inline sub-workflow containing `Vec<WorkflowNode>` + `Vec<Edge>`). Methods: `flatten()` recursively collects leaf `Node`s, `depth()` computes max nesting depth, `is_leaf()`, `is_sub_workflow()`, `kind()`, `node_count()`. Serde roundtrip tested. 16 unit tests including 50-level deep nesting stress test. Existing handlers untouched; they can adopt `WorkflowNode` incrementally.
  Files: `crates/apxm-core/src/types/execution/workflow.rs`, `crates/apxm-core/src/types/execution/mod.rs`, `crates/apxm-core/src/types/mod.rs`

- [x] **Gap 2 — DAG modification is expand-only (no condensation).**
  Resolved: `condense_subdag()` added to `DagSplicer` trait with full `SchedulerState` implementation. `ReadySet::remove_pending()` added for node removal. 8 unit tests covering condensation, edge rewiring, and scheduler state consistency.
  _(Compiler-side counterpart: `CondenseOps` pass tracked in `docs/implementation/compiler/TODO.md`)_
  Files: `crates/apxm-runtime/src/scheduler/splicing.rs`, `crates/apxm-runtime/src/scheduler/state.rs`

- [x] **Gap 5 — Self-organization operations implemented.** -- Resolved: AISOperationType::SpawnAgent and RegisterCapability exist as enum variants with full handlers and dispatcher routing.

---

## P2: Nice to Have (future-proofing, polish)

- [x] **Gap 10 — Latency tiers are compile-time constants.**
  Resolved: `LatencyTierConfig` added to `apxm-core::types::execution::node` with per-backend tier map and default fallback. Wired into `SchedulerConfig.latency_tiers` (with builder method `with_latency_tiers()`). `DataflowScheduler::apply_latency_overrides()` mutates the DAG before cost-budget enforcement and state construction, overriding `estimated_latency` for nodes whose `"backend"` attribute matches a configured tier. Backward compatible: empty config (default) is a no-op. 14 unit tests (6 for `LatencyTierConfig` in apxm-core, 8 for `apply_latency_overrides` in apxm-runtime).
  Remaining sub-gap: model capability probing / profile-guided adaptation (future work).
  Files: `crates/apxm-core/src/types/execution/node.rs`, `crates/apxm-runtime/src/scheduler/config.rs`, `crates/apxm-runtime/src/scheduler/dataflow.rs`

- [ ] **Gap 6 — No exploded workflow format (runtime impact).**
  CLI side done (`apxm init`, `apxm decompile`, directory compilation), but the runtime scheduler and executor have no concept of directory-based workflow state. Workflows are monolithic JSON blobs. Runtime would need: per-sub-task directories for prompts, tools, and data; a loader that resolves `$ref` pointers from node attributes to files on disk; a watcher or cache-invalidation mechanism so hot-reloading works during development; integration with the `WorkflowNode::SubWorkflow` variant so nested workflows can reference sub-directories.
  _(CLI and compiler implementation tracked in `docs/implementation/compiler/TODO.md`: `apxm init`, `apxm decompile`, directory compilation)_
  Files: `crates/apxm-runtime/src/executor/engine.rs`, `crates/apxm-graph/src/`

- [x] **Gap 8c — `enter_operation`/`exit_operation` call stack is dead code.**
  Resolved: `OperationDispatcher::dispatch_inner()` now calls `ctx.aam.enter_operation(node.id)` before handler dispatch and `ctx.aam.exit_operation()` after (regardless of success/failure). The AAM call stack now reflects actual execution flow, enabling `current_exception_handler()` to resolve TryCatch scopes. Also added manual `Debug` impl for `Aam` (required by concurrent workspace changes).
  Files: `crates/apxm-runtime/src/executor/dispatcher.rs`, `crates/apxm-runtime/src/aam/mod.rs`

- [x] **Gap 8d — `effects.rs` aligned with handler behavior.**
  Fixed in `61d408f`: Fence effect corrected to empty (pure barrier), Reflect corrected to read+write Episodic, all ops have explicit entries (no wildcard catch-all). Handlers now match their declared effects.
  Files: `crates/apxm-runtime/src/aam/effects.rs`, `crates/apxm-runtime/src/executor/handlers/fence.rs`, `crates/apxm-runtime/src/executor/handlers/reflect.rs`
