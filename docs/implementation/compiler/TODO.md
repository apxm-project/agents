# Compiler TODOs

Gap analysis between documentation/specifications and actual implementation.
Generated 2026-03-17.

## P0: Critical

- [x] **Phase 1 ISA ops missing from C++ compiler** -- Resolved: `UpdateGoal`, `Guard`, `Claim`, `Pause`, `Resume` added to `AISOps.td` with proper memory effects, assembly formats, and verifiers. `OperationKind` entries 25-29 and `.Case<>()` arms added to `ArtifactEmitter.cpp`. Files: `crates/apxm-compiler/mlir/include/ais/Dialect/AIS/IR/AISOps.td`, `crates/apxm-compiler/mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp`

- [x] **Graph-to-MLIR lowerer missing 7 ops** -- Resolved: all 20 missing match arms added to `emit_node()` in `lower_mlir.rs`. The `unsupported` catch-all removed; match is now exhaustive. Covers Communicate, FlowCall, Exc, Print, Jump, Return, Agent, UpdateGoal, Guard, Claim, Pause, Resume, Nop, Identity, Yield, Delegate, Negotiate, SpawnAgent, RegisterCapability, Autonomous. Files: `crates/apxm-graph/src/lower_mlir.rs`

- [x] **O1/O2/O3 pipeline levels are identical** -- Resolved: pipeline.rs now has O1 (8 passes), O2 (11 passes + template-specialization, dead-context-elimination, schema-narrowing), O3 (O2 iterated 10x for convergence). Files: `crates/apxm-compiler/src/passes/pipeline.rs`

## P1: Important

- [ ] **No `CondenseOps` pass** -- Gap analysis (Gap 2) and optimization-passes doc both describe a pass that replaces sub-DAGs with single capability calls. No implementation exists in C++ or Rust. The FuseAskOps pass is the only domain-specific optimization. Files: `crates/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/`

- [ ] **No Rust-side artifact emitter (write path)** -- `codegen/artifact.rs` only contains a binary *parser* (`parse_wire_dags`). All artifact *emission* goes through C++ FFI (`apxm_codegen_emit_artifact`). A pure-Rust emitter would enable artifact generation without the MLIR/LLVM toolchain for simple graphs. Files: `crates/apxm-compiler/src/codegen/artifact.rs`

- [x] **`apxm decompile` command** -- Implemented in `f7a3c8c`: `Commands::Decompile` with `dag_to_graph()` reverse mapping from `ExecutionDag` to `ApxmGraph` JSON. Files: `crates/apxm-cli/src/main.rs`

- [x] **`apxm init` scaffolding command** -- Implemented in `f7a3c8c`: creates agents/, flows/, nodes/, prompts/, tools/ directories and apxm.toml. Files: `crates/apxm-cli/src/main.rs`

- [x] **Exploded workflow format (directory compilation)** -- Implemented in `f7a3c8c`: `load_graph_from_directory()` traverses flows/ and nodes/ subdirs, merges fragments via `ApxmGraph::merge`. Files: `crates/apxm-cli/src/main.rs`

- [x] **No iterative pass convergence** -- Resolved: O3 implements fixed-point loop with MAX_CONVERGENCE_ITERATIONS = 10.

## P2: Nice to Have

- [ ] **Future optimization passes not started** -- Docs list 5 future passes: prompt caching, memoization, quality-aware fusion, speculative execution, token compression. None have any implementation. Files: `docs/implementation/compiler/optimization-passes.md`

- [ ] **No profile-guided optimization** -- Gap analysis (Gap 10) describes JIT-style profile-guided adaptation and runtime-configurable latency tiers. No profiling infrastructure exists in the compiler. Files: `crates/apxm-compiler/src/passes/`

- [x] **`--no-cse-llm` flag** -- Implemented in `f7a3c8c`: CLI flag, `PipelineConfig.no_cse_llm`, `build_pipeline_with_config()` conditionally skips CSE pass. Files: `crates/apxm-cli/src/main.rs`, `crates/apxm-compiler/src/passes/pipeline.rs`, `crates/apxm-core/src/types/compiler/optimization.rs`

- [ ] **Diagnostics are post-hoc, not per-pass** -- `--emit-diagnostics` generates a JSON with node/edge counts after compilation, but does not report per-pass metrics (which pass fired, how many ops it fused/eliminated, time per pass). Files: `crates/apxm-cli/src/main.rs` (line ~604)

- [x] **No AUTONOMOUS op** -- Resolved: enum variant (Autonomous), handler (autonomous.rs), dispatcher routing, OperationSpec entry all exist.

- [x] **Templates loadable from external files** -- Implemented in `f7a3c8c`: `load_user_templates()` reads `~/.apxm/templates.json`, merges with built-in templates in list/show commands. Files: `crates/apxm-cli/src/main.rs`

_Note: unified WorkflowNode for INV/FLOW_CALL/inner-plan is a cross-cutting concern tracked in master `docs/implementation/TODO.md`._
