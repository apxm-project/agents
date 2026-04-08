# Architecture Integration Map

**Date**: March 31, 2026
**Scope**: How APXM, vLLM, and ACPX connect at the code level

---

## Current State: Three Isolated Systems

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              TODAY                                          │
│                                                                             │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐                  │
│  │    ACPX       │    │    APXM       │    │    vLLM       │                 │
│  │              │    │              │    │              │                 │
│  │ .flow.ts     │    │ .apxm.json   │    │ OpenAI API   │                 │
│  │ Sequential   │    │ Parallel     │    │ Batched      │                 │
│  │ Node.js      │    │ Rust+MLIR    │    │ Python+CUDA  │                 │
│  │              │    │              │    │              │                 │
│  │ No GPU       │    │ No model     │    │ No graph     │                 │
│  │ awareness    │    │ awareness    │    │ awareness    │                 │
│  └──────────────┘    └──────────────┘    └──────────────┘                  │
│        |                    |                    |                          │
│        |                    |                    |                          │
│   (disconnected)      (disconnected)      (disconnected)                   │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Target State: Unified Agent Execution

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              TARGET                                         │
│                                                                             │
│  ┌──────────────┐                                                          │
│  │  OpenClaw     │  Trigger: "Review PR #42"                               │
│  │  (Trigger)    │                                                          │
│  └──────┬───────┘                                                          │
│         │                                                                   │
│         v                                                                   │
│  ┌──────────────┐    ┌──────────────┐                                      │
│  │  ACPX Flow   │───>│  APXM Graph   │  acpx-to-apxm compiler             │
│  │  (Authoring) │    │  (Compiled IR) │  Parallelism detection             │
│  └──────────────┘    └──────┬───────┘                                      │
│                             │                                               │
│                    ┌────────v────────┐                                      │
│                    │  APXM Compiler   │  MLIR passes:                      │
│                    │  + Model Affinity │  - Dead code elimination           │
│                    │  + Speculation    │  - Operation fusion                │
│                    │  + Context Budget │  - Model affinity analysis         │
│                    └────────┬────────┘  - Speculation insertion            │
│                             │                                               │
│                    ┌────────v────────┐    ┌──────────────┐                  │
│                    │  APXM Runtime    │<──>│  vLLM         │                │
│                    │                  │    │               │                │
│                    │  ModelRouter ────────>│ Graph-Aware   │                │
│                    │  ContextStack ───────>│ Scheduler     │                │
│                    │  MemoCache ──────────>│ KV-Cache Pin  │                │
│                    │  Speculation ────────>│ Priority Sched│                │
│                    │  DataflowSched ─────>│ Streaming     │                │
│                    └────────┬────────┘    └──────────────┘                  │
│                             │                                               │
│                    ┌────────v────────┐                                      │
│                    │  Unified Trace   │  OpenTelemetry-compatible          │
│                    │  + Replay Viewer │  Cross-system correlation           │
│                    └─────────────────┘                                      │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Integration Points (Code Level)

### 1. ACPX -> APXM (Graph Translation)

```
ACPX Side                              APXM Side
─────────                              ─────────
acpx/src/flows/flow-loader.ts          apxm-graph/src/lib.rs
  defineFlow() -> FlowDefinition         ApxmGraph { nodes, edges, ... }

acpx/src/flows/types.ts                apxm-ais/src/operations/definitions.rs
  AcpNode | ComputeNode | ShellNode      AISOperationType (32 variants)

acpx/src/flows/runner.ts               apxm-runtime/src/scheduler/dataflow.rs
  FlowRunner (sequential)                DataflowScheduler (parallel)

NEW: tools/acpx-to-apxm/               apxm-cli/src/commands/import.rs
  TypeScript bridge that loads           `apxm import --from acpx`
  .flow.ts and emits .apxm.json
```

**Translation table**:

| ACPX | APXM | Notes |
|------|------|-------|
| `defineFlow({ name, startAt, nodes, edges })` | `ApxmGraph { name, nodes, edges, parameters, metadata }` | Direct mapping |
| `compute({ run: fn })` | `CONST_STR` or `EXC` | Static -> literal, dynamic -> sandbox exec |
| `acp({ profile, prompt, parse })` | `ASK` + attributes | Profile maps to model_policy |
| `shell({ exec, parse })` | `EXC` | Command + args |
| `checkpoint({ summary })` | `PAUSE` | Human-in-the-loop |
| `{ from, to }` | `{ from_id, to_id, dependency: "Data" }` | Linear edge |
| `{ from, switch: { on, cases } }` | `BRANCH_ON_VALUE` node + edges | Needs intermediate node |
| `session.handle` | `FlowCall` group metadata | Shared context = same agent |
| `embedSkills([...])` | Context manifest attributes | Skill content in system_prompt |

### 2. APXM -> vLLM (Graph-Aware Inference)

```
APXM Side                              vLLM Side
─────────                              ─────────
apxm-backends/src/lib.rs               vllm/entrypoints/openai/api_server.py
  trait LLMBackend {                     app.post("/v1/completions")
    async fn generate(LLMRequest)
  }

NEW: apxm-backends/src/vllm_aware.rs   NEW: vllm/extensions/graph_scheduler.py
  GraphAwareVllmBackend {                 GraphAwareScheduler {
    register_graph()                        active_graphs
    generate_with_hints()                   pinned_kv_caches
  }                                         priority_queue
                                          }

apxm-runtime/src/scheduler/dataflow.rs vllm/core/scheduler.py
  DataflowScheduler                       Scheduler.schedule()
  - knows critical path                  - receives priority hints
  - publishes graph metadata              - pins KV-cache for downstream
  - sends prefillable context             - starts eager prefill
```

**New API surface** (REST extension on vLLM):

```
POST /v1/graphs/register
  Body: { graph_id, nodes, critical_path, prefillable_contexts }

POST /v1/completions (extended)
  Headers:
    X-APXM-Graph-Id: exec-123
    X-APXM-Node-Id: 5
    X-APXM-Downstream: 6,7
    X-APXM-Priority: critical_path
    X-APXM-Disposition: pin_in_gpu

DELETE /v1/graphs/{graph_id}
  Cleanup after execution completes

GET /v1/graphs/{graph_id}/status
  Returns: pinned caches, active nodes, memory usage
```

### 3. Model Router (APXM Runtime Internal)

```
apxm-runtime/src/
├── model_router/
│   ├── mod.rs              # ModelRouter struct + resolve() method
│   ├── registry.rs         # ModelRegistry: catalog of available models
│   ├── health.rs           # HealthMonitor: circuit breakers + latency tracking
│   ├── policy.rs           # RoutingPolicy: priority lists + constraints
│   └── config.rs           # ~/.apxm/models.toml schema
│
├── executor/handlers/
│   └── llm.rs              # MODIFIED: uses ModelRouter.resolve() instead of static lookup
│
├── runtime.rs              # MODIFIED: initializes ModelRouter
└── context.rs              # MODIFIED: ModelRouter in ExecutionContext
```

**Interaction flow**:

```
Handler::llm::execute()
  |
  |-- 1. Extract model_policy from node.attributes
  |     (e.g., "fast", "best", "budget", "specific:gpt-5.4")
  |
  |-- 2. ctx.model_router.resolve(request, op_type, policy)
  |     |-- Check circuit breakers (skip unhealthy models)
  |     |-- Filter by capability (chat, reasoning, code, multimodal)
  |     |-- Sort by policy priority
  |     |-- Apply load balancing within tier
  |     |-- Return ModelCandidate { model_id, provider, base_url }
  |
  |-- 3. ctx.llm_registry.get_or_create_backend(candidate)
  |     (Lazy initialization of backend for resolved model)
  |
  |-- 4. backend.generate(request)
  |
  |-- 5. On failure: circuit_breaker.record_failure(model_id)
  |     -> Retry with next candidate from priority list
```

### 4. Context Stack (APXM Runtime Internal)

```
apxm-runtime/src/
├── context_stack/
│   ├── mod.rs              # ContextStack, ContextFrame, ContextReference
│   ├── assembly.rs         # assemble_for_node(): bottom-up + top-down traversal
│   ├── demand_paging.rs    # Lazy loading of referenced data chunks
│   ├── manifest.rs         # Per-node context declarations (from graph attributes)
│   └── collapse.rs         # Collapse completed branches into summaries
│
├── memo/
│   ├── mod.rs              # MemoCache struct
│   ├── hasher.rs           # Input hashing (op + model + inputs -> cache key)
│   └── speculation.rs      # Speculative execution with commit/rollback
│
├── scheduler/
│   └── dataflow.rs         # MODIFIED: speculation + memoization checks before dispatch
│
└── executor/handlers/
    └── llm.rs              # MODIFIED: context assembly from ContextStack
```

---

## Data Flow (End-to-End)

```
1. User Request
   "Review all open PRs in org/repo"

2. OpenClaw Trigger
   Matches "PR review" skill
   Spawns flow for each open PR

3. ACPX Flow (or direct APXM graph)
   review.flow.ts loaded

4. acpx-to-apxm Translation
   .flow.ts -> ApxmGraph JSON
   Parallelism detected: review + test can run independently

5. APXM Compilation
   apxm compile review.apxm
   Passes:
     - DAG validation (unique IDs, valid ops)
     - Dead code elimination
     - FuseReasoning (adjacent ASK+THINK -> single call)
     - Model affinity analysis (nodes sharing context -> same model)
     - Critical path analysis (identify bottleneck nodes)
     - Speculation insertion (add speculative edges where profitable)
     - Context budget analysis (calculate per-node token requirements)
   Output: review.apxmobj

6. APXM Execution
   Runtime loads artifact
   DataflowScheduler builds ready set

   For each ready node:
     a. Check MemoCache (exact input hash)
        -> If hit: return cached result in microseconds

     b. Check speculation opportunity
        -> If downstream has cached prior output: start speculative execution

     c. Assemble context from ContextStack
        -> Walk spaghetti stack (bottom-up from leaf)
        -> Load only declared manifests
        -> Skip irrelevant branches

     d. Resolve model via ModelRouter
        -> Check health, filter by capability
        -> Select best available model from priority list

     e. Send to vLLM with graph hints
        -> Priority: critical_path or normal
        -> Disposition: pin_in_gpu if downstream exists
        -> Prefillable context for next node

     f. Stream response
        -> If pipelining: stream tokens into next node's prefill
        -> If not: buffer complete response

     g. Publish output token
        -> Signal ready_set for downstream nodes
        -> Record in episodic memory

7. Results
   RuntimeExecutionResult {
     results: final node outputs
     stats: nodes executed, failed, duration
     scheduler_metrics: utilization, queue depth
     llm_metrics: per-model token counts, latencies
     context_metrics: tokens loaded vs available, cache hits
     speculation_metrics: predictions attempted, hit rate
   }
```

---

## File Change Summary

### New Files

| File | System | Purpose |
|------|--------|---------|
| `apxm-runtime/src/model_router/mod.rs` | APXM | Model routing with health + priorities |
| `apxm-runtime/src/model_router/health.rs` | APXM | Circuit breakers + health monitoring |
| `apxm-runtime/src/model_router/registry.rs` | APXM | Model catalog management |
| `apxm-runtime/src/model_router/policy.rs` | APXM | Routing policies (fast/best/budget) |
| `apxm-runtime/src/context_stack/mod.rs` | APXM | Spaghetti-stack context management |
| `apxm-runtime/src/context_stack/assembly.rs` | APXM | Context traversal and assembly |
| `apxm-runtime/src/context_stack/demand_paging.rs` | APXM | Lazy loading of data chunks |
| `apxm-runtime/src/memo/mod.rs` | APXM | Two-tier memoization cache |
| `apxm-runtime/src/memo/speculation.rs` | APXM | Speculative execution engine |
| `apxm-backends/src/vllm_graph_aware.rs` | APXM | Graph-aware vLLM backend |
| `apxm-cli/src/commands/models.rs` | APXM | `apxm models list/health` |
| `apxm-cli/src/commands/import.rs` | APXM | `apxm import --from acpx` |
| `tools/acpx-to-apxm/src/index.ts` | Bridge | Flow translator (TypeScript) |
| `vllm/extensions/graph_scheduler.py` | vLLM | Graph-aware scheduling extension |

### Modified Files

| File | System | Change |
|------|--------|--------|
| `apxm-runtime/src/executor/handlers/llm.rs` | APXM | ModelRouter + ContextStack integration |
| `apxm-runtime/src/runtime.rs` | APXM | Initialize new subsystems |
| `apxm-runtime/src/context.rs` | APXM | Add ModelRouter, ContextStack, MemoCache to ctx |
| `apxm-runtime/src/scheduler/dataflow.rs` | APXM | Memoization + speculation before dispatch |
| `apxm-backends/src/lib.rs` | APXM | LLMBackend trait: add health_check() |
| `apxm-cli/src/main.rs` | APXM | Register models + import subcommands |
| `vllm/core/scheduler.py` | vLLM | Priority hints + KV-cache pinning |
| `vllm/entrypoints/openai/api_server.py` | vLLM | Graph registration endpoint |

---

## Configuration Files

### ~/.apxm/models.toml (New)

```toml
[defaults]
routing_policy = "best"        # Default policy for unspecified nodes
health_check_interval_ms = 30000
circuit_breaker_threshold = 3
circuit_breaker_reset_ms = 60000

[[models]]
id = "gpt-5.4"
provider = "llm-gateway"
base_url = "https://llm-gateway.internal/v1"
api_key_env = "LLM_GATEWAY_API_KEY"
capabilities = ["chat", "multimodal", "reasoning"]
tier = "top"
tpm = 15_000_000
rpm = 150_000
cost_per_1k_input = 0.03
cost_per_1k_output = 0.06

[[models]]
id = "gpt-5.4-nano"
provider = "llm-gateway"
base_url = "https://llm-gateway.internal/v1"
api_key_env = "LLM_GATEWAY_API_KEY"
capabilities = ["chat", "multimodal"]
tier = "budget"
tpm = 225_000_000
rpm = 225_000
cost_per_1k_input = 0.001
cost_per_1k_output = 0.002

[[models]]
id = "local-llama"
provider = "vllm"
base_url = "http://localhost:8000/v1"
capabilities = ["chat"]
tier = "local"
# No rate limits for local models

[[policies]]
name = "fast"
description = "Lowest latency, prefer nano/mini models"
priority = ["gpt-5.4-nano", "gpt-5.4-mini", "gpt-5.4"]

[[policies]]
name = "best"
description = "Highest quality, prefer top-tier models"
priority = ["gpt-5.4", "gpt-5.3-codex", "gpt-5.4-mini"]

[[policies]]
name = "budget"
description = "Lowest cost, prefer cheap models"
priority = ["gpt-5.4-nano", "local-llama", "gpt-5.4-mini"]

[[policies]]
name = "code"
description = "Code-specialized models preferred"
priority = ["gpt-5.3-codex", "gpt-5.4", "gpt-5.4-mini"]
```

### Graph attributes (extended)

```json
{
  "id": 5,
  "name": "review_code",
  "op": "ASK",
  "attributes": {
    "template": "Review this code: {0}",
    "model_policy": "best",
    "context_manifest": ["diff", "project_conventions"],
    "context_exclude": ["build_logs", "old_traces"],
    "memoizable": true,
    "speculation_confidence": 0.8
  }
}
```
