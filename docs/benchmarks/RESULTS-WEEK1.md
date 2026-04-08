# Week 1 Benchmark Results: O0 vs O2 Comparison

**Date**: 2026-04-08
**Purpose**: Baseline measurement of compiler optimization passes
**Method**: Stress benchmarks with mock backend (500ms latency)

## Executive Summary

Tested 6 stress benchmarks across different optimization categories:
- **Fusion**: 2% speedup (FuseAskOps)
- **CSE**: 15% speedup (Common Subexpression Elimination)
- **DCE**: **100% speedup** (DeadContextElimination — best performer)
- **Prefix**: 24% speedup (PromptCanonicalization)
- **Priority**: -9% slowdown (needs investigation)
- **Memo**: 5% speedup (MemoCache)

**Key Finding**: DeadContextElimination shows exceptional 2x speedup by removing unused context operations. Priority scheduling shows unexpected regression that needs investigation.

---

## 1. Fusion Stress (FuseAskOps)

**Graph**: 10 sequential ask→think pairs
**Pass Under Test**: FuseAskOps (merge consecutive reasoning ops)

### Results

| Metric | O0 | O2 | Change |
|--------|----|----|--------|
| Nodes Executed | 24 | 22 | **-2 (-8.3%)** |
| Wall Time (ms) | 57,130 | 56,349 | -781 (-1.4%) |
| Duration (ms) | 56,429 | 55,433 | -996 (-1.8%) |
| Avg Parallelism | 1.04 | 1.05 | +0.01 |
| Max Parallelism | 2 | 2 | 0 |
| **Speedup** | - | **1.02x** | - |

### Analysis

- **Node Reduction**: 24 → 22 nodes (-8.3%) shows fusion working
- **Modest Speedup**: 2% improvement is lower than expected
- **Why Small?**: Sequential chain limits parallelism benefit
- **Pass Status**: ✅ Working, but limited by graph structure

---

## 2. CSE Stress (Common Subexpression Elimination)

**Graph**: 3 identical prompts executed in parallel
**Pass Under Test**: CommonSubexprElimination (deduplicate identical ops)

### Results

| Metric | O0 | O2 | Change |
|--------|----|----|--------|
| Nodes Executed | 10 | 7 | **-3 (-30%)** |
| Wall Time (ms) | 13,957 | 12,214 | -1,743 (-12.5%) |
| Duration (ms) | 13,252 | 11,505 | -1,747 (-13.2%) |
| Avg Parallelism | 1.90 | 1.57 | -0.33 |
| Max Parallelism | 3 | 3 | 0 |
| **Speedup** | - | **1.15x** | - |

### Analysis

- **Node Reduction**: 10 → 7 nodes (-30%) — CSE eliminated duplicates
- **Strong Speedup**: 15% improvement from deduplication
- **Parallelism Drop**: Avg parallelism decreased (1.90 → 1.57) because CSE merged parallel nodes
- **Pass Status**: ✅ Working well, good reduction

---

## 3. Dead Context Stress (DeadContextElimination)

**Graph**: 5 context operations, only 1 actually used
**Pass Under Test**: DeadContextElimination (remove unused context)

### Results

| Metric | O0 | O2 | Change |
|--------|----|----|--------|
| Nodes Executed | 9 | 3 | **-6 (-66.7%)** |
| Wall Time (ms) | 11,991 | 6,121 | -5,870 (-49%) |
| Duration (ms) | 10,969 | 5,489 | -5,480 (-50%) |
| Avg Parallelism | 2.22 | 1.33 | -0.89 |
| Max Parallelism | 5 | 2 | -3 |
| **Speedup** | - | **2.00x** | - |

### Analysis

- **Best Performer**: 100% speedup (2x faster)
- **Massive Node Reduction**: 9 → 3 nodes (-66.7%)
- **Why So Effective?**: Removed 4 out of 5 unused context operations
- **Parallelism Impact**: Max parallelism dropped (5 → 2) because dead branches removed
- **Pass Status**: ✅ **Excellent** — most impactful optimization

---

## 4. Prefix Fanout Large (PromptCanonicalization)

**Graph**: 8-way fanout with 4KB shared prefix
**Pass Under Test**: PromptCanonicalization (deduplicate shared prefixes)

### Results

| Metric | O0 | O2 | Change |
|--------|----|----|--------|
| Nodes Executed | 12 | 11 | **-1 (-8.3%)** |
| Wall Time (ms) | 8,233 | 6,763 | -1,470 (-17.9%) |
| Duration (ms) | 7,604 | 6,135 | -1,469 (-19.3%) |
| Avg Parallelism | 3.42 | 3.64 | +0.22 |
| Max Parallelism | 8 | 8 | 0 |
| **Speedup** | - | **1.24x** | - |

### Analysis

- **Good Speedup**: 24% improvement from prefix deduplication
- **Node Reduction**: 12 → 11 nodes (-8.3%)
- **Parallelism Boost**: Avg parallelism increased (3.42 → 3.64)
- **Why Effective?**: Large 4KB shared prefix means significant token savings
- **Pass Status**: ✅ Working well

---

## 5. Priority Scheduling

**Graph**: Critical path (4 sequential ops) + 5 parallel background tasks
**Pass Under Test**: Priority-based scheduling optimization

### Results

| Metric | O0 | O2 | Change |
|--------|----|----|--------|
| Nodes Executed | 18 | 15 | **-3 (-16.7%)** |
| Wall Time (ms) | 24,638 | 27,257 | **+2,619 (+10.6%)** |
| Duration (ms) | 24,004 | 26,279 | **+2,275 (+9.5%)** |
| Avg Parallelism | 2.61 | 2.80 | +0.19 |
| Max Parallelism | 6 | 6 | 0 |
| **Speedup** | - | **0.91x** | - |

### Analysis

- **⚠️ REGRESSION**: 9% slowdown (0.91x)
- **Node Reduction**: 18 → 15 nodes (-16.7%) but still slower
- **Why Slower?**: Priority scheduling overhead may outweigh benefits in mock mode
- **Investigation Needed**:
  - Is priority logic adding overhead?
  - Does mock backend (fixed 500ms latency) hide the benefit?
  - Would real backend with variable latency show benefit?
- **Pass Status**: ⚠️ Needs investigation

---

## 6. Memo Cache Stress

**Graph**: Repeated prompts to test memoization cache effectiveness
**Pass Under Test**: MemoCache (cache and reuse identical LLM calls)

### Results

| Metric | O0 | O2 | Change |
|--------|----|----|--------|
| Nodes Executed | 13 | 9 | **-4 (-30.8%)** |
| Wall Time (ms) | 6,793 | 6,804 | **+11 (+0.2%)** |
| Duration (ms) | 5,999 | 5,696 | -303 (-5.1%) |
| Avg Parallelism | 2.54 | 2.00 | -0.54 |
| Max Parallelism | 6 | 3 | -3 |
| **Speedup** | - | **1.05x** | - |

### Analysis

- **Node Reduction**: 13 → 9 nodes (-30.8%) — cache hit rate is good
- **Minimal Speedup**: 5% improvement (wall time almost unchanged)
- **Why Small?**: Mock backend with fixed latency doesn't show cache benefit
- **Parallelism Impact**: Max parallelism dropped (6 → 3) due to cache hits
- **Expected**: Real backend would show much larger speedup (cache hits = instant)
- **Pass Status**: ✅ Working (cache hits confirmed), but mock mode hides benefit

---

## E2E Benchmark Suite Results

Ran 4 benchmarks from the end-to-end suite (mock mode, 500ms latency):

| Benchmark | Nodes (O0→O2) | Wall Time | Speedup |
|-----------|---------------|-----------|---------|
| shared_prefix_fanout | 7 → 7 | 3,398ms → 3,590ms | **0.95x** |
| chained_llm | 5 → 5 | 2,709ms → 2,812ms | **0.96x** |
| mixed_priority | 9 → 9 | 4,234ms → 3,792ms | **1.12x** |
| multi_model | 7 → 7 | 3,458ms → 3,685ms | **0.94x** |

### E2E Analysis

- **mixed_priority**: Only benchmark showing speedup (12%)
- **All others**: Slight regressions (4-6%)
- **Why Regressions?**: Mock backend with fixed latency + small graphs = overhead dominates
- **Artifact Size**: All O2 artifacts are **larger** (17-76%) due to optimization metadata
- **Conclusion**: E2E suite needs real backend to show meaningful results

---

## Compilation Metrics

### Decompile Warnings

All benchmarks showed decompile warnings (e.g., "Extra data: line 142 column 1"):
- Not a critical error (compilation/execution still works)
- Indicates artifact format may have extra metadata
- Should investigate artifact serialization format

### Compile Times

All benchmarks compiled in 264-269ms (O0 and O2 nearly identical):
- Optimization passes add negligible compile-time overhead
- Good: fast iteration during development
- Compile time is **not** a bottleneck

### Artifact Sizes

E2E benchmarks show O2 artifacts are **17-76% larger**:
- shared_prefix_fanout: 7,955 → 9,378 bytes (+17.9%)
- chained_llm: 1,897 → 2,899 bytes (+52.8%)
- mixed_priority: 2,703 → 4,749 bytes (+75.7%)
- multi_model: 2,876 → 4,250 bytes (+47.8%)

**Why?**: Optimization passes add metadata (dependency info, scheduling hints, etc.)

---

## Summary Table

| Benchmark | Node Reduction | Wall Time Change | Speedup | Pass Status |
|-----------|----------------|------------------|---------|-------------|
| fusion_stress | -8.3% (24→22) | -1.4% | 1.02x | ✅ Working |
| cse_stress | -30% (10→7) | -12.5% | 1.15x | ✅ Good |
| dead_context_stress | **-66.7% (9→3)** | **-49%** | **2.00x** | ✅ **Excellent** |
| prefix_fanout_large | -8.3% (12→11) | -17.9% | 1.24x | ✅ Good |
| priority_scheduling | -16.7% (18→15) | **+10.6%** | 0.91x | ⚠️ Regression |
| memo_cache_stress | -30.8% (13→9) | +0.2% | 1.05x | ✅ Cache working |

---

## Key Insights

### 1. DeadContextElimination is the MVP
- 2x speedup (best performer)
- 67% node reduction
- Clear win for graphs with unused branches

### 2. CSE and Prefix Optimization Work Well
- 15-24% speedup
- 30% node reduction (CSE)
- Strong evidence of effective deduplication

### 3. Priority Scheduling Needs Investigation
- 9% regression is concerning
- Node reduction is good (-17%), but wall time increased
- Hypothesis: mock backend hides benefit or adds overhead

### 4. Mock Backend Limits Insight
- Fixed 500ms latency masks cache benefits
- Real backend with variable latency needed for:
  - MemoCache (cache hits should be instant)
  - Priority scheduling (critical path vs background)
  - Prefix canonicalization (token reduction = cost reduction)

### 5. Artifact Size Increase is Expected
- O2 artifacts 17-76% larger
- Metadata overhead for scheduling/optimization hints
- Acceptable tradeoff for runtime speedup

---

## Action Items

### 1. Investigate Priority Scheduling Regression
- **File**: examples/python/benchmarks/priority_scheduling.py
- **Issue**: 9% slowdown despite 17% node reduction
- **Tasks**:
  - Profile scheduler overhead
  - Test with real backend (variable latency)
  - Check if priority logic has bugs

### 2. Test with Real Backend
- **Blocker**: Mock backend (fixed latency) hides benefits
- **Tasks**:
  - Set up amd-onprem backend
  - Re-run all benchmarks with real LLM
  - Measure token reduction (not just wall time)

### 3. Fix Decompile Warnings
- **Issue**: "Extra data" warnings on all benchmarks
- **Not blocking**: Compilation/execution works
- **Tasks**:
  - Investigate artifact serialization format
  - Check if extra metadata is intentional or a bug

### 4. Add Token Metrics
- **Current**: Only wall time and node count
- **Missing**: Token reduction (input/output)
- **Tasks**:
  - Instrument LLM backend to count tokens
  - Report token savings in benchmark output

---

## Reproduction

To reproduce these results:

```bash
# Individual stress benchmarks
export PYTHONPATH=crates/apxm-frontend/python
python3 scripts/benchmark.py --graph fusion_stress
python3 scripts/benchmark.py --graph cse_stress
python3 scripts/benchmark.py --graph dead_context_stress
python3 scripts/benchmark.py --graph prefix_fanout_large
python3 scripts/benchmark.py --graph priority_scheduling
python3 scripts/benchmark.py --graph memo_cache_stress

# E2E suite (mock mode)
python3 scripts/benchmark_e2e.py --all --mock

# E2E suite (real backend)
python3 scripts/benchmark_e2e.py --all --real  # requires configured backend
```

Raw benchmark data saved to:
- `benchmark_results_*.json` (individual benchmarks)
- `benchmark_e2e_*.json` (e2e suite)

---

## Conclusion

**Week 1 baseline established**. Compiler optimizations show measurable impact:
- **Best**: DeadContextElimination (2x speedup)
- **Good**: CSE, Prefix (15-24% speedup)
- **Modest**: Fusion, MemoCache (2-5% speedup, limited by mock backend)
- **Regression**: Priority scheduling (needs investigation)

**Next Steps**: Real backend testing to measure token reduction and validate cache/priority benefits.
