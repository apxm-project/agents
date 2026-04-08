# Caching and Memoization

APXM's MemoCache optimization provides runtime caching of identical LLM requests within a single execution. This guide covers how caching works, when it applies, and how to control it.

---

## What is MemoCache?

MemoCache is a compiler optimization pass (enabled at `-O2` and `-O3`) that caches LLM responses **within a single workflow execution**.

**Key characteristics:**
- **Scope:** Single execution (not persistent across runs)
- **Hit rate:** 30-50% typical (depends on workflow structure)
- **Latency:** <5ms cache hit vs ~500ms LLM call (100x speedup)
- **Cache key:** Prompt template + interpolated values + model + backend

**Example:**
```python
# Same prompt executed twice in one workflow
r1 = g.ask("What is the capital of France?")
r2 = g.ask("What is the capital of France?")  # Cache hit
```

**Result:**
- First call: LLM request (~500ms)
- Second call: Cache hit (<5ms)
- Total: ~505ms instead of ~1000ms

---

## How Caching Works

### Cache Key Generation

MemoCache computes a hash from:
1. **Prompt template** (after interpolation)
2. **Model name** (if specified)
3. **Backend name** (if specified)
4. **Operation type** (ASK, THINK, REASON)

**Example cache keys:**
```python
# Different cache keys (different prompts)
r1 = g.ask("Summarize: AI")        # Key: hash("Summarize: AI", default_model, default_backend)
r2 = g.ask("Summarize: ML")        # Key: hash("Summarize: ML", default_model, default_backend)

# Same cache key (identical prompts)
r3 = g.ask("What is 2+2?")         # Key: hash("What is 2+2?", ...)
r4 = g.ask("What is 2+2?")         # Same key → cache hit
```

### Cache Lifecycle

1. **Execution starts** → Cache initialized (empty)
2. **First LLM call** → Miss, call backend, store result
3. **Subsequent identical calls** → Hit, return cached result
4. **Execution ends** → Cache cleared

**Important:** Cache is **not persistent** across executions. Each `dekk apxm execute` starts with an empty cache.

---

## Viewing Cache Statistics

Use `--emit-metrics` to see cache performance:

```bash
dekk apxm execute workflow.air --emit-metrics metrics.json
```

**Output:**
```json
{
  "memo_cache": {
    "hits": 12,
    "misses": 8,
    "hit_rate": 0.60,
    "total_requests": 20,
    "time_saved_ms": 6420,
    "avg_hit_latency_ms": 4.2,
    "avg_miss_latency_ms": 537.3
  },
  "total_llm_calls": 8,
  "execution_time_ms": 4312
}
```

**Interpretation:**
- **Hit rate:** 60% (12 hits, 8 misses)
- **Time saved:** 6.4 seconds (12 avoided LLM calls)
- **Speedup:** 1.5x overall (4.3s vs ~10.7s without cache)

---

## CLI Commands

### View Cache Stats

```bash
dekk apxm cache stats
```

**Output:**
```
APXM Cache Statistics
─────────────────────────────────────

Last Execution:
  Total requests: 24
  Cache hits: 15 (62.5%)
  Cache misses: 9 (37.5%)
  Time saved: 7.8s

Cache Configuration:
  Scope: per-execution
  TTL: N/A (cleared after execution)
  Max size: unlimited
  Enabled: true (via -O2)
```

### Clear Cache

For persistent caches (future feature), clear with:

```bash
dekk apxm cache clear
```

**Note:** Current implementation (v1.0) uses per-execution caching only. This command is reserved for future persistent cache support.

---

## Per-Operation Cache Control

### Disable Caching for Specific Nodes

Use `nocache=True` to prevent caching for non-deterministic operations:

```python
from apxm import compile, GraphRecorder
import time


@compile()
def workflow(g: GraphRecorder):
    # Deterministic — will be cached
    fact = g.ask("What is the capital of France?")

    # Non-deterministic — should NOT be cached
    timestamp = g.ask("What is the current timestamp?", nocache=True)
    random_uuid = g.ask("Generate a random UUID", nocache=True)

    # User-specific data — should NOT be cached
    user_data = g.ask("Fetch user profile for {user_id}", nocache=True)

    g.done(fact)
```

**When to use `nocache=True`:**
- Timestamp/date operations
- Random number generation
- Session-specific data
- User PII lookups
- Testing/debugging (force fresh calls)

### Disable Caching Globally

```bash
# Compile without MemoCache
dekk apxm compile workflow.air -O2 --no-memo-cache

# Execute without caching (same as -O0)
dekk apxm execute workflow.air -O0
```

---

## Cache Hit Rate Patterns

### High Hit Rate (60-80%)

**Workflow characteristics:**
- Repeated prompts with same inputs
- Shared context across parallel branches
- Validation/verification steps

**Example:**
```python
# 3 prompts executed twice (6 total, 3 cache hits)
prompts = ["Summarize: AI", "Summarize: ML", "Summarize: NLP"]
for prompt in prompts:
    first = g.ask(prompt)   # Cache miss
    second = g.ask(prompt)  # Cache hit (3 hits, 50% rate)
```

### Low Hit Rate (10-30%)

**Workflow characteristics:**
- Unique prompts per node
- Highly parameterized inputs
- Dynamic/user-specific data

**Example:**
```python
# Each prompt is unique
for i in range(10):
    g.ask(f"Analyze document {i}")  # 10 misses, 0 hits
```

### Zero Hit Rate (0%)

**Workflow characteristics:**
- `-O0` compilation (caching disabled)
- All nodes marked `nocache=True`
- Single-pass linear workflows (no repeats)

---

## Benchmark Results

From `memo_cache_stress.py` benchmark:

| Metric | -O0 (no cache) | -O2 (MemoCache) | Improvement |
|--------|----------------|-----------------|-------------|
| Total LLM calls | 6 | 3 | **50% reduction** |
| Execution time | 3.2s | 1.6s | **2x faster** |
| Cache hits | 0 | 3 | 50% hit rate |
| Avg hit latency | N/A | 4.8ms | 100x faster than LLM |

**Workflow:** 3 prompts executed twice (6 total requests)

---

## Advanced: Per-Op TTL (Future Feature)

Planned for v1.1: per-operation cache TTL for longer-lived caches:

```python
# Cache for 5 minutes (proposed API)
weather = g.ask("Current weather in SF", cache_ttl=300)

# Cache indefinitely until invalidated (proposed API)
constants = g.ask("What is the speed of light?", cache_ttl=-1)
```

**Use cases:**
- API rate limiting
- Expensive external tool calls
- Cross-execution optimization

---

## Caching vs Optimization Passes

MemoCache complements other optimization passes:

| Pass | Scope | When Applied | Savings |
|------|-------|--------------|---------|
| **CSE** | Compile-time | Identical nodes in graph | 100% (node eliminated) |
| **MemoCache** | Runtime | Identical prompts in execution | ~100% (cached response) |
| **FuseReasoning** | Compile-time | Sequential reasoning nodes | 40-60% (fewer calls) |

**Example interaction:**
```python
# CSE eliminates duplicate nodes at compile-time
r1 = g.ask("Summarize: AI")
r2 = g.ask("Summarize: AI")  # CSE merges r2 → r1 (1 node)

# MemoCache handles runtime duplicates (dynamic inputs)
for topic in topics:  # Runtime loop, CSE can't help
    g.ask(f"Summarize: {topic}")  # MemoCache deduplicates identical topics
```

---

## Best Practices

1. **Enable MemoCache by default** (use `-O2`)
2. **Mark non-deterministic ops** with `nocache=True`
3. **Benchmark cache impact** with `--emit-metrics`
4. **Aim for 40%+ hit rate** — if lower, consider workflow refactoring
5. **Don't rely on caching for correctness** — it's a performance optimization, not a semantic feature

---

## Troubleshooting

### Cache Not Working

**Symptoms:** 0% hit rate despite repeated prompts

**Checks:**
1. Confirm `-O2` or `-O3` (not `-O0`)
2. Check nodes aren't marked `nocache=True`
3. Verify prompts are **exactly identical** (interpolation values must match)
4. Inspect metrics: `--emit-metrics metrics.json`

**Example:**
```python
# Won't cache (different values)
r1 = g.ask("Summarize: AI")
r2 = g.ask("Summarize: ML")  # Different prompt → miss

# Will cache (identical after interpolation)
topic = "AI"
r3 = g.ask(f"Summarize: {topic}")
r4 = g.ask(f"Summarize: {topic}")  # Same prompt → hit
```

### Unexpected Cache Hits

**Symptoms:** Outputs seem cached when they shouldn't be

**Cause:** Prompts resolve to same value after interpolation

**Example:**
```python
# Both resolve to "Summarize: AI" → cache hit
topic1 = "AI"
topic2 = "AI"  # Same value
r1 = g.ask(f"Summarize: {topic1}")
r2 = g.ask(f"Summarize: {topic2}")  # Hit (not miss)
```

**Fix:** Add unique identifiers to prompts or use `nocache=True`

---

## Next Steps

- [Optimization Guide](optimization.md) — Full optimization pass details
- [Multi-Model Routing](multi-model.md) — Per-node model selection
- [Benchmarking](../implementation/benchmarking.md) — Measuring cache impact
