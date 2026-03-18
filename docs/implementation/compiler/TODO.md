# Compiler TODOs

Gap analysis between documentation/specifications and actual implementation.
Generated 2026-03-17.

## P0: Critical

- [x] **Phase 1 ISA ops missing from C++ compiler** -- Resolved: `UpdateGoal`, `Guard`, `Claim`, `Pause`, `Resume` added to `AISOps.td` with proper memory effects, assembly formats, and verifiers. `OperationKind` entries 25-29 and `.Case<>()` arms added to `ArtifactEmitter.cpp`. Files: `crates/apxm-compiler/mlir/include/ais/Dialect/AIS/IR/AISOps.td`, `crates/apxm-compiler/mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp`

- [x] **Graph-to-MLIR lowerer missing 7 ops** -- Resolved: all 20 missing match arms added to `emit_node()` in `lower_mlir.rs`. The `unsupported` catch-all removed; match is now exhaustive. Covers Communicate, FlowCall, Exc, Print, Jump, Return, Agent, UpdateGoal, Guard, Claim, Pause, Resume, Nop, Identity, Yield, Delegate, Negotiate, SpawnAgent, RegisterCapability, Autonomous. Files: `crates/apxm-graph/src/lower_mlir.rs`

- [x] **O1/O2/O3 pipeline levels are identical** -- Resolved: pipeline.rs now has O1 (8 passes), O2 (11 passes + template-specialization, dead-context-elimination, schema-narrowing), O3 (O2 iterated 10x for convergence). Files: `crates/apxm-compiler/src/passes/pipeline.rs`

## P1: Important

- [x] **No `CondenseOps` pass** -- Resolved: C++ MLIR pass `CondenseOps.cpp` condenses consecutive QMEM/UMEM operations targeting the same memory space into batched single-node ops. Rust PassSpec added to `apxm-ais`, convenience method `condense_ops()` on PassManager, registered in O2/O3 pipelines after `fuse-ask-ops`. Files: `crates/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/CondenseOps.cpp`, `crates/apxm-ais/src/passes/mod.rs`, `crates/apxm-compiler/src/passes/manager.rs`, `crates/apxm-compiler/src/passes/pipeline.rs`, `crates/apxm-compiler/mlir/include/ais/Dialect/AIS/Transforms/Passes.td`, `crates/apxm-compiler/mlir/include/ais/Dialect/AIS/Transforms/Passes.h`

- [x] **No Rust-side artifact emitter (write path)** -- Resolved: `emit_wire_dags()` and `emit_wire_dag()` added to `codegen/artifact.rs` as pure-Rust inverse of `parse_wire_dags()`. Includes `BinaryWriter` struct, `write_node()`/`write_value()` helpers, and `AISOperationType::to_wire_index()` inverse mapping. 9 round-trip tests cover minimal, complex (all Value variants), multi-DAG, empty, unnamed, all dependency types, all 32 wire-indexed ops, convenience single-DAG, and deterministic output. Files: `crates/apxm-compiler/src/codegen/artifact.rs`, `crates/apxm-ais/src/operations/definitions.rs`

- [x] **`apxm decompile` command** -- Implemented in `f7a3c8c`: `Commands::Decompile` with `dag_to_graph()` reverse mapping from `ExecutionDag` to `ApxmGraph` JSON. Files: `crates/apxm-cli/src/main.rs`

- [x] **`apxm init` scaffolding command** -- Implemented in `f7a3c8c`: creates agents/, flows/, nodes/, prompts/, tools/ directories and apxm.toml. Files: `crates/apxm-cli/src/main.rs`

- [x] **Exploded workflow format (directory compilation)** -- Implemented in `f7a3c8c`: `load_graph_from_directory()` traverses flows/ and nodes/ subdirs, merges fragments via `ApxmGraph::merge`. Files: `crates/apxm-cli/src/main.rs`

- [x] **No iterative pass convergence** -- Resolved: O3 implements fixed-point loop with MAX_CONVERGENCE_ITERATIONS = 10.

## P2: Nice to Have

- [ ] **Future optimization passes not started** -- Docs list 5 future passes: prompt caching, memoization, quality-aware fusion, speculative execution, token compression. None have any implementation. Files: `docs/implementation/compiler/optimization-passes.md`

- [~] **No profile-guided optimization** -- Partially resolved: `ExecutionProfile` / `NodeProfile` data structures with JSON persistence and merge support added to `profile.rs`. `apply_to_graph()` annotates nodes with observed latency, error-rate, token-usage, and injects `retry_count` for high-error-rate nodes and token-budget warnings. `PipelineConfig` gains `profile_path` and `token_budget` fields; `Pipeline::compile_graph` loads and applies the profile before MLIR lowering. Remaining: runtime profile *collection*, JIT-style adaptation, latency-tier configuration. Files: `crates/apxm-compiler/src/passes/profile.rs`, `crates/apxm-core/src/types/compiler/optimization.rs`, `crates/apxm-compiler/src/api/pipeline.rs`

- [x] **`--no-cse-llm` flag** -- Implemented in `f7a3c8c`: CLI flag, `PipelineConfig.no_cse_llm`, `build_pipeline_with_config()` conditionally skips CSE pass. Files: `crates/apxm-cli/src/main.rs`, `crates/apxm-compiler/src/passes/pipeline.rs`, `crates/apxm-core/src/types/compiler/optimization.rs`

- [x] **Diagnostics are post-hoc, not per-pass** -- Resolved: `PassMetrics` and `PipelineDiagnostics` structs added to `passes/metrics.rs`. `build_pass_list()` factored out of `pipeline.rs` as the single source of truth for pipeline composition. `PassManager::run_with_metrics()` executes passes individually with per-pass timing and op-count tracking. `Pipeline::compile_graph_with_diagnostics()` and corresponding `Compiler` methods exposed through the driver. CLI `--emit-diagnostics` now emits `pass_metrics` array (per-pass name, duration_ms, ops_before, ops_after, ops_delta) and `pass_summary` (total_passes, initial_ops, final_ops, total_ops_eliminated, active_passes). Files: `crates/apxm-compiler/src/passes/metrics.rs`, `crates/apxm-compiler/src/passes/pipeline.rs`, `crates/apxm-compiler/src/passes/manager.rs`, `crates/apxm-compiler/src/api/pipeline.rs`, `crates/apxm-driver/src/compiler/mod.rs`, `crates/apxm-cli/src/main.rs`

- [x] **No AUTONOMOUS op** -- Resolved: enum variant (Autonomous), handler (autonomous.rs), dispatcher routing, OperationSpec entry all exist.

- [x] **Templates loadable from external files** -- Implemented in `f7a3c8c`: `load_user_templates()` reads `~/.apxm/templates.json`, merges with built-in templates in list/show commands. Files: `crates/apxm-cli/src/main.rs`

_Note: unified WorkflowNode for INV/FLOW_CALL/inner-plan is a cross-cutting concern tracked in master `docs/implementation/TODO.md`._
