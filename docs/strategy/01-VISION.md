# Unified Agent Execution Vision

**Date**: March 31, 2026
**Scope**: APXM + vLLM + ACPX Integration Strategy

---

## The Vision

**One sentence**: An OpenClaw agent receives a request, automatically generates an APXM graph, compiles it, and executes it on a runtime that is deeply aware of model availability, GPU state, and inference scheduling -- closing the gap between orchestration and execution.

**The full picture**:

```
  User Request (natural language)
         |
         v
  +------------------+
  |   OpenClaw Agent  |   "Build me a code review pipeline"
  |   (Trigger Layer) |
  +--------+---------+
           |
           v
  +------------------+
  |   Graph Generator |   Understands AIS operations, agent capabilities,
  |   (ACPX -> APXM)  |   model availability, and hardware topology
  +--------+---------+
           |
           v
  +------------------+
  |   APXM Compiler   |   Validates, optimizes (fusion, parallelism,
  |   (MLIR Pipeline)  |   speculation, memoization, context routing)
  +--------+---------+
           |
           v
  +------------------+     +------------------+
  |   APXM Runtime    |<--->|   vLLM Backend    |   GPU-aware scheduling,
  |   (Dataflow Sched)|     |   (Model Serving)  |   KV-cache sharing,
  |                    |     |                    |   streaming prefill,
  |   32 AIS ops       |     |   Model registry   |   result pinning
  +------------------+     +------------------+
           |
           v
  +------------------+
  |   Results + Trace  |   Structured output, episodic memory,
  |   (Observable)     |   metrics, replay bundles
  +------------------+
```

---

## Why This Matters

### The Current Disconnection

Today, three systems operate in isolation:

| System | Knows | Doesn't Know |
|--------|-------|-------------|
| **ACPX** | Workflow structure, agent profiles, ACP protocol | GPU state, model capacity, optimization opportunities |
| **APXM** | Graph semantics, AIS operations, dataflow scheduling | Which models are available right now, GPU memory pressure |
| **vLLM** | Model serving, KV-cache, batching, GPU memory | That its results feed a pipeline, what comes next in the graph |

Each system makes decisions in a vacuum:
- ACPX picks a model **statically** in the flow definition (`profile: "claude"`)
- APXM compiles model assignments **at compile time** (`model: "gpt-4"` in node attributes)
- vLLM serves requests **in isolation**, unaware that the output will immediately feed another inference call

### The Opportunity

When these three systems are unified:

1. **The graph knows the hardware**: APXM's compiler can see which models are available, healthy, and underloaded -- and make intelligent routing decisions
2. **The hardware knows the graph**: vLLM can see that a result will feed another LLM call and keep tensors in GPU memory instead of streaming them back
3. **The orchestrator knows both**: ACPX-generated workflows get compiled into optimized execution plans that exploit parallelism, speculation, and caching

### Concrete Example

A PR review pipeline today:

```
fetch_diff -> review_code(claude) -> judge_verdict -> post_result
             [waits 30s for API]    [waits 5s]
```

With the unified system:

```
fetch_diff -> review_code(best_available_model) -> judge_verdict -> post_result
              |                                     |
              | Model selected at runtime:          | Speculative execution:
              | gpt-5.4 is at 100% health,          | Started before review
              | 15M TPM available, 8ms latency      | finishes, using cached
              | vs claude-opus-4 at 80% capacity     | template from prior runs
              |                                     |
              | KV-cache hint:                      | Memoization:
              | "next node will use my output"      | Same diff -> cached review
              | -> keep in GPU, don't stream        | (microsecond response)
```

---

## Five Pillars of the Unified System

### Pillar 1: Dynamic Model Routing (APXM <-> Model Registry)

**Today**: Model is hardcoded at graph authoring time.
**Tomorrow**: Model is resolved at execution time based on availability, cost, latency, and capability.

```
Node: ASK { template: "Review this code..." }
  |
  Runtime Model Router:
  |
  Priority List:
  1. gpt-5.4       -> Healthy, 15M TPM, 150K RPM -> SELECTED
  2. claude-opus-4  -> Healthy, but 80% capacity
  3. gpt-5.3-chat  -> Healthy, 8M TPM (fallback)
  4. gpt-5.4-mini  -> Healthy, budget option
  |
  Constraints:
  - Operation type ASK -> prefers fast models (latency: LOW)
  - Operation type THINK -> prefers reasoning models (latency: HIGH)
  - Operation type REASON -> prefers structured models (latency: MEDIUM)
```

### Pillar 2: Graph-Aware Inference (vLLM <-> APXM Runtime)

**Today**: vLLM treats each request independently.
**Tomorrow**: vLLM receives graph metadata with each request, enabling:

- **Result pinning**: Keep KV-cache warm when the next node will use this output
- **Streaming prefill**: Start prefilling the next node while current node is still generating
- **Batch coalescing**: Group related nodes from the same graph into a single batch
- **Priority scheduling**: Critical-path nodes get higher GPU priority

### Pillar 3: Automatic Graph Generation (ACPX -> APXM Graph)

**Today**: Developers write `.flow.ts` files manually.
**Tomorrow**: An OpenClaw agent takes a natural language request and generates an APXM graph.

```
User: "Set up a pipeline that reviews PRs, runs tests, and merges if clean"

Agent generates:
{
  "name": "auto-pr-pipeline",
  "nodes": [
    {"id": 1, "name": "fetch_pr", "op": "INV", "attributes": {"capability": "gh-pr-diff"}},
    {"id": 2, "name": "review", "op": "ASK", "attributes": {"template": "Review: {0}"}},
    {"id": 3, "name": "test", "op": "EXC", "attributes": {"command": "pytest"}},
    {"id": 4, "name": "decide", "op": "BRANCH_ON_VALUE", "attributes": {"key": "verdict"}},
    {"id": 5, "name": "merge", "op": "INV", "attributes": {"capability": "gh-pr-merge"}},
    {"id": 6, "name": "comment", "op": "INV", "attributes": {"capability": "gh-pr-comment"}}
  ],
  "edges": [...]
}
```

### Pillar 4: Spaghetti-Stack Context Management

From the patent (Internal): Context organized as a cactus stack with collapsed summaries and reference pointers. Five integrated techniques:

1. **Demand-paged context**: Only load relevant context per stage (34% of flat approach)
2. **Speculative execution**: Start downstream stages before upstream completes
3. **Token pipelining**: Stream tokens into next stage's prefill phase
4. **Memoization**: Hash-keyed caching for repeated inputs (microsecond returns)
5. **Stage-specific loading**: Each node sees only what it needs

### Pillar 5: Compilation-Driven Optimization

APXM's MLIR pipeline already does:
- Dead code elimination
- Constant folding
- Operation fusion (FuseReasoning)

With the unified system, new passes:
- **Model affinity analysis**: Which nodes should share a model to maximize KV-cache reuse
- **Speculation insertion**: Identify nodes where speculative execution is profitable
- **Context budget analysis**: Calculate per-node context requirements
- **Pipeline scheduling**: Determine optimal inter-node streaming strategy

---

## Relationship to Existing Work

| Component | Current State | Target State |
|-----------|--------------|-------------|
| **APXM Runtime** | Static model config, no GPU awareness | Dynamic model routing, graph-aware inference hints |
| **APXM Compiler** | MLIR passes for graph optimization | + model affinity, speculation insertion, context analysis |
| **vLLM** | Independent request serving | Graph-metadata-aware serving with result pinning |
| **ACPX** | Static `.flow.ts` files, sequential execution | Graph generator -> APXM compiler -> parallel execution |
| **OpenClaw** | Trigger layer only | + graph generation from natural language |
| **Patent (IDF)** | Theoretical techniques | Implemented across APXM runtime + vLLM backend |

---

## Success Metrics

1. **Latency**: 2-10x wall-clock speedup for multi-stage pipelines (speculation + pipelining + memoization)
2. **Cost**: 3-10x reduction in context tokens per stage (spaghetti-stack + stage-specific loading)
3. **GPU Utilization**: >90% utilization during multi-agent workflows (graph-aware scheduling)
4. **Developer Experience**: Zero-config workflow creation from natural language
5. **Reliability**: Automatic model failover with priority-based routing
