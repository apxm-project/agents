# Top 5 Optimization Opportunities

**Date**: March 31, 2026
**Scope**: Cross-cutting optimizations across APXM, vLLM, and ACPX

---

## Opportunity 1: Dynamic Model Routing with Priority Fallback

### Problem

Model selection is static and brittle. In APXM, the model is set at graph authoring time via node attributes (`model: "gpt-4"`). In ACPX, it's hardcoded in the flow definition (`profile: "claude"`). If the selected model is down, overloaded, or rate-limited, the entire pipeline fails.

Meanwhile, the available model fleet is rich and dynamic:

| Model | TPM | RPM | Health | Tier |
|-------|-----|-----|--------|------|
| gpt-5.4 | 15M | 150K | Healthy | Top |
| gpt-5.4-mini | 15M | 150K | Healthy | Fast |
| gpt-5.4-nano | 225M | 225K | Healthy | Budget |
| gpt-5.3-chat | 8M | 8K | Healthy | Legacy |
| gpt-5.3-codex | 15M | 150K | Healthy | Code |
| computer-use-preview | 2M | 2K | Healthy | Special |

### Solution

**Model Router** in APXM runtime that resolves models at execution time:

```rust
// New: ModelRouter in apxm-runtime
pub struct ModelRouter {
    registry: Arc<ModelRegistry>,       // Available models + health
    policies: Vec<RoutingPolicy>,       // Priority, cost, latency rules
    circuit_breakers: DashMap<String, CircuitBreaker>,  // Per-model health
}

pub struct RoutingPolicy {
    pub name: String,
    pub priority: Vec<ModelCandidate>,  // Ordered fallback list
    pub constraints: RoutingConstraints, // Min capability, max cost, max latency
}

pub struct ModelCandidate {
    pub model_id: String,
    pub provider: String,
    pub weight: f64,           // Load balancing weight
    pub capabilities: Vec<ModelCapability>, // chat, code, multimodal, reasoning
}

impl ModelRouter {
    pub async fn resolve(&self, request: &LLMRequest, op_type: AISOperationType) -> ModelCandidate {
        // 1. Filter by capability (ASK -> chat, THINK -> reasoning, etc.)
        // 2. Filter by health (circuit breaker state)
        // 3. Sort by policy priority
        // 4. Apply load balancing within same-priority tier
        // 5. Return best candidate
    }
}
```

**Integration points**:
- `apxm-runtime/src/executor/handlers/llm.rs`: Replace static model lookup with `model_router.resolve()`
- `apxm-cli/src/commands/register.rs`: Add `apxm models list` showing live health status
- New AIS node attribute: `model_policy: "fast"` / `"best"` / `"budget"` instead of `model: "gpt-4"`

### Impact

- **Reliability**: Automatic failover when a model goes down (zero downtime)
- **Cost**: Route simple ASK operations to nano models (225M TPM, fraction of cost)
- **Latency**: Route to least-loaded model in the priority tier
- **Flexibility**: New models added to fleet without recompiling graphs

### Effort: Medium (2-3 weeks)

---

## Opportunity 2: Graph-Aware Inference Scheduling (vLLM Extension)

> **Updated**: See [vLLM Graph Awareness](vllm-graph-awareness.md) for the revised approach based on repo investigation.

### Problem

vLLM treats every inference request as independent. But in an agent pipeline, requests are causally linked: the output of node A becomes the input of node B. This creates two inefficiencies:

1. **Unnecessary data movement**: Node A's output is decoded to text, sent over the network, then re-tokenized and re-prefilled for node B. The KV-cache from A's generation is discarded.

2. **Idle GPU time**: While the APXM runtime processes node A's output and constructs node B's prompt, the GPU sits idle. The runtime doesn't know it could start prefilling node B's static context (system prompt, few-shot examples) immediately.

### Solution

**Graph Metadata API** between APXM runtime and vLLM:

```python
# New: Graph-aware request metadata for vLLM
class GraphAwareRequest:
    request_id: str
    prompt: str
    # --- New fields ---
    graph_id: str                    # Which execution graph this belongs to
    node_id: int                     # Which node in the graph
    downstream_nodes: List[int]      # Nodes that will consume this output
    prefillable_context: str         # Static context for next node (start prefilling now)
    result_disposition: str          # "stream" | "pin_in_gpu" | "cache"
    priority: str                    # "critical_path" | "speculative" | "background"
```

**Three key behaviors**:

1. **Result Pinning**: When `downstream_nodes` is non-empty, keep the KV-cache warm instead of evicting it. The next node's prefill can reuse the cached keys/values.

2. **Eager Prefill**: When `prefillable_context` is provided, start prefilling the next node's static context (system prompt, skill embeddings) while the current node is still generating. This overlaps prefill compute with generation I/O.

3. **Priority Scheduling**: Critical-path nodes (on the graph's longest path) get higher scheduling priority than speculative or background nodes, improving end-to-end latency.

### Integration Architecture

```
APXM Runtime                          vLLM Backend
     |                                      |
     | POST /v1/completions                 |
     | + X-Graph-Id: exec-123              |
     | + X-Node-Id: 5                      |
     | + X-Downstream: [6, 7]             |
     | + X-Priority: critical_path         |
     |------------------------------------->|
     |                                      |  Schedule with graph awareness
     |                                      |  Pin KV-cache for downstream
     |                                      |  Start prefilling node 6's context
     |<-------------------------------------|
     | Response + cache_token               |
     |                                      |
     | POST /v1/completions (node 6)        |
     | + X-Cache-Token: <token>            |
     | + X-Reuse-KV: true                  |
     |------------------------------------->|
     |                                      |  Attach to warm KV-cache
     |                                      |  Skip redundant prefill
```

### Impact

- **Latency**: 30-50% reduction in inter-node latency (skip re-tokenization + re-prefill)
- **GPU Utilization**: 20-40% improvement during pipeline execution (no idle gaps)
- **Memory**: Shared KV-cache across pipeline stages reduces GPU memory pressure
- **Throughput**: Pipeline overlapping means more requests served per unit time

### Effort: Hard (4-6 weeks, requires vLLM extension)

---

## Opportunity 3: Spaghetti-Stack Context Management (Patent Implementation)

### Problem

Every pipeline stage receives the **full accumulated context** -- all prior tool outputs, file contents, reasoning traces, and conversation history. As the internal patent documents, this causes:

- Context pollution (irrelevant information dilutes model attention)
- Token waste (paying for tokens the model doesn't need)
- Quality degradation (more noise = worse answers)
- Memory pressure (larger KV-caches consume more GPU RAM)

### Solution

Implement the five techniques from the Spaghetti-Stack Context Management patent directly in the APXM runtime:

#### 3a. Spaghetti-Stack Context Organization

```rust
// New: Context stack in APXM runtime
pub struct ContextStack {
    root: ContextFrame,
    branches: HashMap<BranchId, Vec<ContextFrame>>,
}

pub struct ContextFrame {
    pub id: FrameId,
    pub parent: Option<FrameId>,
    pub summary: String,                    // Collapsed summary
    pub references: Vec<ContextReference>,  // Pointers to full data
    pub data_chunks: Vec<DataChunk>,        // Actual loaded data
    pub status: FrameStatus,               // Active | Collapsed | Pruned
}

pub struct ContextReference {
    pub pointer: String,           // e.g., "[->test_output]"
    pub storage_key: String,       // Where to fetch full data
    pub estimated_tokens: usize,   // Cost to load
}
```

#### 3b. Demand-Paged Context Loading

Only load data chunks when the model explicitly references them (or when the stage manifest declares them):

```rust
impl ContextStack {
    pub fn assemble_for_node(&self, node: &Node) -> AssembledContext {
        // 1. Walk from node's leaf frame to root (bottom-up traversal)
        // 2. Collect summaries (always loaded)
        // 3. Resolve only declared reference pointers (lazy load)
        // 4. Return assembled context (34% of flat approach per patent)
    }
}
```

#### 3c. Stage-Specific Context Manifests

Each AIS operation declares what context it needs:

```json
{
  "id": 5,
  "name": "review_api",
  "op": "ASK",
  "attributes": {
    "template": "Review the API endpoint: {0}",
    "context_manifest": ["api_endpoints", "test_results"],
    "context_exclude": ["auth_module", "config_files"]
  }
}
```

#### 3d. Input-Keyed Memoization

```rust
pub struct MemoCache {
    session_cache: DashMap<CacheKey, CachedResult>,   // Microsecond access
    persistent_cache: SqlitePool,                       // Millisecond cross-session
}

pub struct CacheKey {
    pub input_hash: u64,        // Hash of all inputs
    pub operation: String,      // AIS operation type
    pub model_config: String,   // Model + temperature + params
}

impl MemoCache {
    pub async fn check(&self, key: &CacheKey) -> Option<CachedResult> {
        // 1. Check session cache (O(1), ~1us)
        // 2. Check persistent cache (~1ms)
        // 3. Return if temperature=0 and exact match
    }
}
```

#### 3e. Speculative Consumer Execution

```rust
// In DataflowScheduler: speculative execution of downstream nodes
impl DataflowScheduler {
    fn maybe_speculate(&self, node: &Node, graph: &ExecutionDag) -> Option<SpeculativeTask> {
        // 1. Check if downstream node has memoized prior output
        // 2. If yes, start executing downstream with predicted input
        // 3. When actual upstream output arrives:
        //    - If matches prediction: commit speculative result
        //    - If differs: rollback and re-execute
    }
}
```

### Impact

- **Tokens**: 3-10x reduction per stage (only load relevant context)
- **Quality**: Better model outputs (focused attention, less noise)
- **Latency**: Microsecond returns for memoized operations
- **GPU Memory**: Smaller KV-caches per request
- **Patent value**: Implements the filed invention

### Effort: Hard (6-8 weeks, core runtime changes)

---

## Opportunity 4: ACPX-to-APXM Graph Compiler

### Problem

ACPX flows are:
- **Sequential**: One node at a time, no parallelism
- **Static**: Model/agent assignment fixed at definition time
- **Interpreted**: No compilation, no optimization passes
- **TypeScript-only**: Requires Node.js runtime

Meanwhile, APXM provides:
- **Parallel** dataflow execution with work-stealing
- **Compiled** graphs with MLIR optimization passes
- **32 AIS operations** with rich semantics
- **Rust runtime** with microsecond scheduling overhead

### Solution

A compiler that translates ACPX flow definitions into APXM graphs:

```
  .flow.ts (ACPX)         APXM Graph JSON           .apxmobj (compiled)
  ┌──────────────┐        ┌──────────────┐          ┌──────────────┐
  │ defineFlow({ │  --->  │ {"nodes": [  │  --->    │ Binary       │
  │   nodes: {   │  parse │   {op: "ASK"}│ compile  │ artifact     │
  │     review:  │        │   {op: "INV"}│          │ (optimized)  │
  │       acp({})│        │   {op: "BRV"}│          │              │
  │   }          │        │ ], "edges":[]│          │              │
  │ })           │        │ }            │          │              │
  └──────────────┘        └──────────────┘          └──────────────┘
```

**Mapping table**:

| ACPX Node Type | APXM AIS Operation | Notes |
|----------------|-------------------|-------|
| `compute` | `CONST_STR` + custom | Inline compute as literal values |
| `acp` | `ASK` / `THINK` / `REASON` | Map by complexity/instruction |
| `shell` | `EXC` | Execute in sandbox |
| `action` | `INV` | Invoke capability |
| `checkpoint` | `PAUSE` | Human-in-the-loop |
| `switch` edge | `BRANCH_ON_VALUE` | Conditional routing |
| Sequential edges | `Data` dependency | Direct mapping |

**What APXM adds that ACPX doesn't have**:
- Parallel execution of independent nodes
- Operation fusion (adjacent ASK+ASK -> single call)
- Dead path elimination (unreachable branches removed)
- Model affinity optimization (nodes sharing context get same model)
- Speculative execution of downstream nodes
- Memoization of repeated computations

### Integration Architecture

```python
# New CLI command: acpx compile -> apxm
# or: apxm import --from acpx flow.ts

# Phase 1: Static translation
apxm import review.flow.ts -o review.apxm
apxm compile review.apxm -o review.apxmobj

# Phase 2: Dynamic generation
# OpenClaw agent receives request, calls:
apxm generate "review PRs, run tests, merge if clean" -o pipeline.apxm
apxm compile pipeline.apxm -O2 -o pipeline.apxmobj
apxm run pipeline.apxmobj --input '{"repo":"org/repo","pr":42}'
```

### Impact

- **Performance**: 10x speedup from parallelism (APXM benchmark: 10.37x multi-agent)
- **Optimization**: Compiler passes reduce redundant work
- **Portability**: Run ACPX workflows without Node.js
- **Composability**: Merge multiple flows into optimized super-graphs

### Effort: Medium-Hard (4-5 weeks)

---

## Opportunity 5: Unified Observability and Feedback Loop

### Problem

Three separate observability systems with no cross-correlation:

| System | Observability | Format |
|--------|-------------|--------|
| ACPX | `~/.acpx/flows/runs/<runId>/trace.ndjson` | NDJSON events |
| APXM | `--emit-metrics metrics.json` | JSON stats file |
| vLLM | Prometheus metrics + request logs | Metrics endpoint |

When a pipeline is slow, you can't tell whether the bottleneck is:
- Orchestration overhead (ACPX/APXM scheduling)?
- Model inference latency (vLLM)?
- Context assembly time (context stack management)?
- Network I/O (API calls)?

### Solution

**Unified Trace Format** with distributed tracing across all three systems:

```json
{
  "trace_id": "exec-123",
  "spans": [
    {
      "span_id": "s1",
      "operation": "apxm.scheduler.dispatch",
      "node_id": 5,
      "start_us": 0,
      "duration_us": 150,
      "attributes": {"node_name": "review_code", "op": "ASK"}
    },
    {
      "span_id": "s2",
      "parent_span": "s1",
      "operation": "apxm.context.assemble",
      "start_us": 150,
      "duration_us": 45,
      "attributes": {"tokens_loaded": 1150, "tokens_skipped": 2250}
    },
    {
      "span_id": "s3",
      "parent_span": "s1",
      "operation": "vllm.inference",
      "start_us": 195,
      "duration_us": 28000,
      "attributes": {
        "model": "gpt-5.4",
        "input_tokens": 1150,
        "output_tokens": 340,
        "kv_cache_hit": true,
        "prefill_overlap_pct": 0.35
      }
    }
  ]
}
```

**Feedback loop**: Traces feed back into the compiler for profile-guided optimization:

```
Execution traces -> Profile data -> Compiler passes -> Better schedule -> Faster execution
                                       |
                                       v
                                   Model routing
                                   adjustments
                                       |
                                       v
                                   Speculation
                                   confidence
                                   updates
```

### Impact

- **Debugging**: Pinpoint bottlenecks across the full stack in one view
- **Optimization**: Profile-guided compilation using real execution data
- **Monitoring**: Unified dashboard for all pipeline metrics
- **Learning**: Memoization and speculation improve over time from trace data

### Effort: Medium (3-4 weeks)

---

## Summary Comparison

| Opportunity | Impact | Effort | Risk | Priority |
|-------------|--------|--------|------|----------|
| 1. Dynamic Model Routing | High | Medium (2-3w) | Low | **P0** |
| 2. Graph-Aware Inference | Very High | Hard (4-6w) | Medium | **P1** |
| 3. Spaghetti-Stack Context | Very High | Hard (6-8w) | Medium | **P1** |
| 4. ACPX-to-APXM Compiler | High | Medium-Hard (4-5w) | Low | **P0** |
| 5. Unified Observability | Medium | Medium (3-4w) | Low | **P2** |

**Recommended order**: 1 -> 4 -> 2+3 (parallel) -> 5

---

> See also: [Gap Analysis](gap-analysis.md) for per-gap prioritization, [Optimization Targets](optimization-targets.md) for goal-directed compiler passes, [Master Plan](master-plan.md) for validation and timeline, [DSPy Integration](dspy-integration.md) for prompt optimization.
