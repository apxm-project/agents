# Caching and Memoization

APXM's MemoCache optimization caches identical LLM responses within a single execution, avoiding redundant calls and reducing latency.

---

## What is MemoCache?

MemoCache is a compiler optimization pass (enabled at `-O2` and above) that deduplicates LLM requests **within a single workflow execution**.

Key characteristics:
- **Scope:** Single execution (not persistent across runs)
- **Typical hit rate:** 30-50% (depends on workflow structure)
- **Latency:** <5ms cache hit vs ~500ms LLM call (100x speedup)
- **Cache key:** Prompt template + interpolated values + model + backend + operation type

```python
# Same prompt executed twice in one workflow
r1 = g.ask("What is the capital of France?")
r2 = g.ask("What is the capital of France?")  # Cache hit
```

First call: LLM request (~500ms). Second call: cache hit (<5ms). Total: ~505ms instead of ~1000ms.

---

## How Caching Works

### Cache Key Generation

MemoCache computes a hash from:
1. **Prompt template** (after interpolation)
2. **Model name** (if specified)
3. **Backend name** (if specified)
4. **Operation type** (ASK, THINK, REASON)

```python
# Different cache keys (different prompts)
r1 = g.ask("Summarize: AI")        # Key: hash("Summarize: AI", default_model, default_backend)
r2 = g.ask("Summarize: ML")        # Key: hash("Summarize: ML", default_model, default_backend)

# Same cache key (identical prompts)
r3 = g.ask("What is 2+2?")         # Key: hash("What is 2+2?", ...)
r4 = g.ask("What is 2+2?")         # Same key -> cache hit
```

### Cache Lifecycle

1. **Execution starts** -- cache initialized (empty).
2. **First LLM call** -- miss; call backend, store result.
3. **Subsequent identical calls** -- hit; return cached result.
4. **Execution ends** -- cache cleared.

Cache is **not persistent** across executions. Each `apxm execute` starts with an empty cache.

---

## Viewing Cache Statistics

Use `--emit-metrics` to inspect cache performance:

```bash
apxm execute workflow.apxm --emit-metrics metrics.json
```

Example output:
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

Interpretation: 60% hit rate (12 hits / 20 total), 6.4 seconds saved, 1.5x overall speedup.

---

## Per-Operation Cache Control

### Disable Caching for Specific Nodes

Use `nocache=True` for non-deterministic operations:

```python
from apxm import compile, GraphRecorder


@compile()
def workflow(g: GraphRecorder):
    # Deterministic -- will be cached
    fact = g.ask("What is the capital of France?")

    # Non-deterministic -- should NOT be cached
    timestamp = g.ask("What is the current timestamp?", nocache=True)
    random_uuid = g.ask("Generate a random UUID", nocache=True)

    g.done(fact)
```

Use `nocache=True` for: timestamp/date operations, random generation, session-specific data, user PII lookups, and testing/debugging (force fresh calls).

### Disable Caching Globally

```bash
# Compile without MemoCache
apxm compile workflow.apxm -O2 --no-memo-cache

# Execute without caching (same as -O0)
apxm execute workflow.apxm -O0
```

---

## Cache Hit Rate Patterns

### High Hit Rate (60-80%)

Workflows with repeated prompts, shared context across parallel branches, or validation/verification steps that re-ask the same questions.

### Low Hit Rate (10-30%)

Workflows with unique prompts per node, highly parameterized inputs, or dynamic user-specific data.

### Zero Hit Rate (0%)

Occurs when: `-O0` disables caching, all nodes use `nocache=True`, or the workflow is a single-pass linear pipeline with no repeated prompts.

---

## Caching vs Optimization Passes

MemoCache complements other optimization passes:

| Pass | Scope | When Applied | Savings |
|------|-------|--------------|---------|
| **CSE** | Compile-time | Identical nodes in graph | 100% (node eliminated) |
| **MemoCache** | Runtime | Identical prompts during execution | ~100% (cached response) |
| **FuseReasoning** | Compile-time | Sequential reasoning nodes | 40-60% (fewer calls) |

CSE eliminates structurally identical nodes at compile time. MemoCache handles runtime duplicates that CSE cannot detect (e.g., dynamically parameterized prompts that happen to resolve to the same value).

See [Optimization Passes](../optimization/passes.md) for details on CSE and FuseReasoning.

---

## Best Practices

1. **Enable MemoCache by default** -- use `-O2`.
2. **Mark non-deterministic ops** with `nocache=True`.
3. **Benchmark cache impact** with `--emit-metrics`.
4. **Aim for 40%+ hit rate** -- if lower, consider restructuring the workflow to share prompts.
5. **Do not rely on caching for correctness** -- it is a performance optimization, not a semantic guarantee.

---

## Troubleshooting

### Cache Not Working

**Symptoms:** 0% hit rate despite repeated prompts.

**Checks:**
1. Confirm `-O2` or higher (not `-O0`).
2. Verify nodes are not marked `nocache=True`.
3. Verify prompts are **exactly identical** after interpolation.
4. Inspect metrics: `--emit-metrics metrics.json`.

### Unexpected Cache Hits

**Symptoms:** Outputs seem cached when they should differ.

**Cause:** Prompts resolve to the same value after interpolation.

**Fix:** Add unique identifiers to prompts, or use `nocache=True`.

---

## See Also

- [Optimization Overview](../optimization/overview.md) -- Optimization levels and targets
- [Optimization Passes](../optimization/passes.md) -- CSE, FuseReasoning, MemoCache details
- [Multi-Model Routing](multi-model.md) -- Per-node model selection affects cache keys
- [Debugging](debugging.md) -- Tracing cache behavior at runtime
- [Configuration Reference](../reference/config.md) -- Global settings that affect caching
