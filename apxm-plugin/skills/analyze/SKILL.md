---
name: analyze
description: Analyze graph parallelism, critical path, and estimated speedup
user-invocable: true
---

# Analyze

Performs dataflow analysis on a graph JSON to reveal its parallelism structure. Computes execution phases (groups of nodes that can run concurrently), identifies the critical path (longest dependency chain that bounds total execution time), and estimates the speedup from parallel execution versus sequential.

This is the "profiler" for graph design — it tells you how well your workflow exploits parallelism before you run it.

## Commands

```bash
dekk apxm analyze graph.air           # human-readable parallelism report
dekk apxm analyze graph.air --json    # machine-readable JSON output
```

## Output

**Human-readable** shows:
- Graph summary: node count, edge count, phase count, max parallelism
- Phase-by-phase breakdown with parallelism degree and estimated latency per phase
- Critical path: the sequence of nodes forming the longest dependency chain
- Speedup estimate: sequential time vs parallel time with multiplier (e.g., "2.50x")
- Suggestions: optimization hints like "nodes X and Y could be parallelized"

**JSON output** includes:
```json
{
  "execution_phases": [
    {"phase": 1, "parallel": true, "parallelism_degree": 3, "estimated_ms": 5000,
     "nodes": [{"id": 2, "name": "search", "op": "INV", "latency_ms": 5000}]}
  ],
  "critical_path": {"nodes": [1, 2, 5], "length": 3, "estimated_ms": 15000},
  "speedup": {"sequential_ms": 35000, "parallel_ms": 15000, "estimated_speedup": "2.33x"},
  "suggestions": ["Phase 2 has 3 independent nodes — good parallelism"]
}
```

## How It Works

Analysis uses BFS layering over the DAG. Nodes are grouped into phases based on data dependencies: nodes in the same phase have no edges between them and can execute concurrently. The critical path is computed as the longest weighted path through the phase graph, where weights are AIS operation latency estimates (e.g., ASK/THINK/REASON = slow, INV = medium, NOP/IDENTITY = fast).

## Related Commands

- `dekk apxm validate graph.air` — structural validation (run before analyze)
- `dekk apxm explain graph.air` — human-readable walkthrough of what the graph does
- `dekk apxm view graph.air` — interactive visual graph explorer

## When to Use

- After designing a workflow, to check how much parallelism you're getting
- When optimizing graph structure — the speedup estimate tells you if adding parallel branches helps
- To identify bottleneck nodes on the critical path
- In CI to track parallelism metrics across graph changes
