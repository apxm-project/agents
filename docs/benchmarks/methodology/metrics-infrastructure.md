# Compiler Metrics Infrastructure

> See [optimization passes](../../optimization/passes.md) for pass descriptions.

## Overview

APXM tracks per-pass compiler metrics through the `PassMetrics` / `PipelineDiagnostics` system, surfaced via the `--emit-diagnostics` CLI flag. The infrastructure is comprehensive -- it records op counts, timing, and active-pass detection for all 13 pipeline passes -- but current benchmark graphs are too simple to trigger most optimizations, so nearly every pass reports `ops_delta = 0`.

Artifact size is **not** a reliable proxy for optimization quality. Across 11 stress-test graphs, O2 artifacts are larger than O0 in 10 of 11 cases because passes inject scheduling metadata, caching hints, and priority annotations. The single exception -- `dead_context_stress` -- confirms that passes which genuinely remove IR (dead context elimination) do reduce artifact size.

## PassMetrics Structure

**Location:** `crates/apxm-compiler/src/passes/metrics.rs`

```rust
pub struct PassMetrics {
    pub pass_name: String,   // "normalize", "fuse-ask-ops", etc.
    pub duration_ms: f64,    // Wall-clock time for this pass
    pub ops_before: usize,   // MLIR op count before pass
    pub ops_after: usize,    // MLIR op count after pass
    pub ops_delta: isize,    // Net change (negative = eliminated)
}
```

```rust
pub struct PipelineDiagnostics {
    pub passes: Vec<PassMetrics>,
    pub total_duration_ms: f64,
    pub initial_ops: usize,
    pub final_ops: usize,
}
```

Helper methods on `PipelineDiagnostics`:
- `total_ops_eliminated()` -- sum of all negative deltas.
- `active_passes()` -- passes where `ops_delta != 0`.
- `pass_count()` -- total passes run.

**Implementation detail** (`passes/manager.rs:121-162`): `PassManager::run_with_metrics()` isolates each pass via a temporary `PassManager`, then counts ops by scanning the MLIR textual representation for AIS-dialect prefixes (`ais.`, `arith.`, `cf.`, `scf.`, `return`). This is an approximation but closely tracks actual op count.

## --emit-diagnostics Flag

```bash
apxm compile graph.apxm --emit-diagnostics diag.json
```

Emits a JSON file containing compilation phases, DAG statistics, per-pass metrics, and a summary:

```json
{
  "compilation_phases": {
    "artifact_gen_ms": 0.094,
    "passes_ms": 0.784,
    "total_ms": 18.334
  },
  "dag_statistics": {
    "entry_nodes": 1,
    "exit_nodes": 2,
    "total_edges": 9,
    "total_nodes": 6
  },
  "graph_name": "chained_llm",
  "optimization_level": "O2",
  "pass_metrics": [
    {
      "pass_name": "normalize",
      "duration_ms": 0.025,
      "ops_before": 6,
      "ops_after": 6,
      "ops_delta": 0
    }
  ],
  "pass_summary": {
    "active_passes": [],
    "final_ops": 6,
    "initial_ops": 6,
    "total_ops_eliminated": 0,
    "total_passes": 13
  }
}
```

Behavior by optimization level:
- **O0:** `pass_metrics` is empty (no passes run).
- **O1/O2/O3:** Full pipeline metrics via `compile_graph_with_config_and_diagnostics()`.

**Known issue:** The `--emit-diagnostics` flag is not functional when invoked through the dekk skill wrapper (`skill = true` in `.dekk.toml`). The wrapper silently drops the argument. Workaround: invoke the binary directly with the correct environment.

## Artifact Size Analysis

Benchmark of 11 stress-test graphs from `examples/python/benchmarks/` (APXM 0.2.0):

| Benchmark | O0 (bytes) | O2 (bytes) | Delta | % Change |
|-----------|-----------|-----------|-------|----------|
| chained_llm | 1,704 | 2,513 | +809 | +47% |
| multi_model | 2,570 | 3,685 | +1,115 | +43% |
| shared_prefix_fanout | 7,649 | 8,813 | +1,164 | +15% |
| mixed_priority | 2,392 | 4,146 | +1,754 | +73% |
| fusion_stress | 4,980 | 10,125 | +5,145 | +103% |
| cse_stress | 1,957 | 2,508 | +551 | +28% |
| **dead_context_stress** | **7,293** | **5,688** | **-1,605** | **-22%** |
| prefix_fanout_large | 64,557 | 66,829 | +2,272 | +4% |
| memo_cache_stress | 2,800 | 3,954 | +1,154 | +41% |
| dspy_quality | 1,326 | 2,073 | +747 | +56% |
| priority_scheduling | 3,664 | 5,929 | +2,265 | +62% |

O2 artifacts grow because optimization passes add scheduling metadata, parallelism annotations, caching hints, and verifier data. `dead_context_stress` is the sole reduction -- its dead-context-elimination pass removes unused context parameters, which directly shrinks the artifact.

Typical compilation times: O0 80-150 ms, O2 90-450 ms. For graphs under 20 nodes the overhead is negligible; even `prefix_fanout_large` (100+ nodes) compiles in under 500 ms at O2.

## Why Most Passes Report ops_delta = 0

Current benchmarks lack the IR patterns that trigger transformations:

| Pass | Trigger Pattern | Why Benchmarks Miss It |
|------|----------------|----------------------|
| **fuse-ask-ops** | Adjacent ASK/THINK ops with compatible contexts | Benchmark chains alternate op types or lack context overlap |
| **dead-context-elimination** | Context ops whose outputs are never consumed | All benchmark nodes have data dependencies (no dead code) |
| **symbol-dce** | Symbols referenced nowhere downstream | No orphan symbols in generated graphs |
| **cse-llm-op** | Identical prompts across branches | Benchmark prompts are unique per node |

The `dead_context_stress` graph is the deliberate exception -- it includes nodes with unused context parameters, and indeed shows the only artifact size reduction (-22%) and is expected to report nonzero `ops_delta` when diagnostics are captured.

Expected pass impacts at runtime (independent of artifact size):

| Pass | Runtime Benefit | Metric to Validate |
|------|----------------|--------------------|
| FuseReasoning | Fewer LLM API calls | API call count |
| CSELLMOp | Higher cache hit rate | Unique prompt hash count |
| DeadCodeElimination | Smaller graph, fewer nodes | Node count, artifact size |
| AssignPriority | Better scheduling | Scheduling metadata (increases artifact) |
| NarrowSchema | Simpler JSON schemas | Schema depth/width |

## Known Issues

1. **`--emit-diagnostics` silently fails through dekk skill wrapper.** The `.dekk.toml` `skill = true` flag wraps compile execution through a layer that drops the argument. Compilation succeeds with `--emit-diagnostics /nonexistent/path.json` without error, and no "Wrote diagnostics to..." message appears. Fix: pass through the flag in the skill wrapper, or disable skill wrapping when the flag is present.

2. **Op counting is approximate.** `count_module_ops()` scans MLIR text for known prefixes rather than walking the IR tree. Accurate for AIS-dialect graphs but may drift if custom ops or non-AIS dialects are introduced.

3. **No pass-specific metadata.** `PassMetrics` only tracks aggregate op counts. There is no field for pass-specific data such as fused pair count, tokens saved, or symbols removed. A `metadata: HashMap<String, Value>` extension or a `PassSpecificMetrics` enum would require wiring data from C++ MLIR passes back through FFI.

## Recommendations

**Immediate (no code changes):**
- Use `--emit-diagnostics` for all benchmark runs (invoke binary directly to avoid the skill-wrapper bug).
- Track artifact size deltas (O0 vs O2) alongside op-count deltas -- artifact size alone is misleading.

**Short-term (benchmarks):**
- Create graphs that exercise each optimization pass:
  - `fusion-heavy.apxm` -- adjacent ASK ops with compatible contexts (FuseReasoning).
  - `dead-code.apxm` -- nodes with disconnected outputs (DCE).
  - `caching.apxm` -- duplicate prompts across branches (CSE).
- Run execution benchmarks (API call count, cache hit rate, end-to-end latency) to validate that O2 delivers runtime savings despite larger artifacts.

**Medium-term (implementation):**
- Extend `PassMetrics` with a `metadata` field for pass-specific data (fused pairs, tokens saved, symbols removed).
- Wire metrics from C++ MLIR passes to Rust diagnostics via FFI.
- Fix the skill wrapper to propagate `--emit-diagnostics`.

**Long-term (research):**
- Token-level metrics: estimate token count per prompt op, track cumulative savings, correlate with LLM API costs.
- Standardized benchmark suite with known optimization profiles and expected pass deltas for regression testing.
