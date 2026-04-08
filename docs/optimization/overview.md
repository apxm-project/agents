---
title: "Optimization Guide"
description: "How APXM's compiler optimization levels, targets, and per-pass controls reduce latency, cost, and token usage."
---

# Optimization Guide

The APXM compiler applies MLIR-based optimization passes to agent workflows before execution. Choosing the right optimization level and target can cut latency by 2-3x and reduce LLM costs by over 50%. This page covers the knobs available; for the full pass catalog see [Optimization Passes](passes.md).

## Optimization Levels

APXM supports four optimization levels via the `-O` flag.

### `-O0` -- No Optimization

Emits the graph unchanged. Fastest compilation, no transformations applied.

**Use when:** debugging workflows (all nodes preserved), testing new graph features, or inspecting intermediate results.

```bash
dekk apxm compile graph.apxm -O0 -o debug.apxmobj
dekk apxm execute graph.apxm -O0
```

### `-O1` -- Lightweight Optimization

Applies normalization, prompt building, scheduling, fusion, and standard MLIR cleanup (canonicalization, CSE, dead-symbol elimination). Good balance for iterative development.

### `-O2` -- Standard Optimization (default)

All O1 passes plus deeper analysis: template specialization, schema narrowing, dead-context elimination, operation condensing, and graph-level prompt-caching and memoization hints. This is the recommended level for production.

**Enabled passes (summary):**

| Category | Passes |
|----------|--------|
| AIS-specific | normalize, build-prompt, fuse-ask-ops, condense-ops, scheduling |
| Analysis | unconsumed-value-warning |
| Standard MLIR | canonicalizer, cse, symbol-dce |
| O2 extras | template-specialization, schema-narrowing, dead-context-elimination |
| Graph-level (pre-MLIR) | prompt_caching, memoization_hints |

See [Optimization Passes](passes.md) for what each pass does and how they compose at every level.

```bash
dekk apxm compile graph.apxm -O2 -o production.apxmobj
dekk apxm execute graph.apxm          # -O2 is the default
```

### `-O3` -- Aggressive Optimization (experimental)

Runs the O2 pass set inside a convergence loop (up to 10 iterations), re-applying fusion, elimination, and canonicalization until the IR stabilizes. Longer compile times, but can squeeze out additional reductions on complex graphs.

```bash
dekk apxm compile graph.apxm -O3 -o aggressive.apxmobj
```

---

## Optimization Targets

The `--target` flag steers the optimizer toward a specific objective.

### `--target latency` (default)

Minimize end-to-end wall time. Prioritizes parallelism and aggressive node fusion; may increase token usage due to more parallel calls.

```bash
dekk apxm compile graph.apxm -O2 --target latency
```

For integration-level latency tuning with self-hosted models, see [vLLM Integration](../integrations/vllm.md) and [Latency Research](../research/optimization/latency.md).

### `--target cost`

Minimize LLM API spend. Maximizes request batching, prefers sequential execution for fusion opportunities, reduces redundant calls. Trade-off: higher latency.

```bash
dekk apxm compile graph.apxm -O2 --target cost
```

### `--target tokens`

Minimize total token consumption. Maximizes prompt sharing and aggressive context pruning. Trade-off: may increase latency or reduce output quality from aggressive pruning.

```bash
dekk apxm compile graph.apxm -O2 --target tokens
```

For token-estimation research and measurement, see [Token Research](../research/optimization/tokens.md) and [Quality Research](../research/optimization/quality.md).

---

## Key Pass Highlights

The full catalog lives in [Optimization Passes](passes.md). Below are the highest-impact passes with brief examples.

### FuseAskOps

Merges sequential ASK chains where one output feeds the next prompt into a single combined LLM call, eliminating API round-trips.

**Before (3 LLM calls):**
```
ASK("Extract key concepts: {topic}")  ->
ASK("Analyze concepts: {concepts}")   ->
ASK("Synthesize: {analysis}")
```

**After (1 LLM call):** A single fused prompt combining all three steps.

The pass is conservative -- it will not fuse across `TRY_CATCH` boundaries, `FENCE` barriers, or when an intermediate result has multiple consumers.

### Common Subexpression Elimination (CSE)

Deduplicates operations with identical opcode and inputs. For LLM operations, same-prompt + same-context are treated as equivalent (sound at temperature 0). Disable with `--no-cse-llm` for non-deterministic workflows.

### Dead-Context Elimination

Removes context values threaded through the graph but never read, pruning unnecessary LLM calls and intermediate data.

### Memoization Hints

Annotates deterministic, side-effect-free operations for cross-run result caching. At runtime, cache hits skip re-execution entirely. For cache architecture and configuration, see the [Caching Guide](../guides/caching.md).

---

## Benchmark Results

Detailed benchmark data -- including per-pass speedups, node reduction counts, and end-to-end latency comparisons across O0/O2 on mock and real backends -- is maintained in the [April 2026 Benchmark Results](../benchmarks/results/2026-04-08.md).

---

## Disabling Specific Passes

Use `--no-cse-llm` to skip CSE for LLM operations (useful for non-deterministic workflows):

```bash
dekk apxm compile graph.apxm -O2 --no-cse-llm
```

For broader pass control, drop to `-O0` and rely on the runtime's own caching layer, or use `-O1` to get basic fusion without the deeper O2 passes.

---

## Per-Node Optimization Control

Disable caching for specific nodes via the `nocache` attribute:

```json
{"id": 5, "op": "ASK", "attributes": {"prompt": "Generate random UUID", "nocache": true}}
```

Use this for non-deterministic operations (random values, timestamps), user-specific data, or testing.

---

## Viewing Optimization Diagnostics

Emit per-pass statistics with `--emit-diagnostics`:

```bash
dekk apxm compile graph.apxm -O2 --emit-diagnostics diagnostics.json
```

The output includes pass names, durations, ops before/after each pass, and ops delta. For the measurement infrastructure behind these metrics, see [Metrics Infrastructure](../benchmarks/methodology/metrics-infrastructure.md).

---

## Best Practices

1. **Default to `-O2`** for production workflows.
2. **Use `-O0` for debugging** to preserve all intermediate nodes.
3. **Target latency** for user-facing workflows (responsiveness matters).
4. **Target cost** for batch processing (cost efficiency matters).
5. **Benchmark before tuning** -- use `--emit-metrics` to measure actual impact.
6. **Check diagnostics** to understand which passes fired and what they eliminated.

---

## Further Reading

- [Optimization Passes](passes.md) -- full pass catalog with per-level composition tables
- [Caching Guide](../guides/caching.md) -- MemoCache architecture (L1 DashMap + L2 SQLite)
- [Multi-Model Routing](../guides/multi-model.md) -- per-node model selection for cost/quality trade-offs
- [vLLM Integration](../integrations/vllm.md) -- optimizing self-hosted inference
- [DSPy Integration](../integrations/dspy.md) -- optimization via DSPy prompt compilation
- [Compiler Pipeline](../implementation/compiler/overview.md) -- four-stage compilation architecture
- [Benchmark Results](../benchmarks/results/2026-04-08.md) -- latest O0 vs O2 measurements
- [Latency Research](../research/optimization/latency.md), [Token Research](../research/optimization/tokens.md), [Quality Research](../research/optimization/quality.md) -- per-target deep dives
