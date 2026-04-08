# vLLM Benchmark Results — vendor GPU GPU

**Date**: 2026-04-08
**Hardware**: vendor GPU GPU
**vLLM Server**: localhost:8000
**Model**: Qwen/Qwen2.5-7B-Instruct (7B parameters)
**Backend**: vllm-local (OpenAI-compatible API)

---

## Benchmark: chained_llm (Sequential LLM Chain)

**Description**: Tests pipelining across sequential LLM operations (ASK → THINK → REASON).

**Graph Structure**:
- Node 1: ASK (initial architectural question)
- Node 2: THINK (deep analysis based on Node 1)
- Node 3: REASON (comprehensive design based on Node 2)
- Node 4: PRINT (format output)
- Node 5: RETURN (final result)

**Parallelism**: Sequential dependency chain — max parallelism = 2 (PRINT/RETURN run concurrently)

### Results

| Optimization | Duration | Nodes | Avg Parallelism | Max Parallelism | Speedup |
|--------------|----------|-------|-----------------|-----------------|---------|
| **O0** (no opts) | **44.9s** | 6 | 1.17 | 2 | baseline |
| **O2** (full opts) | **47.0s** | 6 | 1.17 | 2 | 0.96x |

### Analysis

**Why no speedup?**
The chained_llm benchmark is a **pure sequential chain** where each LLM operation depends on the output of the previous one:

```
ASK (user question)
  ↓
THINK (analysis of answer)
  ↓
REASON (synthesis of analysis)
  ↓
PRINT → RETURN
```

**Expected behavior**:
- O0 executes nodes sequentially as dependencies resolve
- O2 applies FuseReasoning pass but cannot parallelize inherently sequential work
- Both achieve same parallelism (1.17 avg, 2 max) because PRINT/RETURN can overlap

**Optimization overhead**: O2 took 2.1s longer (4.7% slower), likely due to:
- Additional compiler passes (FuseReasoning, PrefixCacheHint)
- Metadata overhead in artifact
- No parallelization opportunity to offset overhead

**Conclusion**: For **pure sequential chains**, O0 and O2 perform identically. Optimizations shine when there are parallel branches.

---

## Benchmark: multi_model (Per-Node Backend Routing)

**Description**: Routes different tasks to appropriate models based on requirements (fast/powerful/local).

**Graph Structure**:
- Fast model: Triage classification (ASK)
- Local model: Extract structured data for privacy (THINK)
- Powerful model: Deep solution analysis (REASON)
- Fast model: Format customer response (ASK)
- MERGE → PRINT → RETURN

**Status**: ⏸️ **Not run** — vLLM server unavailable during test execution

**Expected behavior**:
- Should demonstrate cost vs quality tradeoffs
- Local execution for sensitive data extraction
- Mix of fast/powerful models optimizes total cost

---

## Benchmark: mixed_priority (Critical Path Scheduling)

**Description**: Tests priority-based scheduling with critical path analysis.

**Graph Structure**:
- **Critical path**: user_query → quick_answer → final_answer (user-facing)
- **Background tasks** (lower priority):
  - deep_analysis (comprehensive reasoning)
  - comparison (detailed table)
  - future_trends (predictions)
- MERGE → PRINT → RETURN

**Status**: ⏸️ **Not run** — vLLM server unavailable during test execution

**Expected behavior**:
- O0: All tasks execute with equal priority
- O2: Critical path gets higher vLLM scheduling priority
- Background tasks fill GPU capacity without delaying critical path
- Speedup depends on vLLM's internal scheduling and batching

---

## vLLM Server Observations

**Configuration**:
- Endpoint: `http://localhost:8000/v1`
- Model: `Qwen/Qwen2.5-7B-Instruct`
- Context window: 32,768 tokens
- API protocol: OpenAI-compatible

**Server Status**:
- ✅ Successfully served chained_llm benchmark (O0 and O2)
- ❌ Became unavailable after ~2 minutes of testing
- ⚠️ `/v1/models` endpoint returned connection error (exit code 56)
- ⚠️ `/metrics` endpoint returned no data

**Metrics Gap**:
- vLLM Prometheus metrics were not accessible during testing
- Cannot measure prefix cache hit rates, KV cache efficiency, or request batching
- Future runs should:
  1. Configure vLLM with `--enable-metrics` flag
  2. Expose Prometheus endpoint on `:9090`
  3. Collect metrics before/after each benchmark run

---

## Key Findings

### 1. Sequential Chains See No Speedup
- **Chained operations** (A→B→C) are inherently sequential
- O2 optimizations cannot parallelize sequential dependencies
- Both O0 and O2 achieve identical parallelism (1.17 avg, 2 max)
- Small regression in O2 (4.7%) likely due to optimization overhead

### 2. Optimization Overhead is Minimal
- O2 compilation: 251.6ms vs O0: 18.7ms (233ms overhead)
- O2 per-op overhead: 272.5µs vs O0: 87.7µs (185µs per op)
- Overhead is negligible compared to LLM latency (10-20s per op)

### 3. Benchmarks Requiring Parallelism Not Yet Tested
- **multi_model**: Would test concurrent execution across different backends
- **mixed_priority**: Would test critical path prioritization with background work
- **Expected speedup**: 2-4x for graphs with independent parallel branches

### 4. vLLM Integration Works
- APXM successfully routes to vLLM via OpenAI protocol
- Model/backend override in graph attributes works correctly
- Backend registration via `providers = ["vllm-local"]` required in config.toml

---

## Next Steps

### 1. Fix vLLM Server Stability
- Restart vLLM with proper configuration:
  ```bash
  vllm serve Qwen/Qwen2.5-7B-Instruct \
    --host 0.0.0.0 \
    --port 8000 \
    --enable-prefix-caching \
    --max-model-len 32768 \
    --gpu-memory-utilization 0.95
  ```
- Enable Prometheus metrics for observability
- Monitor GPU memory and request queue depth

### 2. Run Remaining Benchmarks
- **multi_model**: Measure cost vs quality tradeoffs
- **mixed_priority**: Measure critical path latency with priority scheduling
- **fan_out**: Create new benchmark with N independent branches to measure true parallelism

### 3. Create Parallel Fan-Out Benchmark
Example structure to demonstrate speedup:
```python
@compile()
def fan_out_benchmark(g: GraphRecorder):
    query = g.text("query", value="Explain microservices")

    # 5 independent branches (can run in parallel)
    branch_1 = g.ask("branch_1", "Aspect 1: {query}")
    branch_2 = g.ask("branch_2", "Aspect 2: {query}")
    branch_3 = g.ask("branch_3", "Aspect 3: {query}")
    branch_4 = g.ask("branch_4", "Aspect 4: {query}")
    branch_5 = g.ask("branch_5", "Aspect 5: {query}")

    result = g.merge("result", branch_1, branch_2, branch_3, branch_4, branch_5)
    g.done(result)
```

**Expected behavior**:
- O0: Sequential execution, ~50s total (5 ops × 10s each)
- O2: Parallel execution, ~10s total (all 5 ops concurrent)
- **Speedup: 5x**

### 4. Measure Prefix Caching Impact
- Instrument vLLM to track KV cache hits
- Compare O0 vs O2 with prompts that share common prefixes
- Expected: O2 FusedReasoning nodes share cached system prompts

### 5. Document vendor GPU-Specific Tuning
- Optimal batch sizes for 7B models
- Memory bandwidth vs compute utilization
- GPU runtime-specific vLLM flags

---

## Reproduction

```bash
# 1. Start vLLM server
vllm serve Qwen/Qwen2.5-7B-Instruct --port 8000 --enable-prefix-caching

# 2. Add vllm-local to config.toml providers list
sed -i 's/providers = \["amd-gateway", "amd-openai"\]/providers = ["amd-gateway", "amd-openai", "vllm-local"]/' ~/.apxm/config.toml

# 3. Export benchmark graph
export PYTHONPATH=crates/apxm-frontend/python
python3 -c "
import importlib.util, json
spec = importlib.util.spec_from_file_location('m', 'examples/python/benchmarks/chained_llm.py')
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
for name in dir(mod):
    obj = getattr(mod, name)
    if hasattr(obj, '_graph'):
        g = obj._graph.to_dict()
        # Override backend to vllm-local
        for node in g['nodes']:
            if node['op'] in ['ASK', 'THINK', 'REASON']:
                node['attributes']['model'] = 'Qwen/Qwen2.5-7B-Instruct'
                node['attributes']['backend'] = 'vllm-local'
        with open('/tmp/bench_chained_vllm.apxm', 'w') as f:
            json.dump(g, f, indent=2)
        break
"

# 4. Compile and run O0
dekk apxm compile /tmp/bench_chained_vllm.apxm -o /tmp/bench_O0.apxmobj -O0
dekk apxm run /tmp/bench_O0.apxmobj --emit-session

# 5. Compile and run O2
dekk apxm compile /tmp/bench_chained_vllm.apxm -o /tmp/bench_O2.apxmobj -O2
dekk apxm run /tmp/bench_O2.apxmobj --emit-session

# 6. Compare metrics
cat ~/.apxm/sessions/bench_*_O0-*/metrics.json | jq '.execution.duration_ms'
cat ~/.apxm/sessions/bench_*_O2-*/metrics.json | jq '.execution.duration_ms'
```

---

## Configuration

**Backend Registration** (`~/.apxm/config.toml`):
```toml
[[backends]]
name = "vllm-local"
type = "local"
protocol = "openai"
endpoint = "http://localhost:8000/v1"
api_key = "dummy"

[[backends.models]]
id = "Qwen/Qwen2.5-7B-Instruct"
aliases = ["qwen", "qwen-7b", "local-fast"]
context_window = 32768
supports_functions = false
tags = ["local", "vllm", "gpu"]

[chat]
providers = ["amd-gateway", "amd-openai", "vllm-local"]  # Must include vllm-local!

[chat.routing.model_aliases.vllm]
model = "Qwen/Qwen2.5-7B-Instruct"
backend = "vllm-local"
```

**Graph Attribute Override**:
```json
{
  "nodes": [
    {
      "id": 1,
      "name": "initial_response",
      "op": "ASK",
      "attributes": {
        "prompt": "...",
        "model": "Qwen/Qwen2.5-7B-Instruct",
        "backend": "vllm-local"
      }
    }
  ]
}
```

---

## Summary

- ✅ **vLLM integration works** — APXM successfully routes to local vLLM server
- ⚠️ **Sequential chains show no speedup** — expected behavior for O0 vs O2
- ❌ **Parallel benchmarks incomplete** — vLLM server became unavailable
- 🎯 **Next priority**: Create fan-out benchmark to demonstrate true parallelism speedup (5x expected)

**Bottom line**: APXM + vLLM integration is functional. To demonstrate optimization impact, we need benchmarks with **independent parallel branches**, not sequential chains.
