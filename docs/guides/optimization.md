# Optimization Levels and Targets

APXM's compiler applies MLIR-based optimizations to improve workflow performance. This guide covers optimization levels, targets, and trade-offs.

---

## Optimization Levels

APXM supports three optimization levels via the `-O` flag:

### `-O0` — No Optimization

Fastest compilation, no transformations:
- No operation fusion
- No dead code elimination
- No common subexpression elimination
- No memoization cache

**Use when:**
- Debugging workflows (preserves all nodes)
- Testing new features
- Inspecting intermediate results

**Example:**
```bash
dekk apxm compile graph.air -O0 -o debug.apxmobj
dekk apxm execute graph.air -O0
```

### `-O2` — Standard Optimization (default)

Balanced compilation time and runtime performance:

**Enabled passes:**
- **FuseReasoning** — Merge sequential ASK/THINK/REASON nodes into single LLM calls
- **DeadContextElimination** — Remove unused CONST_STR nodes and redundant operations
- **CommonSubexpressionElimination** — Deduplicate identical prompts
- **MemoCache** — Cache identical LLM requests within a single execution
- **PrefixSharing** — Share common prompt prefixes across parallel requests

**Use when:**
- Production workflows
- Minimizing LLM costs
- Reducing latency

**Example:**
```bash
dekk apxm compile graph.air -O2 -o production.apxmobj
dekk apxm execute graph.air  # -O2 is default
```

### `-O3` — Aggressive Optimization (experimental)

Maximum optimization, longer compilation:
- All -O2 passes
- More aggressive fusion heuristics
- Cross-subgraph optimizations
- Speculative execution

**Use when:**
- Performance-critical workflows
- Willing to trade compilation time for runtime speed
- Benchmarking peak performance

**Example:**
```bash
dekk apxm compile graph.air -O3 -o aggressive.apxmobj
```

---

## Optimization Targets

The `--target` flag optimizes for specific metrics:

### `--target latency` (default)

Minimize end-to-end execution time:
- Prioritize parallelism
- Aggressive node fusion
- Minimize sequential dependencies

**Example:**
```bash
dekk apxm compile graph.air -O2 --target latency
```

**Trade-offs:**
- May increase token usage (more parallel calls)
- Higher cost (less batching)

### `--target cost`

Minimize LLM API costs:
- Maximize request batching
- Prefer sequential execution for fusion opportunities
- Reduce redundant calls

**Example:**
```bash
dekk apxm compile graph.air -O2 --target cost
```

**Trade-offs:**
- Higher latency (more sequential execution)
- Fewer parallel calls

### `--target tokens`

Minimize total token consumption:
- Maximize prompt sharing
- Aggressive context pruning
- Prefer smaller intermediate results

**Example:**
```bash
dekk apxm compile graph.air -O2 --target tokens
```

**Trade-offs:**
- May increase latency
- May reduce output quality (aggressive pruning)

---

## Optimization Pass Details

### FuseReasoning

Merges sequential reasoning nodes into a single LLM call:

**Before:**
```python
concepts = g.ask("Extract key concepts: {topic}")
analysis = g.think("Analyze concepts: {concepts}")
conclusion = g.reason("Synthesize: {analysis}")
concepts >> analysis >> conclusion
```

**After optimization:**
Single LLM call with combined prompt:
```
Extract key concepts: {topic}
Then analyze those concepts.
Finally, synthesize a conclusion.
```

**Speedup:** 3x (1 LLM call vs 3)
**Cost savings:** ~60% (shared context, single request)

### CommonSubexpressionElimination (CSE)

Deduplicates identical prompts:

**Before:**
```python
r1 = g.ask("Summarize: {text}")
r2 = g.ask("Summarize: {text}")  # Identical prompt
```

**After optimization:**
Both `r1` and `r2` reference the same node.

**Speedup:** 2x for duplicate requests
**Cost savings:** 50% per duplicate

### DeadContextElimination

Removes unused intermediate results:

**Before:**
```python
unused = g.ask("Complex analysis...")  # Never referenced
result = g.ask("Final answer: {input}")
```

**After optimization:**
The `unused` node is removed entirely.

**Speedup:** Avoids unnecessary LLM calls
**Cost savings:** 100% for eliminated nodes

### MemoCache

Runtime caching of identical requests within a single execution:

**Example:**
```python
# Same prompt executed twice
r1 = g.ask("What is 2+2?")
r2 = g.ask("What is 2+2?")  # Cache hit at runtime
```

**Speedup:** <5ms cache hit vs ~500ms LLM call (100x)
**Cache hit rate:** 30-50% typical (depends on workflow)

See [Caching Guide](caching.md) for details.

---

## Benchmark Results

From Week 1 benchmarks (20 workflows, 100 executions each):

| Metric | -O0 | -O2 | Improvement |
|--------|-----|-----|-------------|
| Avg latency | 8.2s | 3.1s | **2.6x faster** |
| Total LLM calls | 6,420 | 2,830 | **56% reduction** |
| Total tokens | 4.2M | 1.8M | **57% savings** |
| Compilation time | 120ms | 380ms | 3.2x slower |

**Key findings:**
- FuseReasoning: 40-60% latency reduction
- CSE: 10-20% token savings
- MemoCache: 30-50% cache hit rate
- Compilation overhead: <400ms (negligible for production)

---

## Disabling Specific Passes

Use `--no-<pass>` to disable individual optimizations:

```bash
# Disable fusion (preserve node boundaries for debugging)
dekk apxm compile graph.air -O2 --no-fuse

# Disable caching (deterministic outputs)
dekk apxm compile graph.air -O2 --no-memo-cache

# Disable CSE (preserve all node instances)
dekk apxm compile graph.air -O2 --no-cse
```

---

## Per-Node Optimization Control

Disable optimization for specific nodes via `nocache` attribute:

```python
# This node's output will never be cached
result = g.ask("Generate random UUID", nocache=True)
```

**Use cases:**
- Non-deterministic operations (random, timestamp)
- User-specific data (PII, session state)
- Testing/debugging

---

## Viewing Optimization Diagnostics

Emit detailed optimization statistics:

```bash
dekk apxm compile graph.air -O2 --emit-diagnostics diagnostics.json
```

**Output:**
```json
{
  "passes_applied": ["FuseReasoning", "CSE", "DeadContextElimination"],
  "fused_nodes": [{"from": [3, 4, 5], "to": 3, "type": "sequential"}],
  "eliminated_nodes": [8, 12],
  "cse_merges": [{"original": [6, 9], "merged_to": 6}],
  "compilation_time_ms": 342,
  "estimated_speedup": 2.3
}
```

---

## Best Practices

1. **Default to -O2** for production workflows
2. **Use -O0 for debugging** to preserve all intermediate nodes
3. **Target latency** for user-facing workflows (responsiveness)
4. **Target cost** for batch processing (cost efficiency)
5. **Benchmark before optimizing** — use `--emit-metrics` to measure impact
6. **Check diagnostics** to understand what optimizations were applied

---

## Next Steps

- [Caching Guide](caching.md) — Deep dive on MemoCache
- [Multi-Model Routing](multi-model.md) — Per-node model selection for cost/quality trade-offs
- [Benchmarking](../implementation/benchmarking.md) — Measuring workflow performance
