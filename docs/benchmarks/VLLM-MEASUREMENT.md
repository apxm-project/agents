# vLLM Inference-Time Optimization Measurement Methodology

**Date**: April 8, 2026
**Status**: Research findings and recommended methodology
**Scope**: Measuring vLLM-level optimizations (priority scheduling, KV-cache pinning, shared-prefix warmup, prompt canonicalization)

---

## Executive Summary

APXM's vLLM optimizations happen at **inference time**, not compile time. These optimizations reduce inference latency and token waste through:
- **Priority scheduling**: critical-path requests served first
- **KV-cache pinning**: prefix blocks stay warm for downstream reuse
- **Shared-prefix warmup**: prefill shared context before fan-out
- **Prompt canonicalization**: reorder prompts so shared context is a real prefix

This document defines metrics, benchmarks, and measurement methodology based on research into vLLM, SGLang, and graph-aware LLM serving systems.

---

## 1. Metrics That Matter for Inference-Time Optimization

### 1.1 Primary Metrics

| Metric | Why It Matters | Source | Expected Impact |
|--------|---------------|--------|-----------------|
| **Prefix cache hit rate** | Direct measure of reuse success | vLLM `/metrics` | +50-99% hits on well-shaped graphs |
| **TTFT (time to first token)** | Critical-path user experience | vLLM histogram | -40-60% on cache hits |
| **Prefill time** | Reduced by prefix cache hits | Per-request timing | -20-80% when reusing large prefixes |
| **Total prefill tokens** | Wasted computation indicator | Derived from queries/hits | -20-60% on shared-prefix fan-out |
| **E2E graph completion time** | What users actually feel | APXM session metrics | Target: 15-40% improvement |
| **Queue wait time** | Reduced by priority scheduling | Derived from scheduling | Better tail latency on mixed loads |

### 1.2 Health and Safety Metrics

| Metric | Purpose | Threshold |
|--------|---------|-----------|
| **KV-cache usage %** | Prevent memory exhaustion | Alert at >92%, downgrade pins at >97% |
| **Throughput (requests/sec)** | Ensure optimizations don't destroy capacity | <10% regression acceptable |
| **P95/P99 latency** | Catch tail latency regressions | <15% regression acceptable |
| **Active pins / pinned bytes** | Memory pressure from pinning | Monitor for leaks |
| **Pin release reasons** | Debug pinning failures | Track `consumed`, `ttl_expired`, `memory_pressure` |

### 1.3 Derived Metrics

- **Prefix cache hit rate** = `vllm:prefix_cache_hits / vllm:prefix_cache_queries`
- **Repeated prefill tokens avoided** = `(1 - hit_rate) * avg_shared_prefix_tokens * num_requests`
- **Warmup reuse ratio** = `warmup_requests_reused / warmup_requests_issued`
- **Critical-path speedup** = `baseline_critical_path_time / optimized_critical_path_time`

---

## 2. What Other Systems Measure

### 2.1 SGLang / RadixAttention (arXiv:2312.07104)

**Key paper**: "SGLang: Efficient Execution of Structured Language Model Programs"

**Benchmarks**:
- Cache hit rates: 50% to nearly 99% across benchmarks
- Throughput improvements: 5-6× higher than baselines (Guidance, vLLM without RadixAttention)
- Latency: Significant reduction in first token latency when prefix cache hits
- Multi-modal: 6× throughput on llava-bench-in-the-wild (same image, multiple questions)

**Workloads tested**:
- Agent, reasoning, extraction, chat, few-shot learning
- Models: Llama-7B, Mixtral-8x7B on NVIDIA A10G GPUs

**Key insight**: RadixAttention maintains LRU cache of KV blocks in a radix tree for efficient matching/eviction.

### 2.2 Helium (Workflow-Aware Serving Framework)

**Approach**: Models agentic workloads as query plans with LLM invocations as first-class operators

**Optimizations**:
- Proactive caching with cache-aware scheduling
- Templated radix tree to capture prompt structure and dependencies
- Maximize prefix cache reuse across batch agentic workloads

**Metrics**:
- End-to-end workflow latency
- Prefix cache reuse rate across batches
- Critical-path request prioritization

### 2.3 Teola (Towards End-to-End Optimization, arXiv:2407.00326)

**Approach**: Parse user queries into primitive-level dataflow graphs

**Results**:
- Up to 2.09× speedup in end-to-end latency
- Application-aware scheduling and batching based on graph topology

**Key insight**: Request correlations and dependencies enable better scheduling decisions.

### 2.4 Autellix (LLM Agents as General Programs, arXiv:2502.13965)

**Approach**: Model multi-threaded programs as dynamic DAGs of LLM calls

**Metrics**:
- Critical-path completion time (longest sequence of dependent calls)
- End-to-end response times and throughput

**Optimization**: Use program-level statistics (cumulative service times) to prioritize LLM calls.

### 2.5 Production Case Study (llm-d.ai, 2025)

**Results**: Prefix-cache aware scheduling delivered:
- **57× faster response times** on identical hardware
- **Double the throughput**

**Conclusion**: "KV-cache hit rate is the single most important metric for a production-stage AI agent."

---

## 3. What vLLM Exposes

### 3.1 Prometheus Metrics Endpoint

vLLM provides a `/metrics` HTTP endpoint in Prometheus Exposition Format. All metrics use the `vllm:` prefix.

**Enabling metrics**:
```bash
vllm serve <model> --disable-log-requests --enable-metrics
```

**Accessing metrics**:
```bash
curl http://localhost:8000/metrics
```

### 3.2 KV Cache Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `vllm:prefix_cache_queries` | Counter | Total number of prefix cache lookups |
| `vllm:prefix_cache_hits` | Counter | Number of successful prefix cache hits |
| `vllm:kv_cache_usage_perc` | Gauge | Percentage of used KV cache blocks |

**Derived hit rate**:
```
hit_rate = vllm:prefix_cache_hits / vllm:prefix_cache_queries
```

### 3.3 Latency Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `vllm:time_to_first_token_seconds` | Histogram | Distribution of TTFT across requests |
| `vllm:e2e_request_latency_seconds` | Histogram | End-to-end request latency |

### 3.4 Advanced Cache Metrics (with `--kv-cache-metrics-sample`)

vLLM emits histograms for:
- **KV block residency time**: how long blocks stay resident in cache
- **KV block reuse count**: how often blocks are reused

**Sampling reduces overhead** while still providing useful distributions.

### 3.5 Benchmark Tools

vLLM provides `benchmarks/benchmark_prefix_caching.py` for testing prefix cache behavior:
- Synthetic shared-prefix workloads
- Configurable prefix length and fan-out
- Measures throughput and latency improvements

---

## 4. Benchmark Methodology

### 4.1 Three-Tier Approach

#### Tier 1: Simulated (Fast Iteration)
**Purpose**: Rapid hypothesis testing and algorithm validation

**Approach**:
- Mock vLLM backend with configurable prefix cache behavior
- Simulate hit/miss probabilities based on graph structure
- Measure theoretical savings in prefill tokens

**Pros**: Fast, reproducible, no infrastructure needed
**Cons**: May not capture real scheduler behavior or memory pressure

**When to use**: Early development, algorithm prototyping

#### Tier 2: Real vLLM Instance (Ground Truth)
**Purpose**: Measure actual inference-time behavior

**Approach**:
- Deploy vLLM with Prometheus metrics enabled
- Run APXM graphs against real vLLM instance
- Scrape `/metrics` endpoint during execution
- Correlate APXM node IDs with vLLM request IDs

**Pros**: Real behavior, actual memory pressure, true scheduler dynamics
**Cons**: Slower, requires infrastructure, harder to debug

**When to use**: Validation, performance characterization, production readiness

#### Tier 3: Comparative (With/Without APXM Hints)
**Purpose**: Isolate APXM optimization benefits

**Approach**:
- Baseline: Same graph, no APXM hints (priority, reuse_group, pin_policy)
- Optimized: Full APXM metadata in `vllm_xargs.apxm`
- Compare metrics side-by-side

**Pros**: Direct A/B comparison, clear attribution
**Cons**: Requires vLLM patches for pinning, double the test runs

**When to use**: Exit criteria validation, performance marketing

### 4.2 Recommended Measurement Stack

```
┌─────────────────────────────────────────────┐
│ APXM Execution Layer                        │
│ - Session tracing (trace.ndjson)           │
│ - Node-level timing (node_statuses.json)   │
│ - Graph completion time (metrics.json)     │
└─────────────────┬───────────────────────────┘
                  │
┌─────────────────▼───────────────────────────┐
│ vLLM Layer                                  │
│ - Prometheus /metrics endpoint             │
│ - Per-request logging (with graph_id)      │
│ - KV cache block manager telemetry         │
└─────────────────┬───────────────────────────┘
                  │
┌─────────────────▼───────────────────────────┐
│ Observability Stack                         │
│ - Prometheus (scrape every 5s)             │
│ - Grafana (visualization + alerting)       │
│ - Custom analysis scripts (correlate APXM  │
│   session with vLLM metrics)               │
└─────────────────────────────────────────────┘
```

### 4.3 Correlation Strategy

**Challenge**: Link APXM node execution with vLLM metrics

**Solution**: Use `vllm_xargs.apxm.execution_id` and `node_id`

1. APXM emits execution_id in session manifest
2. Each LLM request includes `execution_id` + `node_id` in vllm_xargs
3. vLLM logs requests with these IDs (if patched to emit them)
4. Post-processing script joins:
   - `~/.apxm/sessions/<exec-id>/trace.ndjson` (APXM events)
   - Prometheus scrapes (vLLM counters)
   - vLLM request logs (if available)

**Output**: Per-node metrics showing cache hits, TTFT, prefill time

---

## 5. Benchmark Shapes for Inference-Time Optimization

### 5.1 Shared-Prefix Fan-Out

**Purpose**: Stress prefix cache reuse

**Graph structure**:
```
CONST_STR(large_shared_context) → 8 × ASK(review_aspects)
```

**Prompt shape after canonicalization**:
```
<4K token diff>
---
Review focus: security
```

**Expected metrics**:
- Baseline (no optimization): 7/8 requests miss cache (only first hits on itself partially)
- Optimized: 7/8 requests hit cache on shared 4K prefix
- Prefill token savings: ~28K tokens (7 × 4K)
- TTFT reduction: 40-60% for downstream requests

**Variations**:
- Fan-out of 4, 8, 16 to test cache capacity
- Prefix size: 1K, 4K, 8K, 16K tokens
- Sequential vs. parallel dispatch

### 5.2 Priority Mix Under Load

**Purpose**: Validate priority scheduling

**Graph structure**:
```
Critical path:    ASK → ASK → ASK (user-visible)
Background path:  20 × ASK (speculative/logging)
```

**Request priorities**:
- Critical: `priority=0`
- Background: `priority=15`

**Expected metrics**:
- Baseline: Critical and background interleaved randomly
- Optimized: Critical path completes faster despite background load
- Queue wait time: Lower for critical requests
- E2E critical path time: 20-40% faster under contention

**Variations**:
- Different load ratios (1:5, 1:10, 1:20 critical:background)
- Concurrent execution from multiple graphs

### 5.3 Sequential Chain (Pinning + Warmup)

**Purpose**: Measure KV-cache pinning benefits

**Graph structure**:
```
ASK(draft) → ASK(review) → ASK(refine)
```

**Optimization**:
- Draft output becomes prefix for review
- Pin draft's KV cache for 30s
- Review reuses pinned prefix

**Expected metrics**:
- Baseline: Review misses cache (draft evicted)
- Optimized: Review hits cache on pinned draft output
- TTFT for review: 30-50% faster
- Pinned memory overhead: Track `vllm:kv_cache_usage_perc`

**Variations**:
- Chain length: 2, 3, 5 nodes
- Pin TTL: 10s, 30s, 60s
- Memory pressure: Run with limited KV cache capacity

### 5.4 Large Shared Context with Multiple Consumers

**Purpose**: Validate warmup strategy

**Graph structure**:
```
WARMUP(16K_shared_prefix) → 12 × ASK(different_tasks)
```

**Warmup behavior**:
- Pre-fill 16K token context
- Generate 1 cheap token
- Mark for reuse with `reuse_group="task-batch"`

**Expected metrics**:
- Baseline: All 12 requests prefill 16K tokens (192K total)
- Optimized: Warmup prefills once, 12 requests hit cache (16K total)
- Prefill token savings: 176K tokens (91.7% reduction)
- Warmup overhead: 1 extra request, ~500ms
- Net latency win: Significant if fan-out ≥ 4

**Variations**:
- Prefix size threshold: 4K, 8K, 16K, 32K
- Fan-out threshold: 2, 4, 8, 12
- Cost vs. latency target (warmup may not be worth it for cost-optimized graphs)

### 5.5 Cancellation Under Load

**Purpose**: Verify pin cleanup and graph lifecycle

**Graph structure**:
```
ASK(long_running) with downstream pins
Cancel execution mid-flight
```

**Expected metrics**:
- Pinned blocks released within 1s of cancellation
- No leaked pins in `vllm:active_pins` counter
- Memory freed: `vllm:kv_cache_usage_perc` returns to baseline

**Acceptance criteria**:
- Zero leaked pins after 100 cancellations
- Memory released correctly under pressure

### 5.6 Memory Pressure Stress

**Purpose**: Validate safe degradation under memory limits

**Setup**:
- Limit vLLM to 80% of usual KV cache capacity
- Run high-concurrency workload with pinning enabled

**Expected behavior**:
- KV usage 85-92%: Critical-path pins only
- KV usage 92-97%: Downgrade pins to normal cache behavior
- KV usage >97%: Release non-critical pins immediately

**Metrics**:
- Pin release reasons: Expect high `memory_pressure` count
- Throughput: <10% regression vs. no-pinning baseline
- P99 latency: <15% regression

---

## 6. Metrics Collection Implementation

### 6.1 APXM Side

**Session metrics** (already exists):
```json
{
  "execution_id": "exec-2026-04-08-001",
  "graph_completion_time_ms": 2450,
  "critical_path_time_ms": 1800,
  "nodes": [
    {
      "node_id": 3,
      "name": "review_security",
      "start_time": "2026-04-08T12:00:01.234Z",
      "end_time": "2026-04-08T12:00:02.100Z",
      "ttft_ms": 320,
      "total_time_ms": 866,
      "prefill_tokens": 4200
    }
  ]
}
```

**New APXM metrics to add**:
- `warmup_requests_issued`
- `warmup_requests_reused` (inferred from downstream cache hits)
- `shared_prefix_groups` (count of reuse groups in graph)
- `priority_classes` (distribution of critical/normal/background)

### 6.2 vLLM Side

**Existing metrics** (use as-is):
- `vllm:prefix_cache_queries`
- `vllm:prefix_cache_hits`
- `vllm:kv_cache_usage_perc`
- `vllm:time_to_first_token_seconds`

**Proposed new metrics** (requires vLLM patch):
- `vllm:apxm_active_pins` (Gauge)
- `vllm:apxm_pinned_blocks` (Gauge)
- `vllm:apxm_pinned_bytes` (Gauge)
- `vllm:apxm_pin_releases_total{reason}` (Counter)
- `vllm:apxm_graph_requests_total{priority_class}` (Counter)

### 6.3 Correlation Script

**Input**:
- APXM session directory: `~/.apxm/sessions/<exec-id>/`
- Prometheus query API: `http://localhost:9090/api/v1/query_range`

**Output**:
```json
{
  "execution_id": "exec-2026-04-08-001",
  "summary": {
    "cache_hit_rate": 0.875,
    "prefill_tokens_saved": 28672,
    "avg_ttft_ms": 285,
    "critical_path_speedup": 1.42
  },
  "per_node": [
    {
      "node_id": 3,
      "name": "review_security",
      "cache_hit": true,
      "ttft_ms": 195,
      "prefill_tokens_saved": 4096
    }
  ]
}
```

---

## 7. Exit Criteria for vLLM Optimizations

### 7.1 Workstream A: No-Fork Wins (Priority + Warmup)

**Success criteria**:
1. **Metadata transmission**: APXM hints visible in vLLM request logs
2. **Priority scheduling**: Critical-path requests show 20-40% lower queue wait time under mixed load
3. **Shared-prefix shaping**: Fan-out graphs show 40-70% cache hit rate (vs. <10% baseline)
4. **Warmup benefit**: Net latency win on graphs with ≥4 fan-out and ≥8K shared prefix

**Metrics to achieve**:
- Prefix cache hit rate: >50% on target graphs
- Prefill token savings: >30% on shared-prefix fan-out
- E2E critical-path time: 15-30% faster under contention

### 7.2 Workstream B: KV-Cache Pinning

**Success criteria**:
1. **Reuse determinism**: Downstream requests hit cache ≥95% of the time when upstream is pinned
2. **Health**: Throughput regression <10%, P99 latency regression <15%
3. **Cleanup**: Zero leaked pins after 100 cancellations
4. **Memory safety**: Graceful degradation under pressure (≥90% cache usage)

**Metrics to achieve**:
- Pin hit rate: >95% for eligible downstream nodes
- Prefill token savings: >50% on chained graphs
- E2E latency: 20-40% faster on sequential chains

### 7.3 Workstream C: Token Pipelining (Research)

**Go/No-Go criteria**:
1. **Beats baseline**: Must show >15% E2E latency win vs. Workstream A+B on chained `ASK → ASK` graphs
2. **Stability**: <5% variance across 10 runs
3. **Complexity**: Implementation <500 LOC in APXM, <1000 LOC in vLLM

**If criteria not met**: Stop. Do not productize.

---

## 8. Recommended Measurement Plan

### Phase 0: Baseline (Week 1)

**Goal**: Know current behavior before optimization

**Tasks**:
1. Deploy vLLM with Prometheus metrics enabled
2. Create 5 benchmark graphs (fan-out, priority-mix, chain, warmup, cancellation)
3. Run baseline measurements (no APXM hints)
4. Collect: cache hit rate, TTFT distribution, E2E latency, throughput

**Deliverable**: Baseline metrics report

### Phase 1: Priority + Metadata (Week 2-3)

**Goal**: APXM hints reach vLLM

**Tasks**:
1. Implement `vllm_xargs.apxm` serialization in APXM backend
2. Implement priority export from APXM graph analysis
3. Run comparative benchmarks (with/without hints)
4. Validate priority scheduling behavior

**Deliverable**: Priority scheduling validation report

### Phase 2: Prefix Shaping + Warmup (Week 4-5)

**Goal**: Improve reuse without vLLM fork

**Tasks**:
1. Implement `PromptCanonicalization` compiler pass
2. Implement warmup heuristic in APXM runtime
3. Run fan-out and warmup benchmarks
4. Measure cache hit rate improvement

**Deliverable**: Prefix-shaping performance report

### Phase 3: KV-Cache Pinning (Week 6-8)

**Goal**: Deterministic reuse with vLLM patch

**Tasks**:
1. Implement vLLM pin policy handling
2. Add Prometheus metrics for pinning
3. Run chain and memory-pressure benchmarks
4. Validate cleanup and degradation

**Deliverable**: Pinning performance and safety report

### Phase 4: Decision Point (Week 9)

**Review**:
- Did Phase 1-3 deliver expected benefits?
- Is there significant latency left on the table?
- Is pipelining research justified?

**Outcomes**:
- **If yes**: Proceed to pipelining prototype
- **If no**: Declare victory, focus on hardening and productization

---

## 9. Tooling and Infrastructure

### 9.1 Recommended Setup

**vLLM deployment**:
```bash
vllm serve meta-llama/Llama-4-Maverick-17B-128E \
  --host 0.0.0.0 \
  --port 8000 \
  --enable-metrics \
  --kv-cache-metrics-sample \
  --disable-log-requests
```

**Prometheus config** (`prometheus.yml`):
```yaml
scrape_configs:
  - job_name: 'vllm'
    scrape_interval: 5s
    static_configs:
      - targets: ['localhost:8000']
```

**Grafana dashboard**: Use vLLM's reference dashboard or build custom with:
- Cache hit rate panel: `rate(vllm:prefix_cache_hits[1m]) / rate(vllm:prefix_cache_queries[1m])`
- TTFT distribution: `histogram_quantile(0.95, vllm:time_to_first_token_seconds_bucket)`
- KV usage: `vllm:kv_cache_usage_perc`

### 9.2 Analysis Scripts

**Location**: `tools/benchmarks/analyze_vllm_metrics.py`

**Functions**:
- Parse APXM session directory
- Query Prometheus for time range matching execution
- Correlate node IDs with vLLM requests
- Compute derived metrics (savings, speedup)
- Generate comparison reports

### 9.3 Benchmark Runner

**Location**: `tools/benchmarks/run_vllm_benchmarks.sh`

**Features**:
- Run all 6 benchmark shapes
- Collect APXM + vLLM metrics
- Generate HTML report with charts
- Archive results with timestamp

---

## 10. Key Takeaways

### What Makes vLLM Optimization Different

Unlike compiler optimizations (which reduce compute/memory at compile time), vLLM optimizations reduce **inference latency and token waste at runtime**. They are measured by:
- Cache hit rates (not fusion counts)
- TTFT distributions (not pass durations)
- Prefill token savings (not dead code elimination)

### The Golden Metric

**Prefix cache hit rate** is the single most important metric. A 2025 production study showed:
- 57× faster response times with cache-aware scheduling
- Double the throughput on identical hardware

For APXM, this means:
- Priority scheduling → better critical-path TTFT
- Prefix shaping → higher hit rates on fan-out
- Pinning → deterministic reuse on chains
- Warmup → amortized prefill cost

### Research Lessons from SGLang and Beyond

1. **RadixAttention** (SGLang): 50-99% cache hit rates translate to 5-6× throughput
2. **Helium**: Cache-aware batch scheduling with templated radix trees maximizes reuse
3. **Teola**: Graph-aware scheduling delivers 2.09× speedup
4. **Autellix**: Critical-path DAG optimization improves end-to-end latency

APXM is well-positioned to leverage these insights by providing graph metadata to vLLM.

---

## References

- vLLM Metrics Documentation: https://docs.vllm.ai/en/latest/design/metrics/
- vLLM Prefix Caching: https://docs.vllm.ai/en/latest/features/automatic_prefix_caching/
- SGLang Paper (RadixAttention): https://arxiv.org/pdf/2312.07104
- Helium (Cache-Aware Scheduling): arXiv:2603.16104
- Teola (End-to-End Optimization): https://arxiv.org/html/2407.00326v1
- Autellix (LLM Agents as Programs): https://arxiv.org/html/2502.13965v1
- Production Case Study: https://llm-d.ai/blog/kvcache-wins-you-can-see
- LLM Inference Monitoring Guide: https://www.glukhov.org/observability/monitoring-llm-inference-prometheus-grafana/

---

## Next Steps

1. **Validate baseline**: Run existing APXM graphs against vLLM with metrics enabled
2. **Implement metadata pass-through**: Land Workstream A (priority + vllm_xargs)
3. **Build correlation tooling**: Script to join APXM sessions with Prometheus metrics
4. **Create benchmark suite**: Implement 6 benchmark shapes in `examples/benchmarks/vllm/`
5. **Establish exit criteria dashboard**: Grafana panels showing go/no-go thresholds

**Owner**: TBD
**Timeline**: 9 weeks (baseline through decision point)
**Dependencies**: vLLM deployment, Prometheus/Grafana setup, APXM backend enhancements
