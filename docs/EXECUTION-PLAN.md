# Execution Plan

**Cross-Project Implementation Plan: APXM + ACPX + vLLM**
**Date**: March 31, 2026

---

## Current State Audit

### APXM (Rust — 17 crates)

| Component | Status | Location |
|-----------|--------|----------|
| Graph IR (ApxmGraph) | DONE | `apxm-core/src/types/graph.rs` |
| AIS operations (39 ops, 17 typed) | DONE | `apxm-core/src/types/operations.rs` |
| Compiler (4-stage MLIR) | DONE | `apxm-compiler/` |
| FuseAskOps pass | DONE | `apxm-compiler/mlir/` |
| CSE pass | DONE | `apxm-compiler/mlir/` |
| DCE pass | DONE | `apxm-compiler/mlir/` |
| DataflowScheduler | DONE | `apxm-runtime/src/scheduler/` |
| ExecutorEngine (parallel + sequential) | DONE | `apxm-runtime/src/executor/engine.rs` |
| ResponseCache (memoization) | DONE | `apxm-runtime/src/executor/memoization.rs` |
| 3-tier memory (STM/LTM/Episodic) | DONE | `apxm-runtime/src/memory/` |
| LLM backends (OpenAI, Anthropic, Ollama) | DONE | `apxm-backends/src/` |
| Sandbox architecture | DONE | `apxm-sandbox/` |
| CLI (compile, run, init, doctor) | DONE | `apxm-cli/src/main.rs` |
| Credential store | DONE | `apxm-credentials/` |
| MCP integration | DONE | `apxm-runtime/src/apxm_mcp.rs` |
| **ModelRouter** | **NOT STARTED** | planned: `apxm-runtime/src/model_router/` |
| **AcpCapability** | **NOT STARTED** | planned: `apxm-runtime/src/capability/acp.rs` |
| **ContextStack (spaghetti)** | **NOT STARTED** | planned: `apxm-runtime/src/context_stack/` |
| **GraphAwareVllmBackend** | **NOT STARTED** | planned: `apxm-backends/src/vllm_graph_aware.rs` |
| **Optimization targets** | **NOT STARTED** | planned: `apxm-compiler/src/passes/targets.rs` |
| **ModelDowngrade pass** | **NOT STARTED** | planned: MLIR pass |
| **ContextBudget pass** | **NOT STARTED** | planned: MLIR pass |
| **ParallelismExtraction pass** | **NOT STARTED** | planned: MLIR pass |
| **SpeculationInsertion pass** | **NOT STARTED** | planned: MLIR pass |
| **PipelineInsertion pass** | **NOT STARTED** | planned: MLIR pass |

### ACPX (TypeScript — v0.4.0)

| Component | Status | Notes |
|-----------|--------|-------|
| ACP protocol client | DONE | `@agentclientprotocol/sdk ^0.17.0` |
| Session management | DONE | Persistent, named, parallel sessions |
| Agent profiles (17 agents) | DONE | Claude, Codex, Gemini, etc. |
| Queue IPC (prompt queueing) | DONE | `src/queue-ipc.ts` |
| Flows runtime | DONE | Sequential only (`while (current)` loop) |
| Flow node types (acp, compute, shell, checkpoint) | DONE | `src/flows/` |
| CLI (40+ commands) | DONE | Commander-based |
| `session/set_model` | NOT YET | Unstable in ACP spec |
| `session/fork` | NOT YET | Unstable in ACP spec |
| Multi-agent orchestration | NOT YET | Agent A prompts Agent B |
| Flow parallel execution | NOT POSSIBLE | Architecture limitation |
| Cost/token tracking | NOT YET | Needs ACP spec support |

**ACPX future**: Gets absorbed into APXM as `AcpCapability` (~800 lines Rust). ACPX continues to work standalone for flow authoring until APXM can fully replace it.

### vLLM (Python — upstream fork)

| Component | Status | Notes |
|-----------|--------|-------|
| Upstream vLLM | SYNCED | Latest upstream commits (Mar 31) |
| Custom code | NONE | Vanilla fork, no custom scheduler |
| GraphAwareScheduler | NOT STARTED | planned: `vllm/extensions/graph_aware_scheduler.py` |
| KV-cache pinning | NOT STARTED | Standard PagedAttention only |
| Graph metadata REST API | NOT STARTED | planned: `POST /v1/graphs/register` |
| Priority scheduling | NOT STARTED | No APXM-aware prioritization |
| Eager prefill | NOT STARTED | No graph-driven prefill |

---

## Execution Plan (16 weeks, 4 phases)

### Phase 1: Model Router (Weeks 1-3) — APXM

Dynamic model selection at runtime instead of compile time.

| Week | Task | Project | Files | Deliverable |
|------|------|---------|-------|-------------|
| 1 | Model Registry + Health Monitor | apxm | `apxm-runtime/src/model_router/mod.rs` | ModelRouter struct |
| 1 | Health monitoring | apxm | `apxm-runtime/src/model_router/health.rs` | Circuit breakers |
| 1 | Model registry | apxm | `apxm-runtime/src/model_router/registry.rs` | `~/.apxm/models.toml` |
| 2 | Runtime integration | apxm | `apxm-runtime/src/executor/handlers/llm.rs` | ModelRouter in dispatch path |
| 2 | Context integration | apxm | `apxm-runtime/src/context.rs` | ModelRouter in ExecutionContext |
| 2 | Backend health API | apxm | `apxm-backends/src/lib.rs` | `health_check()` on LLMBackend trait |
| 3 | CLI commands | apxm | `apxm-cli/src/commands/models.rs` | `apxm models list`, `apxm models health` |
| 3 | Unit tests | apxm | `apxm-runtime/tests/model_router_tests.rs` | Routing + failover tests |

**Dependencies**: None — this is the starting point.
**Blocks**: Phase 3 (vLLM backend needs ModelRouter interface).

---

### Phase 2: ACP Client in APXM (Weeks 4-6) — APXM

Replace ACPX orchestration with ~800 lines of Rust inside APXM.

| Week | Task | Project | Files | Deliverable |
|------|------|---------|-------|-------------|
| 4 | AcpCapability core | apxm | `apxm-runtime/src/capability/acp.rs` | CapabilityExecutor impl |
| 4 | ACP protocol | apxm | `apxm-runtime/src/capability/acp/protocol.rs` | JSON-RPC 2.0 over stdio |
| 4 | Session lifecycle | apxm | `apxm-runtime/src/capability/acp/session.rs` | create, prompt, cancel |
| 4 | Agent registry | apxm | `apxm-runtime/src/capability/acp/registry.rs` | Profile -> spawn command |
| 5 | Configuration | apxm | `apxm-runtime/src/capability/acp/config.rs` | `~/.apxm/agents.toml` |
| 5 | Reverse requests | apxm | `apxm-runtime/src/capability/acp/reverse.rs` | File I/O, terminal ops |
| 5 | Model routing integration | apxm | capability + model_router | Profile-based policy |
| 6 | CLI commands | apxm | `apxm-cli/src/commands/agent.rs` | `apxm agent list/test/add` |
| 6 | Auto-register | apxm | `apxm-runtime/src/capability/mod.rs` | AcpCapability in capability registry |
| 6 | E2E test | apxm | tests/ | INV(acp) -> Claude Code -> code changes |

**Dependencies**: Can run in parallel with Phase 1 (different developers).
**Blocks**: Phase 4 (ACP agents used in integration tests).
**Builds on**: Existing `UserToolCapability` pattern, `apxm_mcp.rs` JSON-RPC, `ProcessSandbox`.

---

### Phase 3: Graph-Aware vLLM (Weeks 7-11) — APXM + vLLM

vLLM becomes aware of APXM execution graphs.

| Week | Task | Project | Files | Deliverable |
|------|------|---------|-------|-------------|
| 7 | Graph metadata protocol (Rust) | apxm | `apxm-backends/src/vllm_graph_aware.rs` | GraphAwareVllmBackend |
| 7 | Graph metadata structs | apxm | same | GraphMetadata, GraphHints |
| 8 | vLLM REST API extension | vllm | `vllm/entrypoints/openai/api_server.py` | `POST /v1/graphs/register` |
| 8 | Graph metadata storage | vllm | `vllm/v1/engine/` | Graph registry in EngineCore |
| 9 | Scheduler extension | vllm | `vllm/extensions/graph_aware_scheduler.py` | GraphAwareScheduler class |
| 9 | Priority scheduling | vllm | same | Critical-path prioritization |
| 10 | KV-cache pinning | vllm | `vllm/v1/core/` | Pin results for downstream nodes |
| 10 | Eager prefill | vllm | `vllm/v1/core/` | Prefill static contexts |
| 11 | Integration test | both | tests/ | APXM graph -> vLLM with hints |
| 11 | Benchmarks | both | benches/ | Latency with/without graph-awareness |
| 11 | ContextStack foundation | apxm | `apxm-runtime/src/context_stack/` | Data structure started |

**Dependencies**: Phase 1 (ModelRouter provides backend interface).
**Blocks**: Phase 4 (pipelining requires vLLM streaming).

---

### Phase 4: Patent + Optimization Targets (Weeks 12-16) — APXM + vLLM

Patent techniques + goal-directed compiler targets.

| Week | Task | Project | Files | Deliverable |
|------|------|---------|-------|-------------|
| 12 | ContextStack (spaghetti) | apxm | `apxm-runtime/src/context_stack/mod.rs` | ContextStack, ContextFrame |
| 12 | Context assembly | apxm | `apxm-runtime/src/context_stack/assembly.rs` | Bottom-up/top-down traversal |
| 12 | Demand paging | apxm | `apxm-runtime/src/context_stack/demand_paging.rs` | Lazy loading |
| 12 | MemoCache (two-tier) | apxm | `apxm-runtime/src/memo/mod.rs` | Session DashMap + SQLite |
| 13 | Speculative execution | apxm | `apxm-runtime/src/memo/speculation.rs` | Commit/rollback |
| 13 | Token pipelining | apxm | executor + vllm backend | Stream tokens into next node's prefill |
| 14 | OptimizationTarget enum | apxm | `apxm-compiler/src/passes/targets.rs` | `--target` flag |
| 14 | ModelDowngrade pass | apxm | `apxm-compiler/mlir/.../ModelDowngrade.cpp` | Replace expensive models |
| 14 | ContextBudget pass | apxm | `apxm-compiler/mlir/.../ContextBudget.cpp` | Per-node token budgets |
| 15 | ParallelismExtraction pass | apxm | `apxm-compiler/mlir/.../ParallelismExtraction.cpp` | Detect parallelizable subgraphs |
| 15 | SpeculationInsertion pass | apxm | `apxm-compiler/mlir/.../SpeculationInsertion.cpp` | Insert speculative edges |
| 15 | PipelineInsertion pass | apxm | `apxm-compiler/mlir/.../PipelineInsertion.cpp` | Mark pipelineable nodes |
| 16 | Target-aware pass pipeline | apxm | `apxm-compiler/src/passes/pipeline.rs` | Target-to-passes mapping |
| 16 | CLI --target flag | apxm | `apxm-cli/src/main.rs` | `apxm compile -O2 --target latency` |
| 16 | Full integration test | all | tests/ | End-to-end: compile -> run -> vLLM -> ACP |
| 16 | Benchmark suite | all | benches/ | All 5 optimization targets |

**Dependencies**: Phase 2 (ACP for integration tests), Phase 3 (vLLM streaming for pipelining).

---

## Dependency Graph

```
Phase 1 (Weeks 1-3)          Phase 2 (Weeks 4-6)
ModelRouter [apxm]            AcpCapability [apxm]
     │                              │
     │  (can run in parallel)       │
     │                              │
     v                              v
Phase 3 (Weeks 7-11)               │
GraphAware vLLM [apxm+vllm]        │
     │                              │
     └──────────┬───────────────────┘
                v
         Phase 4 (Weeks 12-16)
         Patent + Targets [apxm]
```

---

## Task Summary by Project

### APXM — 38 tasks

| Phase | Count | Type |
|-------|-------|------|
| 1: ModelRouter | 8 | New module + integration |
| 2: AcpCapability | 10 | New module + integration |
| 3: GraphAwareVllmBackend | 4 | New backend |
| 4: ContextStack | 3 | New module |
| 4: MemoCache two-tier | 2 | Extend existing |
| 4: Speculation + Pipelining | 2 | New runtime features |
| 4: Optimization Targets | 7 | Compiler passes + CLI |
| 4: Integration + Benchmarks | 2 | Testing |

### vLLM — 7 tasks

| Phase | Count | Type |
|-------|-------|------|
| 3: REST API extension | 2 | New endpoint |
| 3: GraphAwareScheduler | 2 | New scheduler class |
| 3: KV-cache pinning + prefill | 2 | Core modifications |
| 3: Benchmarks | 1 | Testing |

### ACPX — 0 new tasks

ACPX is feature-complete for its current role. It continues as-is until APXM's AcpCapability (Phase 2) can replace it. Then:

1. `acpx flow run` flows keep working — they spawn agents the same way
2. APXM gains ability to dispatch to the same agents via `INV(acp)`
3. New workflows get written as APXM graphs instead of `.flow.ts`
4. ACPX remains available as a lightweight CLI for ad-hoc agent interaction

---

## Optimization Targets (Phase 4 detail)

```
$ apxm compile graph.json -O2 --target tokens      # Minimize token usage
$ apxm compile graph.json -O2 --target parallel     # Maximize parallelism
$ apxm compile graph.json -O2 --target latency      # Minimize end-to-end time
$ apxm compile graph.json -O2 --target cost         # Minimize API costs
$ apxm compile graph.json -O2 --target balanced      # Default: good all-around
```

| Target | Key Passes | Runtime Effect |
|--------|-----------|----------------|
| tokens | ContextBudget, DeadContextElim, FuseAskOps(aggressive) | Stage-specific loading, context manifests |
| parallel | ParallelismExtraction, SpeculationInsertion | Max concurrency, speculation enabled |
| latency | PipelineInsertion, SpeculationInsertion, FuseAskOps | Token pipelining + prefill + speculation |
| cost | ModelDowngrade, FuseAskOps, CondenseOps | Router prefers cheap models |
| balanced | All standard passes at moderate settings | Default behavior |

---

## Benchmarks (Phase 4, Week 16)

| Scenario | Baseline | Target | Metric |
|----------|----------|--------|--------|
| Echo flow (2 agents) | Sequential (1x) | Parallel (2x+) | Wall clock time |
| Review flow (1 LLM) | Full agent spawn | Direct ASK | Latency reduction |
| 5-node pipeline | Flat context | Spaghetti-stack | Token reduction (34%) |
| Repeated invocations | Fresh each time | Memoized | Cache hit rate |
| Cost-optimized flow | Top-tier models | Budget models | Cost per execution |

---

## Risk Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| vLLM extension rejected upstream | High | Fork with minimal patches; contribute back |
| ACP protocol changes | Medium | Pin protocol version; abstract transport |
| Model routing adds latency | Medium | Cache health checks; async monitoring |
| Speculation accuracy too low | Medium | Start conservative (memo-only); tune |
| Context stack adds complexity | High | Extensive testing; fallback to flat context |
| New compiler passes regress | High | Existing tests + per-pass metrics |

---

## What This Enables

After 16 weeks, the system supports:

```
User Request (natural language)
  -> Graph Generation [future: P3]
  -> apxm compile -O2 --target latency
       MLIR passes: normalize, fuse-ask-ops, parallelism-extraction,
                    speculation-insertion, pipeline-insertion, cse, dce
  -> apxm run (DataflowScheduler + ModelRouter + ContextStack + MemoCache)
       -> vLLM (graph-aware scheduling + KV-cache pinning + pipelining)
       -> ACP agents (Claude Code / Codex / Gemini) for complex coding
  -> Results + Unified Trace
```

No separate orchestration layer. APXM is the compiler, runtime, and orchestrator.
