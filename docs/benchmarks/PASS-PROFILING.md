# MLIR Optimization Pass Profiling

**Date**: 2026-04-08
**Optimization Level**: O2 (Balanced target)
**Benchmarks**: 9 graphs (chained_llm, cse_stress, dead_context_stress, fusion_stress, memo_cache_stress, mixed_priority, multi_model, prefix_fanout_large, shared_prefix_fanout)

## Executive Summary

Of **13 passes** in the O2 pipeline, only **2 passes actually fire**:
- `canonicalizer` (6/9 graphs, 7 ops eliminated total)
- `cse` (2/9 graphs, 5 ops eliminated total)

The remaining **11 passes never transform any operations** across all benchmark graphs.

## Per-Pass Results

| Pass | Fires | Graphs Fired On | Ops Eliminated | Total Time (ms) | Max Time (ms) | Status |
|------|-------|-----------------|----------------|-----------------|---------------|--------|
| **canonicalizer** | 6 | cse_stress, dead_context_stress, fusion_stress, memo_cache_stress, mixed_priority, prefix_fanout_large | 7 | 1.808 | 0.402 | ✅ Active |
| **cse** | 2 | cse_stress, memo_cache_stress | 5 | 0.334 | 0.072 | ✅ Active |
| normalize | 0 | - | 0 | 0.548 | 0.161 | ❌ Never fires |
| build-prompt | 0 | - | 0 | 0.090 | 0.016 | ❌ Never fires |
| prompt-canonicalization | 0 | - | 0 | 0.184 | 0.037 | ❌ Never fires |
| template-specialization | 0 | - | 0 | 0.215 | 0.075 | ❌ Never fires |
| unconsumed-value-warning | 0 | - | 0 | 0.151 | 0.055 | ❌ Never fires (warning-only) |
| schema-narrowing | 0 | - | 0 | 0.105 | 0.021 | ❌ Never fires |
| scheduling | 0 | - | 0 | 0.548 | 0.101 | ❌ Never fires |
| fuse-ask-ops | 0 | - | 0 | 0.200 | 0.083 | ❌ Never fires |
| condense-ops | 0 | - | 0 | 0.121 | 0.035 | ❌ Never fires |
| dead-context-elimination | 0 | - | 0 | 0.269 | 0.084 | ❌ Never fires |
| symbol-dce | 0 | - | 0 | 0.470 | 0.127 | ❌ Never fires |

## Analysis by Benchmark Graph

### CSE Stress (Best Case)
- **Initial ops**: 10 → **Final ops**: 7 (3 eliminated)
- **Active passes**: canonicalizer (-1), cse (-2)
- **Compilation time**: 63.1ms (passes: 1.6ms)

### Memo Cache Stress
- **Initial ops**: 13 → **Final ops**: 9 (4 eliminated)
- **Active passes**: canonicalizer (-1), cse (-3)
- **Compilation time**: 90.6ms (passes: time not individually measured)

### Dead Context Stress
- **Initial ops**: 4 → **Final ops**: 3 (1 eliminated)
- **Active passes**: canonicalizer (-1)
- **Compilation time**: 78.4ms
- **❗ Note**: Named "dead_context_stress" but `dead-context-elimination` pass doesn't fire!

### Fusion Stress
- **Initial ops**: 24 → **Final ops**: 22 (2 eliminated)
- **Active passes**: canonicalizer (-2)
- **Compilation time**: 38.6ms
- **❗ Note**: Named "fusion_stress" but `fuse-ask-ops` pass doesn't fire!

### Chained LLM, Multi Model, Shared Prefix Fanout
- **Ops eliminated**: 0
- **Active passes**: None
- These graphs pass through the pipeline unchanged

## Root Cause Analysis

### Why `fuse-ask-ops` Never Fires

The FuseAskOps pass requires:
1. **Two ASK ops** in a producer-consumer relationship
2. **Single-use chain**: Producer must have exactly one use
3. **Token budget**: Combined template ≤ 2000 tokens (default)
4. **Same dialect attributes**: Compatible model/mode/etc.

**Benchmark issues**:
- `fusion_stress`: Uses ASK→ASK→**THINK** chains — THINK ops aren't fusible with ASK (by design, line 28-29 of FuseAskOps.cpp)
- `chained_llm`: Uses THINK and REASON ops, not ASK
- Other benchmarks: Either no ASK ops or no producer-consumer ASK chains

**Recommendation**: Create a benchmark with pure ASK→ASK→ASK chains:
```python
# fusion_pure_ask.py
ask1 = graph.add_ask("What is X?", name="ask1")
ask2 = graph.add_ask("Given {0}, what is Y?", context=[ask1], name="ask2")
ask3 = graph.add_ask("Given {0}, what is Z?", context=[ask2], name="ask3")
```

### Why `dead-context-elimination` Never Fires

**Hypothesis**: The pass eliminates unused context parameters passed to ASK/THINK/REASON ops. Our benchmarks likely:
1. Don't have extraneous context parameters, OR
2. Have context parameters that ARE used in templates (via `{0}`, `{1}`, etc.)

**Recommendation**: Create a benchmark with unused context:
```python
# dead_context.py
x = graph.add_ask("What is X?")
y = graph.add_ask("What is Y?")
# Pass both but only use x
result = graph.add_ask("Given {0}, answer.", context=[x, y])  # y is dead
```

### Why `condense-ops` Never Fires

**Hypothesis**: CondenseOps merges adjacent string concatenation (MERGE + CONST_STR chains). Benchmarks use template interpolation (`{0}`) instead of explicit MERGE ops.

**Recommendation**: Generate MERGE ops in frontend or test with explicit string building.

### Why `schema-narrowing` Never Fires

**Hypothesis**: SchemaNarrowing optimizes structured output schemas (JSON Schema narrowing). Our benchmarks use free-form text prompts, no structured outputs.

**Recommendation**: Add REASON ops with explicit schema constraints.

### Why `template-specialization` Never Fires

**Hypothesis**: TemplateSpecialization replaces template parameters with constant values when possible. Benchmarks don't have this pattern (parameters are PARAM nodes or runtime values).

### Why `symbol-dce` Never Fires

**Hypothesis**: SymbolDCE removes unused function symbols / module-level declarations. Our single-graph benchmarks don't have unused symbols.

## Performance Observations

1. **Pass overhead is low**: Even non-firing passes take <0.5ms each
2. **Total pass time**: 1-3ms across all 13 passes
3. **Compilation overhead**: Most time (60-160ms) is in MLIR infrastructure, not passes
4. **Canonicalizer is the workhorse**: Fires on 67% of graphs

## Recommendations

### 1. Benchmark Gaps (High Priority)

Create targeted stress tests:
- **`fusion_pure_ask.apxm`**: ASK→ASK→ASK chains (10+ deep)
- **`dead_context.apxm`**: Multiple context params with only subset used
- **`schema_narrowing.apxm`**: REASON ops with JSON Schema that can be narrowed
- **`template_spec.apxm`**: Templates with PARAM nodes that resolve to constants
- **`condense_merge.apxm`**: Explicit MERGE + CONST_STR chains (not template `{0}`)

### 2. Pass Heuristics Review (Medium Priority)

Passes that never fire may have overly conservative heuristics:
- **FuseAskOps**: Should it support ASK→THINK fusion? (Currently forbidden)
- **DeadContextElimination**: Is template analysis too conservative?
- **CondenseOps**: Should it handle template interpolation patterns?

### 3. Documentation (Low Priority)

Update pass documentation with:
- Example input patterns that trigger the pass
- Heuristic thresholds (token budgets, depth limits, etc.)
- Test coverage status

### 4. Pass Pipeline Optimization (Future)

Since 11/13 passes never fire on real workloads:
- Consider **lazy pass evaluation** (skip passes with no matches)
- Add **pass guards** (cheap pre-checks before expensive traversals)
- Profile with **larger graphs** (100+ ops) to see if passes fire at scale

## Appendix: O2 Pipeline Order (Balanced Target)

```
1.  normalize
2.  build-prompt
3.  prompt-canonicalization
4.  template-specialization
5.  unconsumed-value-warning
6.  schema-narrowing
7.  scheduling
8.  fuse-ask-ops           ← Should be high-impact but never fires
9.  condense-ops
10. dead-context-elimination
11. canonicalizer           ← Only consistently active pass
12. cse                     ← Active on graphs with duplicate ops
13. symbol-dce
```

## Appendix: Diagnostic Collection Method

```bash
# For each benchmark graph
dekk apxm compile graph.apxm -O2 --emit-diagnostics diag.json

# Extract per-pass metrics
jq '.pass_metrics[] | {pass_name, ops_delta, duration_ms}' diag.json

# Aggregate across all benchmarks
python3 analyze_passes.py  # See /tmp/analyze_passes.py
```

## Conclusion

The MLIR optimization pipeline is **underutilized** on current benchmarks. Only 2/13 passes provide value, suggesting either:
1. **Benchmarks don't exercise advanced optimizations** (fusion, DCE, schema narrowing)
2. **Pass heuristics are too conservative** (e.g., refusing ASK→THINK fusion)
3. **Real-world graphs are simpler than anticipated**

**Next step**: Build targeted stress tests to validate pass correctness and measure potential impact if heuristics are relaxed.
