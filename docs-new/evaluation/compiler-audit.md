# Compiler Pass Pipeline Audit

Deep dive into the 14 MLIR optimization passes at O2. Companion to the [evaluation scorecard](readme.md).

**Overall effectiveness: ~65%** — 9/14 passes active, 5 incomplete or problematic.

## Pass Pipeline (O2)

Passes execute in this order:

```
 1. normalize                  # Structural: standardize graph
 2. build-prompt               # Structural: construct templates
 3. prompt-canonicalization    # Extract shared prefixes (vLLM blocked)
 4. template-specialization   # Inline constants into templates
 5. unconsumed-value-warning  # Diagnostic: warn on unused outputs
 6. schema-narrowing          # Simplify output schemas
 7. scheduling                # Reorder for parallelism
 8. fuse-ask-ops              # Merge sequential ASK chains
 9. condense-ops              # Batch memory operations
10. dead-context-elimination  # Remove unused context inputs
11. canonicalizer             # MLIR standard canonicalization
12. cse                       # Common subexpression elimination
13. symbol-dce                # Dead code elimination
```

## Pass Audit Table

| Pass | Type | Fires? | ops_delta | Real Impact | Status |
|------|------|--------|-----------|-------------|--------|
| normalize | Structural | Always | 0 | Enables downstream | Required |
| build-prompt | Structural | Always | 0 | Feeds CSE + PromptCanon | Required |
| assign-priority | Structural | Always | 0 | Scheduling input | Required |
| fuse-ask-ops | Optimization | Yes | -2 (fusion_stress) | 1500-2000ms/fused pair | Effective |
| dead-context-elimination | Optimization | Yes | -6 (dead_context) | ~5120 tokens saved | Effective |
| cse | Optimization | Yes (2/9) | -5 ops | Dedup API calls | Effective |
| canonicalizer | Cleanup | Yes (6/9) | -7 ops | Prepares IR | Required |
| symbol-dce | Cleanup | Yes (indirect) | Variable | Removes dead code | Effective |
| scheduling | Scheduling | Yes | 0 | **10% REGRESSION** | Broken |
| template-specialization | Medium | Never | 0 | <50us overhead | Marginal |
| schema-narrowing | Medium | Never | 0 | 10-20% latency (theory) | Untested |
| prompt-canonicalization | Cleanup | Never | 0 | 400-800ms (blocked) | Incomplete |
| condense-ops | Cleanup | Never | 0 | 1-50ms (blocked) | Untested |
| unconsumed-value-warning | Diagnostic | Always | 0 | Warnings only | Dead weight |

## Effectiveness Tiers

### Tier 1: Proven Effective (6 passes)

**fuse-ask-ops** — Merges sequential ASK→ASK chains into single LLM calls.
- Benchmark: `fusion_stress` — 24→22 nodes (1 fused pair)
- Impact: Eliminates 1500-2000ms per fused pair (API roundtrip saved)
- Limitation: Won't fuse ASK→THINK by design (different operation semantics)

**dead-context-elimination** — Removes unused context inputs from templates.
- Benchmark: `dead_context_stress` — 9→3 nodes (-66.7%), **2.00x speedup**
- Impact: ~5120 tokens saved per eliminated context chain
- Best-performing pass in the pipeline

**cse** — Standard MLIR common subexpression elimination.
- Benchmark: `cse_stress` — 10→7 nodes (-30%), 1.15x speedup
- Fires on 2/9 benchmarks (needs duplicate prompts to trigger)
- Reliable: standard MLIR pass, well-tested upstream

**canonicalizer** — MLIR standard canonicalization rules.
- Fires on 6/9 benchmarks, 7 total ops eliminated
- Required for downstream passes to work correctly

**symbol-dce** — Dead code elimination after other passes.
- Catches dangling nodes left by fusion, CSE, dead-context
- Works best as the final pass

**Infrastructure** (normalize, build-prompt, assign-priority) — Required structural passes. Zero ops_delta expected.

### Tier 2: Incomplete or Untested (5 passes)

**scheduling** — Reorders ops to maximize parallelism.
- Benchmark: `priority_scheduling` — O2 is **10% slower** (24.6s→27.3s)
- Parallelism marginally improved (2.61→2.8) but wall-clock worse
- Root cause unknown: may be compilation overhead, mock backend artifact, or scheduler bug
- Same scheduler delivers 1.12x on realistic `mixed_priority` graph

**template-specialization** — Substitutes compile-time constants into templates.
- Never fires on benchmarks. Impact is sub-millisecond (~5-50us per template).
- Docs admit this is "NOT SIGNIFICANT" for LLM-dominated workloads.

**prompt-canonicalization** — Reorders templates for vLLM prefix cache hits.
- **Declared but blocked**: Requires vLLM prefix cache integration (Phase 4).
- Potential impact: 400-800ms per 4000-token cached prefix.
- When tested manually with real vLLM: 1.51x speedup (validated in separate test, not through this pass).

**schema-narrowing** — Removes output_schema from unused results.
- No benchmark graph exercises this pattern.
- Theoretical impact: 10-20% latency reduction per schema-narrowed node.

**condense-ops** — Batches consecutive QMEM/UMEM chains.
- No memory operations in benchmark graphs.
- Theoretical impact: 1-50ms per batched chain.

### Tier 3: Dead Weight (1 pass)

**unconsumed-value-warning** — Issues diagnostics for unused values.
- Runs in O1/O3, adds ~1-2ms compilation overhead per invocation.
- No optimization: diagnostic only. Should be moved to a separate `--warn` flag.

## Pass Profiling Data

Across 9 benchmark graphs at O2, per-pass firing results:

| Pass | Graphs Where It Fires | Total ops_delta |
|------|----------------------|-----------------|
| canonicalizer | 6/9 | -7 |
| cse | 2/9 | -5 |
| All other passes | 0/9 | 0 |

**Interpretation**: The high-impact passes (fuse-ask-ops, dead-context-elimination) produce their effects indirectly — they modify the IR such that canonicalizer and CSE clean up the results. The profiling tool only measures ops_delta per pass invocation, so the true source of optimization is attributed to the cleanup passes.

Source data: [archive/pass-profiling-2026-04-08.md](../archive/pass-profiling-2026-04-08.md)

## Benchmark Results

### fusion_stress (Sequential ASK-THINK chains)

| Metric | O0 | O2 | Delta |
|--------|----|----|-------|
| Nodes executed | 24 | 22 | -2 (-8.3%) |
| Wall time | 57,130ms | 56,349ms | -781ms (-1.4%) |
| Speedup | — | 1.02x | |

Small wall-time impact because sequential chain limits parallelism. Real value is cost savings (fewer API calls).

### dead_context_stress (5 context sources, 1 used)

| Metric | O0 | O2 | Delta |
|--------|----|----|-------|
| Nodes executed | 9 | 3 | -6 (-66.7%) |
| Wall time | 11,991ms | 6,121ms | -5,870ms (-49%) |
| Speedup | — | **2.00x** | |

Best result. Eliminated 4/5 unused context operations plus dependent nodes.

### cse_stress (3 identical prompts)

| Metric | O0 | O2 | Delta |
|--------|----|----|-------|
| Nodes executed | 10 | 7 | -3 (-30%) |
| Wall time | 13,957ms | 12,214ms | -1,743ms (-12.5%) |
| Speedup | — | 1.15x | |

### priority_scheduling (Critical path + background)

| Metric | O0 | O2 | Delta |
|--------|----|----|-------|
| Nodes executed | 18 | 15 | -3 (-16.7%) |
| Wall time | 24,638ms | 27,257ms | +2,619ms (+10.6%) |
| Speedup | — | **0.91x** | REGRESSION |

Node reduction is good (-17%) but wall time increased. Under investigation.

## Root Causes

### Why only 2 passes fire in profiling?

1. **Benchmark graphs are simple**: Current stress tests target specific passes but don't create conditions for others (no memory ops → condense-ops idle, no schema usage → schema-narrowing idle).
2. **Attribution artifact**: fuse-ask-ops and dead-context-elimination modify the IR, but ops_delta is attributed to the cleanup passes that run after them (canonicalizer, CSE).
3. **Conservative heuristics**: FuseAskOps refuses ASK→THINK fusion by design. Template-specialization impact is sub-millisecond.

### Why scheduling regresses?

Hypotheses (not yet confirmed):
1. Compilation overhead of scheduling pass exceeds execution savings on small mock-backend graphs.
2. Fixed 500ms latency neutralizes reordering benefit (all operations take the same time regardless of order).
3. Re-ordering introduces suboptimal dependency chains in specific graph topologies.

## Recommendations

**Immediate:**
1. Investigate scheduling regression — profile CPU overhead vs. execution savings
2. Add stress tests for schema-narrowing, condense-ops, prompt-canonicalization
3. Move unconsumed-value-warning to `--warn` flag

**Medium-term:**
4. Complete vLLM prefix cache integration — unblock prompt-canonicalization
5. Implement variable-latency mock — distinguish scheduling from optimization
6. Add pass-specific metadata to `--emit-diagnostics` (fused_pairs, dead_context_eliminated)

**Long-term:**
7. Dynamic pass selection — skip expensive passes for small graphs
8. Target-aware pass configuration — tune per OptimizationTarget
9. Per-pass micro-benchmarking — measure compilation time vs. runtime savings trade-off
