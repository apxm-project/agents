# Runtime TODOs

Derived from gap analysis and verified against current source code.

---

## Completed

The following gaps have been resolved:

| Gap | Description | Key Commit / Files |
|-----|-------------|-------------------|
| 8   | AAM transitions wired into all handlers | `61d408f` -- handlers/*.rs, effects.rs |
| 8b  | Two divergent AAM types (spec vs runtime) | Canonical types in apxm-ais::aam |
| 4   | Flat goals (no parent-child hierarchy) | GoalTree in aam/mod.rs |
| 3   | No per-task AAM scoping | ScopePolicy: Inherit/Isolate/Snapshot/Filter |
| 9   | Goal priority not connected to scheduler | apply_goal_priorities() in state.rs |
| 7   | Tool discovery pipeline partially connected | UserToolCapability + ProcessSandbox |
| 1   | Fixed 4-level hierarchy, no recursive composition | WorkflowNode enum in apxm-core |
| 2   | DAG modification is expand-only (no condensation) | condense_subdag() in splicing.rs |
| 5   | Self-organization operations | SpawnAgent + RegisterCapability handlers |
| 10  | Latency tiers are compile-time constants | LatencyTierConfig + apply_latency_overrides() |
| 8c  | enter_operation/exit_operation call stack dead code | Wired into OperationDispatcher |
| 8d  | effects.rs aligned with handler behavior | Fence/Reflect corrected, no wildcard |

---

## Open

- [ ] **Gap 6 -- No exploded workflow format (runtime impact).**
  CLI side done (`apxm init`, `apxm decompile`, directory compilation), but the runtime scheduler and executor have no concept of directory-based workflow state. Workflows are monolithic JSON blobs. Runtime would need: per-sub-task directories for prompts, tools, and data; a loader that resolves `$ref` pointers from node attributes to files on disk; a watcher or cache-invalidation mechanism so hot-reloading works during development; integration with the `WorkflowNode::SubWorkflow` variant so nested workflows can reference sub-directories.
  _(CLI and compiler implementation tracked in `docs/implementation/compiler/TODO.md`)_
  Files: `crates/apxm-runtime/src/executor/engine.rs`, `crates/apxm-graph/src/`
