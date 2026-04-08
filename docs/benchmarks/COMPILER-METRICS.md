# Compiler Metrics Analysis

**Date**: 2026-04-08
**Research Goal**: Investigate what compiler metrics APXM collects and identify gaps for optimization measurement.

---

## Summary

APXM has a **comprehensive metrics infrastructure** for compiler passes, but current benchmarks don't show optimization impact because the graphs are too simple (no fusing/elimination opportunities). The system is ready for detailed measurement — we just need graphs that exercise the optimization passes.

---

## 1. PassMetrics — What's Already Tracked

**Location**: `crates/apxm-compiler/src/passes/metrics.rs`

### PassMetrics Structure
```rust
pub struct PassMetrics {
    pub pass_name: String,        // "normalize", "fuse-ask-ops", etc.
    pub duration_ms: f64,          // Wall-clock time for this pass
    pub ops_before: usize,         // MLIR op count before pass
    pub ops_after: usize,          // MLIR op count after pass
    pub ops_delta: isize,          // Net change (negative = eliminated)
}
```

### PipelineDiagnostics Structure
```rust
pub struct PipelineDiagnostics {
    pub passes: Vec<PassMetrics>,  // Per-pass metrics in execution order
    pub total_duration_ms: f64,    // Total pipeline time
    pub initial_ops: usize,        // Ops at start
    pub final_ops: usize,          // Ops at end
}
```

**Helper methods**:
- `total_ops_eliminated()` — sum of all negative deltas
- `active_passes()` — list of passes that changed IR (ops_delta ≠ 0)
- `pass_count()` — total number of passes run

**Implementation details** (`crates/apxm-compiler/src/passes/manager.rs:121-162`):
- `PassManager::run_with_metrics()` runs each pass individually
- Uses temporary PassManager that's cleared after each pass to isolate metrics
- `count_module_ops()` scans MLIR textual representation (lines with `ais.`, `arith.`, `cf.`, `scf.`, `return`)
- This is an **approximation** but closely tracks actual op count for AIS dialect

---

## 2. CLI Integration — `--emit-diagnostics` Flag

**Location**: `crates/apxm-cli/src/main.rs:1421-1554`

### Usage
```bash
dekk apxm compile graph.apxm --emit-diagnostics diag.json
```

### What's Emitted (JSON format)
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
  "input": "examples/python/benchmarks/chained_llm.apxm",
  "mode": "graph",
  "optimization_level": "O2",
  "pass_metrics": [
    {
      "pass_name": "normalize",
      "duration_ms": 0.025,
      "ops_before": 6,
      "ops_after": 6,
      "ops_delta": 0
    },
    // ... all 13 passes
  ],
  "pass_summary": {
    "active_passes": [],          // Names of passes with ops_delta != 0
    "final_ops": 6,
    "initial_ops": 6,
    "total_ops_eliminated": 0,
    "total_passes": 13
  }
}
```

### When Diagnostics are Collected
- **O0**: `pass_metrics` is empty (no passes run)
- **O1/O2/O3**: Full pipeline metrics collected via `compile_graph_with_diagnostics()`

**Code path**:
1. `compile_command()` checks if `emit_diagnostics.is_some()`
2. Uses `compiler.compile_graph_with_config_and_diagnostics()` instead of fast path
3. Returns `(Module, PipelineDiagnostics)`
4. Diagnostics are serialized to JSON file at specified path

---

## 3. `apxm analyze` — Runtime Parallelism Analysis

**Tested on**: `examples/python/benchmarks/chained_llm.apxm`

### Human-readable output
```
Analysis: chained_llm
─────────────────────
5 nodes, 6 edges, 5 phases, max parallelism 1

▶ Phase 1 (sequential)
  #1 initial_response ASK [1000ms]
▶ Phase 2 (sequential)
  #2 analysis THINK [5000ms]
...

⚡ Critical path: 5 nodes, ~7020ms
🚀 Speedup: 1.00x (sequential 7020ms → parallel 7020ms)

Suggestions:
  • Graph is fully sequential — no parallelism opportunities
  • Critical path bottleneck: node 2 ('analysis', op=THINK)
```

### JSON output (`--json`)
```json
{
  "critical_path": {
    "estimated_ms": 7020,
    "length": 5,
    "nodes": [1, 2, 3, 4, 5]
  },
  "execution_phases": [
    {
      "phase": 1,
      "parallel": false,
      "parallelism_degree": 1,
      "estimated_ms": 1000,
      "nodes": [{"id": 1, "name": "initial_response", "op": "ASK", "latency_ms": 1000}]
    }
    // ...
  ],
  "max_parallelism": 1,
  "speedup": {
    "estimated_speedup": "1.00x",
    "sequential_ms": 7020,
    "parallel_ms": 7020
  }
}
```

**What it measures**:
- **Runtime parallelism** (how many nodes can run concurrently)
- **Critical path** (longest dependency chain)
- **Estimated speedup** (sequential vs parallel execution time)

**What it does NOT measure**:
- Compiler optimization impact (this is orthogonal to runtime parallelism)
- Token savings
- Fusion opportunities

---

## 4. Artifact Sizes — O0 vs O2 Comparison

**Test**: `examples/python/benchmarks/chained_llm.apxm`

| Opt Level | Artifact Size | Decompiled Lines | Notes |
|-----------|---------------|------------------|-------|
| O0        | 1.9K          | 102 lines        | No optimizations |
| O2        | 2.9K          | 126 lines        | +1K size increase! |

**Surprising finding**: O2 artifacts are **LARGER** than O0.

**Hypothesis**: Optimization passes add metadata/annotations (scheduling hints, caching info, parallelism annotations) that increase artifact size even when op count stays the same.

**Artifact structure** (from `crates/apxm-compiler/mlir/include/ais/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.h`):
```cpp
struct ArtifactEmitOptions {
  std::string moduleName;
  bool emitDebugJson = false;
  std::string targetVersion;
};
```

The emitter (`ArtifactEmitter.cpp`) serializes MLIR modules to binary format. O2 may add:
- Scheduling attributes
- Memoization hints
- Prompt caching metadata
- vLLM priority hints

**Current benchmarks**: All show **ops_delta = 0** for every pass (no ops eliminated).

---

## 5. Per-Pass Metrics: What's Measurable?

### ✅ Already Captured
- **Ops eliminated** (count) — via `ops_delta`
- **Nodes reduced** (before → after) — via `ops_before`, `ops_after`
- **Duration** (ms) — via `duration_ms`
- **Which passes changed IR** — via `active_passes()`

### ❌ Missing but Feasible
1. **Ops fused** (count) — would need pass-specific metadata
2. **Tokens saved** (estimate) — would need to track:
   - Context inputs eliminated (dead-context-elimination)
   - Prompts merged (fuse-ask-ops)
   - Estimated token counts per op
3. **Context inputs eliminated** — DCE pass could emit this
4. **Dead code removed** — symbol-dce could track removed symbols
5. **Fusion pairs** (e.g., ASK→ASK, THINK→ASK) — fuse-ask-ops could log fused pairs

### 🔍 Implementation Path for Missing Metrics

**Option 1: Extend PassMetrics struct**
```rust
pub struct PassMetrics {
    // Existing fields...
    pub pass_name: String,
    pub duration_ms: f64,
    pub ops_before: usize,
    pub ops_after: usize,
    pub ops_delta: isize,

    // New fields for pass-specific metrics
    pub metadata: HashMap<String, serde_json::Value>,  // Pass-specific data
}
```

**Example metadata** for specific passes:
```json
{
  "fuse-ask-ops": {
    "fused_pairs": 2,
    "fusion_types": ["ASK→ASK", "THINK→ASK"]
  },
  "dead-context-elimination": {
    "contexts_removed": 4,
    "tokens_saved_est": 1200
  },
  "symbol-dce": {
    "symbols_removed": ["unused_prompt", "dead_branch"]
  }
}
```

**Option 2: Pass-specific diagnostics structs**
```rust
pub enum PassSpecificMetrics {
    FuseAskOps { fused_pairs: usize, types: Vec<String> },
    DeadContextElim { contexts_removed: usize, tokens_saved: usize },
    SymbolDce { symbols: Vec<String> },
    Generic,  // For passes without specific metrics
}
```

---

## 6. Proposed `--emit-pass-report` Flag

**Goal**: Richer per-pass reporting for optimization analysis.

### Proposed Schema
```json
{
  "passes": [
    {
      "name": "normalize",
      "duration_ms": 2.5,
      "ops_before": 12,
      "ops_after": 12,
      "changed": false
    },
    {
      "name": "fuse-ask-ops",
      "duration_ms": 5.2,
      "ops_before": 12,
      "ops_after": 8,
      "changed": true,
      "fused_pairs": 2,
      "fusion_types": ["ASK→ASK", "THINK→ASK"]
    },
    {
      "name": "dead-context-elimination",
      "duration_ms": 3.1,
      "ops_before": 8,
      "ops_after": 8,
      "changed": false,
      "contexts_removed": 4,
      "tokens_saved_est": 1200
    }
  ],
  "total": {
    "ops_before": 12,
    "ops_after": 8,
    "passes_active": 3,
    "total_duration_ms": 15.8
  }
}
```

**Status**: Not implemented yet, but `--emit-diagnostics` provides the foundation.

**Gap**: Need to wire pass-specific metrics (fused pairs, tokens saved, etc.) through MLIR C++ passes back to Rust.

---

## 7. Current Benchmark Results

### chained_llm.apxm (O2)
```
Passes: 13
Initial ops: 6 → Final ops: 6
Active passes: [] (none changed IR!)
Total duration: 0.78ms
```

**All passes show ops_delta = 0** — no optimization opportunities.

### shared_prefix_fanout.apxm (O2)
```
Passes: 13
Initial ops: 8 → Final ops: 8
Active passes: []
Total duration: 0.22ms
```

**Again, no ops eliminated.**

---

## 8. Why No Optimizations in Current Benchmarks?

**Hypothesis**: The benchmark graphs are too simple — they don't have patterns that trigger optimizations.

### fuse-ask-ops Pass
**What it looks for**: Adjacent ASK/THINK/REASON operations with compatible contexts.

**Example that WOULD fuse**:
```json
{
  "nodes": [
    {"id": 1, "op": "ASK", "prompt": "What is X?"},
    {"id": 2, "op": "ASK", "prompt": "Follow-up question"}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}]
}
```
→ Could fuse into single multi-turn ASK.

**Current benchmarks**: Don't have adjacent fuse-able ops.

### dead-context-elimination Pass
**What it looks for**: Context ops whose outputs are never consumed.

**Example that WOULD eliminate**:
```json
{
  "nodes": [
    {"id": 1, "op": "THINK", "prompt": "Analyze..."},
    {"id": 2, "op": "ASK", "prompt": "Final answer"}
  ],
  "edges": []  // No data edge — node 1 output unused!
}
```
→ Would eliminate node 1.

**Current benchmarks**: All nodes have data dependencies (no dead code).

---

## 9. Recommendations

### Short-term (Ready to Use)
1. **Use `--emit-diagnostics`** for all benchmark runs
2. **Create benchmarks with optimization opportunities**:
   - Adjacent ASK ops (for fusion)
   - Dead context branches (for DCE)
   - Duplicate prompts (for CSE)
   - Shared prefix patterns (for prompt caching)

3. **Track artifact size deltas** (O0 vs O2) in benchmark results

### Medium-term (Needs Implementation)
4. **Add pass-specific metadata** to PassMetrics:
   - Fused pairs count (fuse-ask-ops)
   - Tokens saved estimate (dead-context-elimination, prompt-caching)
   - Symbols removed (symbol-dce)

5. **Wire metrics from C++ passes** to Rust diagnostics:
   - Extend MLIR pass base to collect metadata
   - Surface via FFI to Rust PassMetrics

6. **Create `--emit-pass-report` alias** for `--emit-diagnostics` (marketing name)

### Long-term (Research)
7. **Token-level metrics**:
   - Estimate token count per prompt op
   - Track cumulative token savings across passes
   - Correlate with actual LLM API costs

8. **Benchmark suite** with known optimization profiles:
   - `fusion-heavy.apxm` — exercises fuse-ask-ops
   - `dead-code.apxm` — exercises DCE
   - `caching.apxm` — exercises prompt caching
   - `complex.apxm` — all optimizations combined

---

## 10. Example: What Benchmark Metrics Could Look Like

**Hypothetical**: `fusion-heavy.apxm` compiled at O2

```json
{
  "pass_metrics": [
    {
      "pass_name": "fuse-ask-ops",
      "duration_ms": 5.2,
      "ops_before": 12,
      "ops_after": 8,
      "ops_delta": -4,
      "metadata": {
        "fused_pairs": 2,
        "fusion_types": ["ASK→ASK", "THINK→ASK"],
        "tokens_saved_est": 800
      }
    },
    {
      "pass_name": "dead-context-elimination",
      "duration_ms": 3.1,
      "ops_before": 8,
      "ops_after": 6,
      "ops_delta": -2,
      "metadata": {
        "contexts_removed": 2,
        "tokens_saved_est": 600
      }
    }
  ],
  "pass_summary": {
    "active_passes": ["fuse-ask-ops", "dead-context-elimination"],
    "initial_ops": 12,
    "final_ops": 6,
    "total_ops_eliminated": 6,
    "total_tokens_saved_est": 1400
  }
}
```

**This would demonstrate**: 50% op reduction, clear optimization value.

---

## Conclusion

**What exists**: Solid metrics foundation (PassMetrics, PipelineDiagnostics, --emit-diagnostics).

**What's missing**:
1. Benchmarks that exercise optimization passes
2. Pass-specific metadata (fused pairs, tokens saved)
3. Token-level tracking

**Next steps**: Create benchmarks with optimization opportunities to validate metrics infrastructure.
