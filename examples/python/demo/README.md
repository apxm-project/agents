# APXM Showcase Demo

This directory contains `showcase.py` — a comprehensive demonstration of ALL APXM optimization features in a single workflow.

## What It Demonstrates

### 1. **Auto-Wiring**
Nodes reference each other by name using template variables `{node_name}`. The compiler automatically resolves dependencies:

```python
triage = g.ask("triage", "...")
arch = g.think("architecture_analysis", "Based on triage: {triage}...")
```

No manual edge creation needed — APXM infers data flow from variable references.

### 2. **Typed Agent Profiles**
Import pre-generated agent profiles with full type safety:

```python
from apxm._generated.agents import claude
coder = g.spawn("coder", profile=claude, cwd=cwd)
```

Profiles include: command, timeouts, permission modes, default models, capabilities.

### 3. **Per-Node Model Routing**
Different nodes can use different models based on task requirements:

```python
# Fast/cheap model for triage
triage = g.ask("triage", "...", model="gpt-4o-mini")

# Powerful model for deep analysis
arch = g.think("architecture_analysis", "...", model="claude-sonnet-4")
```

Optimizes cost/quality trade-offs across the workflow.

### 4. **Parallel Execution (Fan-Out)**
Independent analyses run concurrently:

```python
arch = g.think("architecture_analysis", "...")
security = g.think("security_analysis", "...")
performance = g.think("performance_analysis", "...")
# All three execute in parallel — no data dependencies between them
```

Expected speedup: ~3x vs sequential execution.

### 5. **Agent Spawning (ACP Protocol)**
Spawn external AI agents as subprocesses:

```python
coder = g.spawn("coder", profile=claude, cwd=cwd)
agent_result = g.communicate(target_agent="coder", message="{task}")
```

Demonstrates multi-agent coordination via ACP (Agent Client Protocol).

### 6. **Memory Operations (AAM Integration)**
Store and recall information across workflow execution:

```python
# Store (UMEM)
g.update_memory("store_analysis", data=arch, key="last_architecture_analysis")

# Recall (QMEM)
recalled = g.query_memory("recall_history", query="previous analyses...")
```

Integrates with AAM (Agent Abstract Machine) Beliefs layer.

### 7. **Compile-Time Optimization Potential**

The workflow contains multiple optimization opportunities that APXM's MLIR-based compiler can exploit:

#### **FuseReasoning Pass**
Adjacent THINK nodes can be fused into a single LLM call:

```python
review = g.think("review", "...")
validation = g.think("validation", "...")
# Compiler can fuse these into one call with combined prompt
```

**Benefit:** Reduced API overhead, faster execution.

#### **SharedPrefixReuse Pass**
Parallel nodes share common prefix:

```python
arch = g.think("...", "Based on triage: {triage}...")
security = g.think("...", "Based on triage: {triage}...")
performance = g.think("...", "Based on triage: {triage}...")
```

**Benefit:** With KV cache, reuse prefix computation across all three. Lower token cost.

#### **DeadContextElimination Pass**
If downstream nodes don't use certain values, prune them:

```python
final_report = g.merge("final_report", synthesis, review, validation)
# If final_report only uses synthesis, review/validation become dead context
```

**Benefit:** Avoid computing unused values.

#### **PriorityScheduling Pass**
Critical path optimization:

```
triage → arch/security/performance → synthesis → review → final_report
```

Scheduler prioritizes critical path nodes to minimize total latency.

## Usage

### View MLIR Output

```bash
python3 -m examples.python.demo.showcase
```

Outputs the compiled `.air` MLIR along with optimization analysis.

### Compile at Different Optimization Levels

```bash
# O0: No optimizations (baseline)
dekk apxm compile showcase.apxm -O0 -o showcase_O0.apxmobj

# O2: All optimizations enabled
dekk apxm compile showcase.apxm -O2 -o showcase_O2.apxmobj
```

Compare artifact sizes and performance.

### Analyze Parallelism

```bash
dekk apxm analyze showcase.apxm
```

Shows:
- Parallelism potential (nodes that can run concurrently)
- Critical path
- Expected speedup

### Execute with Full Tracing

```bash
dekk apxm execute showcase.apxm --emit-session --emit-metrics metrics.json
```

Creates session directory with:
- `trace.ndjson` — live execution trace
- `metrics.json` — performance metrics
- `results.json` — all node outputs
- `nodes/<id>_<name>/` — per-node workspaces

### Replay Session Timeline

```bash
dekk apxm replay ~/.apxm/sessions/<execution-id>
```

Renders execution timeline from trace events.

## Optimization Impact Comparison

| Metric | O0 (Baseline) | O2 (Optimized) | Improvement |
|--------|---------------|----------------|-------------|
| **Total Latency** | ~45s | ~18s | **2.5x faster** |
| **Parallel Nodes** | Sequential | 3 concurrent | **3x speedup** |
| **LLM Calls** | 10 calls | 8 calls (fused) | **20% fewer** |
| **Tokens Processed** | 8,500 | 6,200 (prefix reuse) | **27% reduction** |
| **Memory Overhead** | Full context | Pruned dead values | **~40% less** |

*Note: Numbers are illustrative based on typical optimization gains.*

## Key Takeaways

1. **APXM is a compiler** — it transforms high-level workflows into optimized execution plans
2. **MLIR foundation** — leverages proven compiler infrastructure (same as TensorFlow, PyTorch)
3. **Optimization passes** — FuseReasoning, SharedPrefixReuse, DeadContextElimination, PriorityScheduling
4. **Multi-agent coordination** — ACP protocol for spawning/communicating with external agents
5. **Memory-augmented** — AAM (Agent Abstract Machine) provides stateful Beliefs/Goals/Capabilities
6. **Cost/quality trade-offs** — per-node model routing optimizes total cost vs quality

## Related Examples

- `examples/python/patterns/plan-fan-out/` — fan-out pattern
- `examples/python/patterns/memory-rag/` — memory operations
- `examples/python/acp-agents/architect_implement_review.py` — multi-agent workflow
- `examples/python/benchmarks/multi_model.py` — per-node model routing
- `examples/python/benchmarks/fusion_stress.py` — fusion optimization benchmark

## Further Reading

- `docs/implementation/compiler/optimization-passes.md` — detailed pass documentation
- `docs/pxm/ais.md` — AIS (Agent Instruction Set) specification
- `docs/guides/first-graph.md` — getting started guide
- `docs/implementation/aam.md` — Agent Abstract Machine architecture
