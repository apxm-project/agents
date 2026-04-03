# Compiler TODOs

Gap analysis between documentation/specifications and actual implementation.
Generated 2026-03-17. Updated 2026-04-03.

## Completed

All P0 (Critical) and P1 (Important) items have been resolved:

- Phase 1 ISA ops added to C++ compiler (UpdateGoal, Guard, Claim, Pause, Resume)
- Graph-to-MLIR lowerer: all match arms added, exhaustive coverage
- O1/O2/O3 pipeline levels differentiated (8, 12, and convergence-iterated passes)
- CondenseOps pass (C++ MLIR + Rust PassSpec, registered in O2/O3)
- Rust-side artifact emitter (write path) with round-trip tests
- `apxm decompile` command
- `apxm init` scaffolding command
- Exploded workflow format (directory compilation)
- Iterative pass convergence (O3, MAX_CONVERGENCE_ITERATIONS = 10)
- `--no-cse-llm` flag
- Per-pass diagnostics (`--emit-diagnostics`)
- AUTONOMOUS op (enum variant, handler, dispatcher routing, OperationSpec)
- Templates loadable from external files
- Prompt caching (graph-level pass at O2+, runs before MLIR lowering)
- Memoization hints (graph-level pass at O2+, runs before MLIR lowering)

## P2: Nice to Have

- [ ] **Future optimization passes** -- Three planned passes have no implementation:
  - **Quality-aware fusion**: merge adjacent ASK ops only when quality metrics (coherence, accuracy) are preserved; needs a quality estimator or A/B test harness.
  - **Speculative execution**: pre-execute likely branches in BRANCH/SWITCH nodes before the condition resolves; discard wrong-path results (requires rollback support via AAM snapshots).
  - **Token compression**: reduce token count in prompts by summarizing context, eliding redundant instructions, or using shorthand encodings (requires a compression pass that preserves semantic fidelity).

- [~] **No profile-guided optimization** -- Partially resolved: compiler-side infra exists (`ExecutionProfile` / `NodeProfile` data structures with JSON persistence and merge support in `profile.rs`; `apply_to_graph()` annotates nodes with observed latency, error-rate, token-usage, and injects `retry_count` for high-error-rate nodes and token-budget warnings; `PipelineConfig` gains `profile_path` and `token_budget` fields; `Pipeline::compile_graph` loads and applies the profile before MLIR lowering). Remaining:
  - **Runtime profile collection**: no code records actual latency/error-rate/token-usage during execution. Would need instrumentation in the dispatcher or event emitter to populate `NodeProfile` entries and persist them via `ExecutionProfile::save_to_file()`.
  - **JIT-style adaptation**: no mechanism to re-compile or re-optimize a running workflow based on observed profile data (e.g., switching backends for slow nodes, adjusting parallelism).
  Files: `crates/apxm-compiler/src/passes/profile.rs`, `crates/apxm-core/src/types/compiler/optimization.rs`, `crates/apxm-compiler/src/api/pipeline.rs`

_Note: unified WorkflowNode for INV/FLOW_CALL/inner-plan is a cross-cutting concern tracked in master `docs/implementation/TODO.md`._
