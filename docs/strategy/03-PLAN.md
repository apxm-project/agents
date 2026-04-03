# Implementation Plan (Revised)

**Date**: March 31, 2026
**Scope**: APXM + vLLM unified agent execution (ACPX absorbed into APXM)

---

## Overview

```
Phase 1 (Weeks 1-3)    Phase 2 (Weeks 4-6)     Phase 3 (Weeks 7-11)    Phase 4 (Weeks 12-16)
┌─────────────────┐    ┌─────────────────┐      ┌─────────────────┐     ┌─────────────────┐
│ Model Router    │    │ ACP Client in   │      │ Graph-Aware     │     │ Patent Impl +   │
│ + Health API    │    │ APXM            │      │ vLLM Extension  │     │ Optimization    │
│                 │    │                  │      │                 │     │ Targets         │
│ Foundation for  │    │ ~800 lines Rust  │      │ Deep HW-SW     │     │ -O(tokens),     │
│ dynamic routing │    │ replaces 30K TS  │      │ integration     │     │ -O(parallel)... │
└─────────────────┘    └─────────────────┘      └─────────────────┘     └─────────────────┘
```

**Key change from v1**: Phase 2 is no longer "ACPX-to-APXM Graph Compiler" (4 weeks of TypeScript bridge code). ACPX is absorbed -- its only unique value is the ACP protocol client, which becomes a `CapabilityExecutor` inside APXM (~800 lines of Rust). Phase 4 adds goal-directed optimization targets to the compiler.

---

## Phase 1: Dynamic Model Router (Weeks 1-3)

**Goal**: APXM runtime resolves models at execution time, not compile time.

### Week 1: Model Registry + Health Monitor

**Files to create/modify**:

| File | Action | Description |
|------|--------|-------------|
| `apxm-runtime/src/model_router/mod.rs` | Create | ModelRouter, RoutingPolicy, ModelCandidate |
| `apxm-runtime/src/model_router/health.rs` | Create | HealthMonitor with circuit breakers |
| `apxm-runtime/src/model_router/registry.rs` | Create | ModelRegistry (available models + metadata) |
| `apxm-runtime/src/lib.rs` | Modify | Add `pub mod model_router` |

**Key design decisions**:

1. **Model catalog format** (`~/.apxm/models.toml`):
```toml
[[models]]
id = "gpt-5.4"
provider = "azure-openai"  # or "openai", "ollama", "vllm-local"
capabilities = ["chat", "multimodal", "reasoning"]
tier = "top"
tpm_limit = 15_000_000
rpm_limit = 150_000
cost_per_1k_input = 0.03
cost_per_1k_output = 0.06

[[models]]
id = "gpt-5.4-nano"
provider = "azure-openai"
capabilities = ["chat", "multimodal"]
tier = "budget"
tpm_limit = 225_000_000
rpm_limit = 225_000
cost_per_1k_input = 0.001
cost_per_1k_output = 0.002

[[models]]
id = "local-llama"
provider = "vllm"
base_url = "http://localhost:8000"
capabilities = ["chat"]
tier = "local"
```

2. **Health check protocol**: Periodic lightweight requests (every 30s) + circuit breaker on 3 consecutive failures.

3. **Policy resolution**: Each AIS operation type maps to a default routing policy:
```rust
// ASK: fast models preferred (latency: LOW)
// THINK: reasoning models preferred (latency: HIGH)
// REASON: structured output models preferred (latency: MEDIUM)
// PLAN: reasoning models with large context
// REFLECT: any capable model
// VERIFY: structured output models
```

### Week 2: Runtime Integration

**Files to modify**:

| File | Action | Description |
|------|--------|-------------|
| `apxm-runtime/src/executor/handlers/llm.rs` | Modify | Use ModelRouter instead of static model attr |
| `apxm-runtime/src/runtime.rs` | Modify | Initialize ModelRouter in Runtime::new() |
| `apxm-runtime/src/context.rs` | Modify | Add ModelRouter to ExecutionContext |
| `apxm-backends/src/lib.rs` | Modify | LLMBackend trait gains `health_check()` method |

**Behavioral change**:

```
BEFORE:
  node.attributes["model"] = "gpt-4"
  -> LLMRegistry.get("gpt-4")
  -> Backend.generate(request)

AFTER:
  node.attributes["model_policy"] = "fast"  // or "best", "budget", "specific:gpt-5.4"
  -> ModelRouter.resolve(request, op_type, policy)
  -> Returns best healthy model matching constraints
  -> LLMRegistry.get(resolved_model)
  -> Backend.generate(request)

  // Backward compatible: model_policy="specific:gpt-4" behaves like old model="gpt-4"
```

### Week 3: CLI + Testing

**Files to create/modify**:

| File | Action | Description |
|------|--------|-------------|
| `apxm-cli/src/commands/models.rs` | Create | `apxm models list`, `apxm models health` |
| `apxm-cli/src/main.rs` | Modify | Register models subcommand |
| `apxm-runtime/tests/model_router_tests.rs` | Create | Unit tests for routing logic |

**CLI output**:
```
$ apxm models list
MODEL             PROVIDER      TIER     HEALTH    COST/1K    TPM         RPM
gpt-5.4           azure-openai  top      healthy   $0.03      15M         150K
gpt-5.4-mini      azure-openai  fast     healthy   $0.005     15M         150K
gpt-5.4-nano      azure-openai  budget   healthy   $0.001     225M        225K
local-llama       vllm          local    healthy   $0.00      unlimited   unlimited

$ apxm models health --json
{"models": [{"id": "gpt-5.4", "health": "healthy", "latency_ms": 45, "utilization": 0.62, ...}]}
```

**Deliverables**:
- [ ] ModelRouter with priority-based resolution
- [ ] Health monitoring with circuit breakers
- [ ] `~/.apxm/models.toml` configuration
- [ ] `apxm models list/health` CLI commands
- [ ] Backward-compatible with existing `model` attribute
- [ ] Unit tests for routing logic + failover

---

## Phase 2: ACP Client in APXM (Weeks 4-6)

**Goal**: APXM can dispatch tasks to external coding agents (Claude Code, Codex, Gemini) via the Agent Client Protocol, eliminating the need for ACPX as a separate system.

**Why this replaces the old Phase 2** (ACPX-to-APXM compiler):
- ACPX's orchestration is redundant -- APXM already has DELEGATE, COMMUNICATE, SPAWN_AGENT, INV, EXC, PAUSE/RESUME, 3-tier memory, parallel scheduling, and MLIR optimization
- The only value ACPX adds is the ACP protocol client for talking to coding agents
- ~800 lines of Rust (a `CapabilityExecutor`) vs ~30K lines of TypeScript eliminated
- Users write APXM graphs directly (or generate them) instead of `.flow.ts` files

### Week 4: AcpCapability Core

**Files to create**:

| File | Action | Description |
|------|--------|-------------|
| `apxm-runtime/src/capability/acp.rs` | Create | AcpCapability implementing CapabilityExecutor |
| `apxm-runtime/src/capability/acp/registry.rs` | Create | Agent adapter registry (profile -> spawn command) |
| `apxm-runtime/src/capability/acp/protocol.rs` | Create | JSON-RPC 2.0 message construction and parsing |
| `apxm-runtime/src/capability/acp/session.rs` | Create | Session lifecycle (create, prompt, cancel) |

**Key implementation**: The ACP protocol is JSON-RPC 2.0 over stdio:

```rust
// 1. Spawn agent: npx @agentclientprotocol/claude-agent-acp@^0.24.2
// 2. Send: initialize { protocolVersion: "2025-11-05" }
// 3. Send: session/new {}
// 4. Send: session/prompt { sessionId, prompt: { type: "text", text: "..." } }
// 5. Collect streaming response events until completion
// 6. Handle reverse requests (readTextFile, createTerminal, etc.)
```

**Building on existing APXM code**:
- `UserToolCapability` pattern (subprocess JSON I/O) -- same spawn + stdin/stdout
- `apxm_mcp.rs` (JSON-RPC 2.0 message handling) -- same protocol
- `ProcessSandbox` (terminal operations) -- maps to ACP reverse requests

### Week 5: Configuration + Model Routing Integration

**Files to create/modify**:

| File | Action | Description |
|------|--------|-------------|
| `apxm-runtime/src/capability/acp/config.rs` | Create | `~/.apxm/agents.toml` loader |
| `apxm-runtime/src/capability/acp/reverse.rs` | Create | Handle agent reverse requests (file I/O, terminal) |
| Integration with ModelRouter | Modify | Model policy resolution for ACP agents |

**Configuration** (`~/.apxm/agents.toml`):

```toml
[defaults]
permission_mode = "approve-all"
timeout_ms = 300000

[[agents]]
profile = "claude"
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@^0.24.2"]
default_model = "claude-sonnet-4"

[[agents]]
profile = "codex"
command = "npx"
args = ["@zed-industries/codex-acp@^0.10.0"]

[[agents]]
profile = "gemini"
command = "gemini"
args = ["--acp"]

[[agents]]
profile = "custom"
command = "./my-agent"
args = ["--acp", "--stdio"]
env = { MY_API_KEY = "$MY_API_KEY" }
timeout_ms = 600000
```

### Week 6: CLI + End-to-End Testing

**Files to create/modify**:

| File | Action | Description |
|------|--------|-------------|
| `apxm-cli/src/commands/agent.rs` | Create | `apxm agent list/test/add` |
| `apxm-runtime/src/capability/mod.rs` | Modify | Auto-register AcpCapability |
| Tests | Create | End-to-end: graph with INV(acp) -> Claude Code |

**Two-tier model -- choosing the right dispatch**:

| Task Complexity | Operation | What Runs | Latency | Cost |
|----------------|-----------|-----------|---------|------|
| Simple analysis | `ASK` / `THINK` / `REASON` | Direct LLM call via vLLM/OpenAI | 1-10s | Low |
| Multi-step coding | `INV(acp)` | Full Claude Code / Codex / Gemini agent | 30-300s | High |
| Orchestrated workflow | `DELEGATE` | Sub-DAG in APXM runtime | Varies | Varies |

**Example graph** -- simple PR review (no agent needed):
```json
{
  "nodes": [
    {"id": 1, "name": "fetch_diff", "op": "EXC", "attributes": {"command": "gh pr diff {prNumber}"}},
    {"id": 2, "name": "review", "op": "ASK", "attributes": {
      "template_str": "Review this diff:\n{0}", "model_policy": "best"
    }}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}]
}
```

**Example graph** -- complex bug fix (needs full agent):
```json
{
  "nodes": [
    {"id": 1, "name": "understand_bug", "op": "ASK", "attributes": {
      "template_str": "Analyze this bug report: {0}", "model_policy": "best"
    }},
    {"id": 2, "name": "fix_and_test", "op": "INV", "attributes": {
      "capability": "acp",
      "params_json": "{\"profile\": \"claude\", \"prompt\": \"Fix this bug: {0}. Run tests until green.\"}"
    }}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}]
}
```

**Deliverables**:
- [ ] AcpCapability implementing CapabilityExecutor trait
- [ ] ACP JSON-RPC 2.0 protocol (initialize, session/new, session/prompt)
- [ ] Agent adapter registry with configurable profiles
- [ ] Reverse request handling (file I/O, terminal ops, permissions)
- [ ] `~/.apxm/agents.toml` configuration
- [ ] `apxm agent list/test` CLI commands
- [ ] End-to-end test: INV(acp) -> Claude Code -> code changes

---

## Phase 3: Graph-Aware vLLM Extension (Weeks 7-11)

**Goal**: vLLM becomes aware of the APXM execution graph and optimizes inference scheduling, KV-cache management, and prefill accordingly.

### Weeks 7-8: Graph Metadata Protocol

Define the communication protocol between APXM runtime and vLLM:

**New backend**: `apxm-backends/src/vllm_graph_aware.rs`

```rust
pub struct GraphAwareVllmBackend {
    base_url: String,
    client: reqwest::Client,
    active_graphs: DashMap<String, GraphMetadata>,
}

pub struct GraphMetadata {
    pub graph_id: String,
    pub nodes: Vec<NodeMetadata>,
    pub critical_path: Vec<u32>,
    pub prefillable_contexts: HashMap<u32, String>,
}

impl GraphAwareVllmBackend {
    /// Register a graph with vLLM before execution begins
    pub async fn register_graph(&self, metadata: GraphMetadata) -> Result<()>;

    /// Enhanced generate with graph hints
    pub async fn generate_with_hints(&self, request: LLMRequest, hints: GraphHints) -> Result<LLMResponse>;
}

pub struct GraphHints {
    pub graph_id: String,
    pub node_id: u32,
    pub downstream_nodes: Vec<u32>,
    pub result_disposition: ResultDisposition, // Stream | PinInGpu | Cache
    pub priority: RequestPriority,             // CriticalPath | Normal | Speculative
}
```

**New vLLM REST API**:

```
POST /v1/graphs/register
  Body: { graph_id, nodes, critical_path, prefillable_contexts }

POST /v1/completions (extended headers)
  X-APXM-Graph-Id: exec-123
  X-APXM-Node-Id: 5
  X-APXM-Downstream: 6,7
  X-APXM-Priority: critical_path
  X-APXM-Disposition: pin_in_gpu

DELETE /v1/graphs/{graph_id}
GET /v1/graphs/{graph_id}/status
```

### Weeks 9-10: vLLM Scheduler Extension (Python side)

**New vLLM plugin**: `vllm/extensions/graph_aware_scheduler.py`

```python
class GraphAwareScheduler(Scheduler):
    """Extended scheduler that understands APXM execution graphs."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.active_graphs: Dict[str, GraphMetadata] = {}
        self.pinned_kv_caches: Dict[str, KVCacheRef] = {}

    def register_graph(self, metadata: GraphMetadata):
        self.active_graphs[metadata.graph_id] = metadata
        self._compute_priorities(metadata)

    def schedule(self, requests: List[Request]) -> ScheduleResult:
        # 1. Separate graph-aware vs regular requests
        # 2. Prioritize critical-path nodes
        # 3. Pin KV-caches for downstream-feeding nodes
        # 4. Start prefilling downstream static contexts

    def on_request_complete(self, request_id: str, output: Output):
        hints = self._get_hints(request_id)
        if hints and hints.result_disposition == "pin_in_gpu":
            self.pinned_kv_caches[request_id] = output.kv_cache_ref
```

### Week 11: Integration Testing + Context Stack Foundation

- End-to-end: APXM graph -> vLLM with graph hints -> verify KV-cache reuse
- Benchmark pipeline latency with/without graph-aware scheduling
- Stress test: multiple concurrent graphs sharing the same vLLM instance
- Begin ContextStack data structure (carries into Phase 4)

**Deliverables**:
- [ ] Graph metadata protocol (Rust + Python)
- [ ] GraphAwareVllmBackend in apxm-backends
- [ ] vLLM scheduler extension with priority scheduling
- [ ] KV-cache pinning for downstream-feeding nodes
- [ ] Eager prefill of static contexts
- [ ] Benchmarks showing latency improvement

---

## Phase 4: Patent Implementation + Optimization Targets (Weeks 12-16)

**Goal**: Implement the patent's five techniques. Add goal-directed optimization targets to the APXM compiler. Wire everything together.

### Week 12: Spaghetti-Stack Context + Memoization

**New module**: `apxm-runtime/src/context_stack/`

```
apxm-runtime/src/context_stack/
  mod.rs          -- ContextStack, ContextFrame, ContextReference
  assembly.rs     -- Bottom-up and top-down traversal
  demand_paging.rs -- Lazy loading of referenced data
  manifest.rs     -- Per-node context declarations
  collapse.rs     -- Collapse completed branches into summaries
```

**New module**: `apxm-runtime/src/memo/`

```
apxm-runtime/src/memo/
  mod.rs          -- MemoCache (two-tier: session DashMap + persistent SQLite)
  hasher.rs       -- Input hashing (operation + model_config + inputs)
  speculation.rs  -- Speculative execution with commit/rollback
```

**MemoCache integration with DataflowScheduler**:
- Before dispatching a node, check MemoCache (exact input hash)
- If memoized result exists (temperature=0, exact match), return in microseconds
- If downstream node has memoized prior output, start speculative execution
- `memoizable: true` attribute on nodes opts in to caching

### Week 13: Speculative Execution + Token Pipelining

**Speculative execution** (patent technique 2):
```rust
// In DataflowScheduler, before waiting for all inputs:
if let Some(predicted_input) = speculator.can_speculate(&node, &dag) {
    let result = dispatch_node(&node, vec![predicted_input]).await;
    speculator.set_result(node.id, result);
}

// When actual upstream result arrives:
speculator.resolve(upstream_id, &actual_result);
if speculator.was_committed(node.id) {
    // Use speculative result -- zero additional latency
} else {
    // Re-execute with actual input (rollback)
}
```

**Token pipelining** (patent technique 3):
```rust
// Stream tokens from current node into next node's prefill
pub async fn execute_with_pipelining(
    ctx: &ExecutionContext,
    node: &Node,
    inputs: Vec<Value>,
    downstream: Option<&Node>,
) -> Result<Value> {
    if let Some(next_node) = downstream {
        let prefillable = ctx.context_stack.get_prefillable(next_node);
        ctx.vllm_backend.start_prefill(next_node.id, prefillable).await?;

        let stream = ctx.vllm_backend.generate_stream(request).await?;
        while let Some(token) = stream.next().await {
            ctx.vllm_backend.append_to_prefill(next_node.id, &token).await?;
        }
    }
}
```

### Week 14-15: Goal-Directed Optimization Targets

This is the key new addition: **the APXM compiler gets optimization targets** that control which passes run and how the runtime behaves.

**New compiler infrastructure**:

| File | Action | Description |
|------|--------|-------------|
| `apxm-compiler/src/passes/targets.rs` | Create | OptimizationTarget enum + target-to-passes mapping |
| `apxm-compiler/mlir/lib/Dialect/AIS/Transforms/ModelDowngrade.cpp` | Create | Replace expensive models with cheaper ones |
| `apxm-compiler/mlir/lib/Dialect/AIS/Transforms/ContextBudget.cpp` | Create | Per-node token budget analysis and enforcement |
| `apxm-compiler/mlir/lib/Dialect/AIS/Transforms/ParallelismExtraction.cpp` | Create | Detect and mark parallelizable subgraphs |
| `apxm-compiler/mlir/lib/Dialect/AIS/Transforms/SpeculationInsertion.cpp` | Create | Insert speculative edges where profitable |
| `apxm-compiler/mlir/lib/Dialect/AIS/Transforms/PipelineInsertion.cpp` | Create | Mark adjacent LLM nodes for token pipelining |
| `apxm-compiler/src/passes/pipeline.rs` | Modify | Target-aware pass list construction |
| `apxm-cli/src/main.rs` | Modify | Add `--target` flag |

**Optimization targets** (see [08-OPTIMIZATION-TARGETS.md](08-OPTIMIZATION-TARGETS.md) for full details):

```
$ apxm compile graph.apxm -O2 --target tokens     # Minimize token usage
$ apxm compile graph.apxm -O2 --target parallel    # Maximize parallelism
$ apxm compile graph.apxm -O2 --target latency     # Minimize end-to-end time
$ apxm compile graph.apxm -O2 --target cost        # Minimize API costs
$ apxm compile graph.apxm -O2 --target balanced     # Default: good all-around
```

Each target enables/disables specific passes and tunes pass options:

| Target | Key Passes Enabled | Key Passes Tuned | Runtime Effect |
|--------|-------------------|-------------------|----------------|
| `tokens` | ContextBudget, DeadContextElim, FuseAskOps(aggressive) | max_template_tokens↓, fusion_mode=eager | Stage-specific loading, context manifests enforced |
| `parallel` | ParallelismExtraction, SpeculationInsertion | parallel_threshold↓ | Scheduler uses max concurrency, speculation enabled |
| `latency` | PipelineInsertion, SpeculationInsertion, FuseAskOps | All latency-reducing passes | Token pipelining + eager prefill + speculation |
| `cost` | ModelDowngrade, FuseAskOps, CondenseOps | model_policy overrides to "budget" | Router prefers cheap models, fewer API calls |
| `balanced` | All standard passes at moderate settings | Default options | Best overall (default) |

**Composability**: Targets can be combined:
```
$ apxm compile graph.apxm -O2 --target tokens,parallel
```

### Week 16: Full Integration + Benchmarking

Wire all systems together:

```
User / OpenClaw Request
  -> Graph Generation (NL -> APXM graph)  [future: P3]
  -> apxm compile -O2 --target latency
       MLIR passes: normalize, build-prompt, fuse-ask-ops, parallelism-extraction,
                    speculation-insertion, pipeline-insertion, scheduling, cse, dce
  -> apxm run (DataflowScheduler + ModelRouter + ContextStack + MemoCache)
       -> vLLM (graph-aware scheduling + KV-cache pinning + pipelining)
       -> ACP agents (Claude Code / Codex / Gemini) for complex coding tasks
  -> Results + Unified Trace
```

**Benchmark suite**:

| Scenario | Baseline | Target | Measurement |
|----------|----------|--------|-------------|
| echo flow (2 agents) | Sequential (1x) | Parallel (2x+) | Wall clock time |
| review flow (1 LLM call) | Full agent spawn | Direct ASK | Latency reduction |
| 5-node pipeline | Flat context | Spaghetti-stack | Token reduction (target: 34%) |
| Repeated invocations | Fresh each time | Memoized | Cache hit rate, latency |
| Cost-optimized flow | Top-tier models | Budget models | Cost per execution |

**Deliverables**:
- [ ] ContextStack with spaghetti-stack organization
- [ ] MemoCache (session + persistent) with speculation
- [ ] Token pipelining between adjacent LLM nodes
- [ ] 5 optimization targets with compiler pass mapping
- [ ] 4+ new compiler passes (ModelDowngrade, ContextBudget, ParallelismExtraction, SpeculationInsertion)
- [ ] `--target` CLI flag
- [ ] End-to-end integration test
- [ ] Benchmark suite across all optimization targets

---

## Timeline Summary

```
        Week 1   2   3   4   5   6   7   8   9  10  11  12  13  14  15  16
        ─────────────────────────────────────────────────────────────────────
Phase 1 ████████████████
        Model Router + Health + CLI

Phase 2              ████████████████
                     ACP Client + Agents + Testing

Phase 3                          ████████████████████████████
                                 vLLM Graph-Aware Extension

Phase 4                                                  ████████████████████
                                                         Patent + Opt Targets

Key:    ████ = Active development
```

**Parallelism**: Phases 1 and 2 can proceed in parallel (different developers). Phase 3 depends on Phase 1 (model router provides the vLLM backend interface). Phase 4 depends on Phase 3 (pipelining requires vLLM streaming) and Phase 2 (ACP agents used in integration tests).

---

## Risk Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| vLLM extension rejected upstream | Medium | High | Fork with minimal patches; contribute back |
| ACP protocol changes | Low | Medium | Pin protocol version; abstract transport layer |
| Model routing adds latency | Low | Medium | Cache health checks; async monitoring thread |
| Speculation accuracy too low | Medium | Medium | Start conservative (memo-only); tune thresholds |
| Context stack adds complexity | Low | High | Extensive testing; fallback to flat context |
| New compiler passes regress existing behavior | Low | High | Existing tests + per-pass metrics + PGO validation |

---

## What Changed From v1

| Aspect | Plan v1 | Plan v2 (this document) |
|--------|---------|------------------------|
| Phase 2 | ACPX-to-APXM Graph Compiler (4 weeks, TypeScript bridge) | ACP Client in APXM (3 weeks, ~800 lines Rust) |
| Phase 4 | Spaghetti-stack only | Patent impl + Optimization Targets |
| Total timeline | 16 weeks | 16 weeks (saved 1 week from Phase 2, added to Phase 4) |
| ACPX dependency | Maintained as separate system | Eliminated -- absorbed into APXM |
| Optimization | Fixed pass pipeline per -O level | Goal-directed targets (`--target tokens/parallel/latency/cost`) |
| New code | ~800 ACP + ~5K bridge TS | ~800 ACP + ~2K compiler passes (all Rust/C++) |
| Code eliminated | 0 | ~30K lines TypeScript (entire ACPX orchestration) |
