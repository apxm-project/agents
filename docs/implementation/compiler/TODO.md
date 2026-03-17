# Compiler TODOs

Gap analysis between documentation/specifications and actual implementation.
Generated 2026-03-17.

## P0: Critical

- [ ] **Phase 1 ISA ops missing from C++ compiler** -- `UpdateGoal`, `Guard`, `Claim`, `Pause`, `Resume` exist in Rust (`definitions.rs`) with runtime handlers but have NO C++ MLIR ops (`AISOps.h`/`AISOps.cpp`), no tablegen definitions, no `OperationKind` entries in `ArtifactEmitter.cpp`, and no wire indices in `from_wire_index()` (map stops at index 24). These ops cannot compile through the MLIR pipeline. Files: `crates/apxm-compiler/mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp`, `crates/apxm-ais/src/operations/definitions.rs`

- [ ] **Graph-to-MLIR lowerer missing 7 ops** -- `lower_mlir.rs` handles 20 of 27 non-internal ops. Missing: `Communicate`, `FlowCall`, `Exc`, `Print`, `Jump`, `Return`, `Agent`. These hit the `unsupported` error arm at line 681. The C++ emitter handles all of them, so only the Rust `ApxmGraph::to_mlir()` path is broken. Files: `crates/apxm-graph/src/lower_mlir.rs`

- [ ] **O1/O2/O3 pipeline levels are identical** -- `build_pipeline()` treats O1, O2, and O3 as the same pass sequence. Docs describe `-O3` as "aggressive optimization." There is no tiered optimization behavior. Files: `crates/apxm-compiler/src/passes/pipeline.rs`

## P1: Important

- [ ] **No `CondenseOps` pass** -- Gap analysis (Gap 2) and optimization-passes doc both describe a pass that replaces sub-DAGs with single capability calls. No implementation exists in C++ or Rust. The FuseAskOps pass is the only domain-specific optimization. Files: `crates/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/`

- [ ] **No Rust-side artifact emitter (write path)** -- `codegen/artifact.rs` only contains a binary *parser* (`parse_wire_dags`). All artifact *emission* goes through C++ FFI (`apxm_codegen_emit_artifact`). A pure-Rust emitter would enable artifact generation without the MLIR/LLVM toolchain for simple graphs. Files: `crates/apxm-compiler/src/codegen/artifact.rs`

- [ ] **No `apxm decompile` command** -- Gap analysis (Gap 6) calls for `.apxmobj` to `ApxmGraph` JSON round-tripping. The artifact parser exists in Rust, but there is no CLI command and no `ExecutionDag -> ApxmGraph` reverse mapping. Files: `crates/apxm-cli/src/main.rs`

- [ ] **No `apxm init` scaffolding command** -- Gap analysis (Gap 6) describes project scaffolding that creates the exploded workflow directory structure. Not implemented. Files: `crates/apxm-cli/src/main.rs`

- [ ] **No exploded workflow format (directory compilation)** -- `apxm compile` only accepts single JSON files or `.ais` DSL files. Docs describe compiling from a directory tree (`my-workflow/` with `agents/`, `flows/`, `nodes/`, `prompts/` subdirectories). No directory-aware frontend exists. Files: `crates/apxm-compiler/src/api/pipeline.rs`, `crates/apxm-graph/src/lib.rs`

- [ ] **No iterative pass convergence** -- Docs say "the pipeline iterates until convergence (no pass makes further changes)." The actual pipeline runs each pass exactly once in sequence with no fixed-point loop. Files: `crates/apxm-compiler/src/passes/pipeline.rs`

## P2: Nice to Have

- [ ] **Future optimization passes not started** -- Docs list 5 future passes: prompt caching, memoization, quality-aware fusion, speculative execution, token compression. None have any implementation. Files: `docs/implementation/compiler/optimization-passes.md`

- [ ] **No profile-guided optimization** -- Gap analysis (Gap 10) describes JIT-style profile-guided adaptation and runtime-configurable latency tiers. No profiling infrastructure exists in the compiler. Files: `crates/apxm-compiler/src/passes/`

- [ ] **No `--no-cse-llm` flag** -- Optimization-passes doc describes a flag to disable CSE for non-zero temperature LLM ops. The flag is not wired in the CLI or pass configuration. Files: `crates/apxm-cli/src/main.rs`, `crates/apxm-compiler/src/passes/pipeline.rs`

- [ ] **Diagnostics are post-hoc, not per-pass** -- `--emit-diagnostics` generates a JSON with node/edge counts after compilation, but does not report per-pass metrics (which pass fired, how many ops it fused/eliminated, time per pass). Files: `crates/apxm-cli/src/main.rs` (line ~604)

- [ ] **No AUTONOMOUS op** -- Gap analysis proposes a new `AUTONOMOUS` AIS operation for model-driven zones within structured DAGs. Not defined in `definitions.rs` or anywhere else. Requires tablegen definition, C++ MLIR op, Rust enum variant, and handler. Files: `crates/apxm-ais/src/operations/definitions.rs`

- [ ] **Templates hardcoded in Rust** -- Templates are compiled into the binary (`template list`). Docs suggest they should be loadable from external files to allow user-defined templates. Files: `crates/apxm-cli/src/main.rs`

_Note: unified WorkflowNode for INV/FLOW_CALL/inner-plan is a cross-cutting concern tracked in master `docs/implementation/TODO.md`._
