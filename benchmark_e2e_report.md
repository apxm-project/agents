================================================================================
END-TO-END BENCHMARK RESULTS
Compiler + Runtime Optimization Impact
================================================================================

Generated: 2026-04-08 00:52:41
Mode: MOCK
Mock Latency: 200ms


## shared_prefix_fanout
**Description**: Tests prefix reuse optimization (4 parallel ASK with shared context)
**Expected**: O2 should deduplicate shared prefix, reducing tokens sent

### Compilation Phase

| Metric | O0 | O2 | Improvement |
|--------|----|----|-------------|
| Nodes | 7 | 7 | +0.0% |
| Est. Tokens | 1648 | 1648 | +0.0% |
| Artifact Size (bytes) | 7955 | 9378 | -17.9% |
| Compile Time (ms) | 266 | 263 | N/A |

### Execution Phase

| Metric | O0 | O2 | Improvement |
|--------|----|----|-------------|
| Wall Time (ms) | 2989.102602005005 | 3155.84397315979 | -5.6% |
| Execution Time (ms) | 2411 | 2597 | -7.7% |
| Nodes Executed | 8 | 8 | +0.0% |
| LLM Calls | 0 | 0 | N/A |
| Input Tokens | 0 | 0 | N/A |
| Output Tokens | 0 | 0 | N/A |
| Cache Hits | 0 | 0 | N/A |
| Avg Parallelism | 1.88 | 1.88 | - |
| Max Parallelism | 4 | 4 | +0.0% |

**Overall Speedup**: 0.95x


## chained_llm
**Description**: Tests sequential ASK chain optimization
**Expected**: O2 may fuse operations or optimize data flow

### Compilation Phase

| Metric | O0 | O2 | Improvement |
|--------|----|----|-------------|
| Nodes | 5 | 5 | +0.0% |
| Est. Tokens | 224 | 224 | +0.0% |
| Artifact Size (bytes) | 1897 | 2899 | -52.8% |
| Compile Time (ms) | 259 | 260 | N/A |

### Execution Phase

| Metric | O0 | O2 | Improvement |
|--------|----|----|-------------|
| Wall Time (ms) | 2464.2927646636963 | 2516.139268875122 | -2.1% |
| Execution Time (ms) | 1893 | 1945 | -2.7% |
| Nodes Executed | 6 | 6 | +0.0% |
| LLM Calls | 0 | 0 | N/A |
| Input Tokens | 0 | 0 | N/A |
| Output Tokens | 0 | 0 | N/A |
| Cache Hits | 0 | 0 | N/A |
| Avg Parallelism | 1.17 | 1.17 | - |
| Max Parallelism | 2 | 2 | +0.0% |

**Overall Speedup**: 0.98x


## mixed_priority
**Description**: Tests priority-based scheduling with parallel branches
**Expected**: O2 should optimize scheduling for critical path

### Compilation Phase

| Metric | O0 | O2 | Improvement |
|--------|----|----|-------------|
| Nodes | 9 | 9 | +0.0% |
| Est. Tokens | 250 | 250 | +0.0% |
| Artifact Size (bytes) | 2703 | 4749 | -75.7% |
| Compile Time (ms) | 263 | 527 | N/A |

### Execution Phase

| Metric | O0 | O2 | Improvement |
|--------|----|----|-------------|
| Wall Time (ms) | 3695.1820850372314 | 3894.2224979400635 | -5.4% |
| Execution Time (ms) | 3124 | 3272 | -4.7% |
| Nodes Executed | 10 | 9 | +10.0% |
| LLM Calls | 0 | 0 | N/A |
| Input Tokens | 0 | 0 | N/A |
| Output Tokens | 0 | 0 | N/A |
| Cache Hits | 0 | 0 | N/A |
| Avg Parallelism | 1.70 | 1.78 | - |
| Max Parallelism | 4 | 4 | +0.0% |

**Overall Speedup**: 0.95x


## multi_model
**Description**: Tests multi-model routing optimization
**Expected**: O2 should optimize model selection strategy

### Compilation Phase

| Metric | O0 | O2 | Improvement |
|--------|----|----|-------------|
| Nodes | 7 | 7 | +0.0% |
| Est. Tokens | 351 | 351 | +0.0% |
| Artifact Size (bytes) | 2876 | 4250 | -47.8% |
| Compile Time (ms) | 262 | 260 | N/A |

### Execution Phase

| Metric | O0 | O2 | Improvement |
|--------|----|----|-------------|
| Wall Time (ms) | 3348.0923175811768 | 3117.715358734131 | +6.9% |
| Execution Time (ms) | 2557 | 2553 | +0.2% |
| Nodes Executed | 8 | 8 | +0.0% |
| LLM Calls | 0 | 0 | N/A |
| Input Tokens | 0 | 0 | N/A |
| Output Tokens | 0 | 0 | N/A |
| Cache Hits | 0 | 0 | N/A |
| Avg Parallelism | 1.25 | 1.25 | - |
| Max Parallelism | 2 | 2 | +0.0% |

**Overall Speedup**: 1.07x


================================================================================
SUMMARY: O0 vs O2 Comparison
================================================================================

| Benchmark | Nodes (O0→O2) | Tokens (O0→O2) | Wall Time (O0→O2) | Speedup |
|-----------|---------------|----------------|-------------------|---------|
| shared_prefix_fanout | 7 → 7 | 1648 → 1648 | 2989ms → 3156ms | 0.95x |
| chained_llm | 5 → 5 | 224 → 224 | 2464ms → 2516ms | 0.98x |
| mixed_priority | 9 → 9 | 250 → 250 | 3695ms → 3894ms | 0.95x |
| multi_model | 7 → 7 | 351 → 351 | 3348ms → 3118ms | 1.07x |
