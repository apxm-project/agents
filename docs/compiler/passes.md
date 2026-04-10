# Compiler Passes

Located in `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/`.

13 MLIR passes optimize the AIS graph before codegen. The pass pipeline is configured by optimization level (O0-O3) and optional target tuning.

## Pass Inventory

| Pass | File | Category | Effect |
|------|------|----------|--------|
| `normalize-agent-graph` | `NormalizeAgentGraph.cpp` | Infrastructure | Canonicalize graph structure |
| `build-prompt` | `BuildPrompt.cpp` | Infrastructure | Generate template placeholders |
| `fuse-ask-ops` | `FuseAskOps.cpp` | Optimization | Batch LLM calls (highest ROI) |
| `dead-context-elimination` | `DeadContextElimination.cpp` | Optimization | Remove unused context (2.00x speedup) |
| `prompt-canonicalization` | `PromptCanonicalization.cpp` | Optimization | Unify prompt format for prefix caching |
| `template-specialization` | `TemplateSpecialization.cpp` | Optimization | Specialize templates per context |
| `schema-narrowing` | `SchemaNarrowing.cpp` | Optimization | Constrain JSON output schemas |
| `condense-ops` | `CondenseOps.cpp` | Optimization | Fuse adjacent operations |
| `dspy-optimize` | `DspyOptimize.cpp` | Integration | DSPy prompt optimization |
| `capability-scheduling` | `CapabilityScheduling.cpp` | Scheduling | Annotate with cost/tier |
| `assign-priority` | `AssignPriority.cpp` | Scheduling | Assign execution priorities |
| `unconsumed-value-warning` | `UnconsumedValuePass.cpp` | Diagnostic | Warn on unused results |
| CSE (builtin) | — | MLIR Builtin | Common subexpression elimination |
| canonicalizer (builtin) | — | MLIR Builtin | Canonicalization patterns |
| symbol-dce (builtin) | — | MLIR Builtin | Dead symbol elimination |

## Pipeline Configuration

Defined in `crates/compiler/apxm-compiler/src/passes/pipeline.rs`:

| Level | Passes |
|-------|--------|
| **O0** | None (passthrough) |
| **O1** | normalize, build-prompt, dspy-optimize, unconsumed-value-warning, scheduling, fuse-ask-ops, assign-priority, canonicalizer, CSE, symbol-dce |
| **O2** | O1 + prompt-canonicalization, template-specialization, target-specific tuning |
| **O3** | O2 iterated to fixed-point (max 10 iterations) |

## Target Tuning

| Target | Focus |
|--------|-------|
| `Balanced` | Default — all passes at standard weight |
| `Latency` | Prioritize fuse-ask-ops, scheduling |
| `Cost` | Prioritize dead-context-elimination, prompt-canonicalization |
| `Tokens` | Minimize total token usage |
| `Parallelism` | Maximize graph parallelism |

## Proven Effectiveness (April 2026 benchmarks)

| Pass | Speedup | Benchmark |
|------|---------|-----------|
| `dead-context-elimination` | **2.00x** | dead_context_stress (9→3 nodes) |
| `prompt-canonicalization` + vLLM | **1.51x** | shared_prefix_fanout (70% cache hit) |
| CSE | **1.15x** | cse_stress |
| `fuse-ask-ops` | **1.10x** | fusion_stress |

## Pass Metrics

`crates/compiler/apxm-compiler/src/passes/metrics.rs` tracks per-pass timing and whether the pass changed the IR. Emitted via `--emit-diagnostics`.
