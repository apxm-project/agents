# APXM Infrastructure Analysis: OpenMultiAgent Comparison & Enhancement Proposal

**Date**: 2026-04-07
**Method**: 18 parallel exploration agents (10 initial + 8 verification)
**Scope**: Full codebase audit of 15 crates, MLIR passes, server, and all 41 AIS operations

---

## Executive Summary

APXM's infrastructure is **significantly more sophisticated** than the OpenMultiAgent architecture in most areas. The scheduler, LLM adapter, compiler pipeline, and nested execution model are production-grade. However, 8 specific gaps exist — ranging from critical (streaming resilience) to architectural (cross-execution agent pooling). This document presents verified findings with exact file locations and concrete enhancement proposals.

---

## Part 1: Capability Mapping — APXM vs OpenMultiAgent

### What APXM Already Exceeds

| OpenMultiAgent | APXM Equivalent | Assessment |
|---|---|---|
| **TaskQueue** (dependency graph, auto unblock, cascade failure) | 4-level priority queue + crossbeam work-stealing (3-tier strategy) + token-based O(1) cascading unblock + deadlock watchdog + cascade failure with fallback values + live DAG splicing | **Far exceeds** — APXM's scheduler is a research-grade dataflow engine |
| **LLMAdapter** (Anthropic, OpenAI, Copilot) | LLMRegistry with 5 providers + operation-specific routing + model aliases + fallback chains + health monitoring + rate limiting + circuit breaker (ModelRouter) + vLLM graph-aware KV-cache pinning | **Far exceeds** — graph-aware LLM scheduling is unique |
| **AgentRunner** (conversation loop, tool dispatch) | ASK tool loop (10 iterations, configurable) + parallel tool dispatch with per-tool write-locks + AUTONOMOUS op (goal-directed multi-turn) + STM result storage + cancellation propagation | **Exceeds** — parallel tool dispatch with concurrency control |
| **ToolRegistry** (defineTool, 5 built-in) | CapabilityRegistry (DashMap) + 4 built-in tools + user tools from JSON + REGISTER_CAPABILITY op + approval/interceptor system + JSON Schema validation | **Exceeds** — runtime registration + approval workflow |
| **Team** (AgentConfig[], MessageBus, TaskQueue, SharedMemory) | TeamRegistry (teams.toml) + SPAWN_TEAM op + CLAIM op (distributed task claiming with leases) + NEGOTIATE op (multi-round consensus) + DELEGATE op | **Exceeds** — CLAIM with lease-based work-stealing is production-grade |

### What OpenMultiAgent Has That APXM Lacks

| OpenMultiAgent | APXM Status | Gap Severity |
|---|---|---|
| **AgentPool** (Semaphore, runParallel) | ProcessTable has limits (max=32, depth=4) but no cross-execution session reuse | Medium |
| **MessageBus** (pub/sub) | EventBus exists (tokio broadcast) but is completely orphaned — never instantiated outside tests | Low (workaround: COMMUNICATE broadcast) |
| **SharedMemory** | Memory is physically shared (Arc<MemorySystem>) but logically scoped by scope_id — no opt-in shared namespace | Low (workaround: episodic is global) |

### What APXM Has That OpenMultiAgent Lacks

- **MLIR Compiler Pipeline**: O0-O3 optimization (FuseAskOps, CondenseOps, CapabilityScheduling, constant folding, prompt caching, memoization hints)
- **Live DAG Splicing**: PLAN generates sub-graphs that are spliced into the running scheduler — no separate workflow launch
- **Recursive ExecutorEngine**: FLOW_CALL, DELEGATE, COMMUNICATE(local) all create nested ExecutorEngine instances with scoped AAM
- **SecurityManifest**: Compiler generates per-graph isolation requirements (7 levels: None → Remote, 4 tiers: T0-T3)
- **Session Tracing**: Atomic live.json updates, per-node workspaces, NDJSON event streams, replay support
- **Profile-Guided Optimization**: Infrastructure for feeding execution profiles back into compiler passes

---

## Part 2: Verified Gaps & Enhancements (Ranked by Impact)

### GAP 1: Streaming Resilience — No Fallback or Retry for Streams

**CRITICAL — Affects production reliability**

**Verified Location**: `crates/runtime/apxm-backends/src/llm/registry/mod.rs:395-419`

**Finding**: `generate()` (non-streaming) has full fallback chain + retry logic. `generate_stream()` resolves a single backend and fails hard if it's unhealthy. No retry, no fallback.

**Root Cause**: Streaming returns `Pin<Box<dyn Stream + '_>>` which is self-referential — can't retry mid-stream.

**Impact**: Any streaming LLM call (used for token-by-token emission in session tracing) fails permanently on first backend error.

**Enhancement**:
```
File: crates/runtime/apxm-backends/src/llm/registry/mod.rs

Add: generate_stream_with_fallback() that:
1. Tries primary backend stream
2. If first chunk errors, drops stream and retries with fallback backend
3. Once first chunk succeeds, commits to that backend (no mid-stream switching)
4. Uses RetryStrategy for initial connection errors only
```

**Effort**: Low (1 function, ~50 lines)
**ROI**: High — prevents silent failures in production streaming

---

### GAP 2: Compiler Never Sets Node Priority — Scheduler Underutilized

**HIGH — Working infrastructure goes unused**

**Verified**: Runtime priority pipeline is FULLY WORKING:
- `scheduler/state.rs:147-152`: Reads `node.metadata.priority`, maps to 4-level enum
- `scheduler/queue.rs`: 4-level priority queue (Critical/High/Normal/Low)
- `scheduler/work_stealing.rs:72-96`: Steals Critical first, then High, Normal, Low
- `scheduler/state.rs:384-419`: AAM goal priority boost (takes max of compile-time and goal priority)

**But the compiler never sets it**:
- `ArtifactEmitter.cpp:582-657`: `emitNode()` leaves `metadata.priority = 0` for ALL nodes
- `graph/optimize.rs:116-281`: `parallelism_analysis()` computes critical path but stores in graph metadata, NOT per-node priority
- `AISOps.td`: No priority attribute on any MLIR op

**Result**: All nodes arrive at scheduler with priority=0 → everything goes to Normal queue → work-stealing degenerates to FIFO.

**Enhancement**:
```
Phase A — Graph-level (Rust, crates/apxm-graph/src/optimize.rs):
  parallelism_analysis() already computes per-node longest_path.
  Add: set node.metadata.priority based on:
    - Critical path nodes → priority 90 (Critical)
    - High fan-out nodes → priority 70 (High)
    - Low-latency I/O nodes → priority 50 (Normal, but ahead of peers)
    - Default → priority 30 (Normal)

Phase B — MLIR pass (crates/compiler/apxm-compiler/mlir/):
  CapabilityScheduling.cpp already classifies tiers.
  Add: ais.priority attribute extraction in ArtifactEmitter.cpp:emitNode()
```

**Effort**: Low-Medium (Phase A: ~30 lines in optimize.rs; Phase B: ~20 lines in ArtifactEmitter.cpp)
**ROI**: High — 10-30% throughput improvement for complex graphs with mixed-latency operations

---

### GAP 3: Sandbox Bypass in Capability Execution

**HIGH — Security gap**

**Verified**: The CapabilitySystem routing IS wired (`capability/mod.rs:304-353`):
- `to_exec_request()` → `select_for_request()` → `backend.execute()` path EXISTS
- EXC operation FULLY uses sandbox via `executor/handlers/exc.rs:50-143`

**But built-in capabilities bypass it**:
- `BashCapability` (`apxm-tools/src/bash.rs:238`): Returns `Some(ExecRequest)` from `to_exec_request()` but `execute()` spawns directly via `tokio::process::Command`
- `UserToolCapability` (`driver/runtime/capabilities.rs:98-111`): Returns `Some(ExecRequest)` but `execute()` creates `ProcessSandbox::new()` directly, bypassing `SandboxRegistry` selection

**The capabilities say "I need sandboxing" but then sandbox themselves with a hardcoded ProcessSandbox, ignoring the registry's selected backend.**

**Enhancement**:
```
Option A (recommended, documented in sandbox_integration.rs:600-622):
  Create SandboxedCapabilityExecutor wrapper that:
  1. Checks to_exec_request()
  2. If Some: routes through SandboxRegistry.select_for_request()
  3. If None: calls execute() directly

Option B (simpler):
  Modify CapabilitySystem.invoke_with_timeout() to SKIP calling
  capability.execute() when to_exec_request() returns Some and
  sandbox backend is available — use ONLY backend.execute().
```

**Effort**: Low (Option B: ~15 lines changed in capability/mod.rs)
**ROI**: High — closes security gap for all INV operations

---

### GAP 4: Cross-Execution Agent Pool

**MEDIUM — Latency optimization**

**Verified**: Within a single execution, agents ARE effectively reused:
- SPAWN_AGENT creates once → ProcessTable registers
- Multiple COMMUNICATE(acp) calls reuse the same AcpSession (turn_count increments)
- This is correct and well-designed

**The gap is cross-execution**:
- Each `Runtime::new()` creates a fresh ProcessTable
- No cleanup: ProcessTable never calls `close_all()` at end of execution (agents leak)
- No warm pool: agents can't be pre-spawned and reused across graph executions
- Cold start per agent: 2-10s (subprocess + ACP init + auth + session/new + preamble)

**Enhancement**:
```
Phase A — Fix cleanup (bug fix):
  Add Runtime::shutdown() or execution-end hook that calls
  process_table.close_all() to gracefully terminate ACP sessions.
  File: crates/runtime/apxm-runtime/src/runtime.rs

Phase B — Warm pool (optimization):
  New struct AgentPool in crates/runtime/apxm-runtime/src/agent_pool.rs:
  - pools: DashMap<String, Vec<PooledSession>>  // profile_name → warm sessions
  - acquire(profile, aam_context) → warm session or spawn new
  - release(session) → return to pool (inject new preamble to reset context)
  - idle_timeout: Duration (reclaim after inactivity)
  - Pre-warm hint: compiler emits ais.warm_up_count from graph analysis

Phase C — Compiler hint:
  optimize.rs: Count unique SPAWN_AGENT profiles in graph → emit warm_up_count
  Runtime reads hint and pre-spawns agents before execution starts
```

**Effort**: Phase A: Low (cleanup bug); Phase B: Medium; Phase C: Low
**ROI**: Medium-High — 2-10s saved per agent spawn in multi-agent graphs

---

### GAP 5: LLM Rate Limiting Per-Request Not Per-Token

**MEDIUM — Affects cost control at scale**

**Verified**: `crates/runtime/apxm-backends/src/llm/rate_limit.rs:163`: Always consumes exactly 1.0 token per request regardless of actual token count.

**Enhancement**:
```
Modify check_and_consume() to accept token_count parameter:
  pub fn check_and_consume(&self, backend: &str, cost: f64) -> Result<()>

Callers pass estimated cost:
  - Pre-request: estimate from max_tokens
  - Post-request: reconcile with actual usage from LLMResponse.usage

Add RateLimitConfig::token_based: bool flag to distinguish per-request vs per-token.
```

**Effort**: Low (~20 lines)
**ROI**: Medium — important for cost control with expensive models

---

### GAP 6: RoundRobin Routing Strategy is a Stub

**MEDIUM — Advertised feature doesn't work**

**Verified**: `crates/runtime/apxm-backends/src/llm/registry/resolver.rs:164-170`:
```rust
fn select_round_robin(...) -> Result<String> {
    select_first_healthy(...)  // Just calls FirstHealthy!
}
```

**Enhancement**:
```
Add Arc<AtomicUsize> counter to LLMRegistry:
  fn select_round_robin(backends, health, counter) -> String {
    let healthy: Vec<_> = backends.filter(healthy);
    let idx = counter.fetch_add(1, Relaxed) % healthy.len();
    healthy[idx]
  }
```

**Effort**: Very low (~10 lines)
**ROI**: Medium — enables actual load distribution

---

### GAP 7: Shared Memory Namespace for Cross-Agent State

**LOW — Design enhancement**

**Verified**: Memory IS physically shared (same `Arc<MemorySystem>`). All agents in an execution share the same backend. Isolation is via `scope_id` prefix: `__scope__/{uuid}/{key}`.

Episodic memory is already global (no scoping applied).

**Enhancement**:
```
Add well-known scope_id "shared" to MemorySystem:
  const SHARED_SCOPE: &str = "__shared__";

Extend QMEM/UMEM with space: "shared" attribute:
  {"op": "UMEM", "attributes": {"space": "shared", "key": "task_board", ...}}

Handler routes to SHARED_SCOPE instead of ctx.scope_id():
  let scope = if space_attr == "shared" { SHARED_SCOPE } else { ctx.scope_id() };
  ctx.memory.write_scoped(space, scope, key, value).await;
```

**Effort**: Very low (~15 lines in qmem.rs + umem.rs)
**ROI**: Low — enables blackboard pattern but COMMUNICATE is usually sufficient

---

### GAP 8: EventBus Integration

**LOW — Orphaned infrastructure**

**Verified**: EventBus is complete and tested (5 unit tests pass) but NEVER instantiated in production code. Zero imports outside `apxm-events/src/bus.rs` and tests.

`ExecutionEventEmitter` trait (4 implementations) is the actual production mechanism.

**Enhancement (if needed)**:
```
Wire EventBus as additional fan-out inside SessionEventEmitter:
  SessionEventEmitter::new() creates EventBus internally
  Each emit_*() call publishes to EventBus in addition to file writes
  External consumers (CLI progress bar, webhook notifier) subscribe

OR: Remove EventBus entirely if multi-subscriber fan-out isn't needed.
```

**Effort**: Low (wire) or Very Low (remove)
**ROI**: Low — ExecutionEventEmitter already handles all current use cases

---

## Part 3: Architecture Comparison Summary

```
                    OpenMultiAgent                          APXM
                    ──────────────                          ────

Orchestrator        createTeam/runTeam/runTasks        Runtime + DataflowScheduler +
                    (simple API)                       live DAG splicing (PLAN)

Team                AgentConfig[] + MessageBus         TeamRegistry + SPAWN_TEAM +
                    (pub/sub channels)                 NEGOTIATE (multi-round consensus)

AgentPool           Semaphore + runParallel()          ProcessTable (max=32, depth=4) +
                    (warm pool)                        ConcurrencyControl semaphore
                                                      (no warm pool across executions)

TaskQueue           dependency graph +                 4-level PriorityQueue +
                    auto unblock +                     crossbeam work-stealing (3-tier) +
                    cascade failure                    token-based O(1) unblock +
                                                      deadlock watchdog +
                                                      cascade failure + fallback values

Agent               run/prompt/stream                  AcpSession (ACP protocol) +
                    (simple lifecycle)                  15 built-in profiles +
                                                      AAM context projection +
                                                      reverse request handling

LLMAdapter          Anthropic/OpenAI/Copilot           5 providers + LLMRegistry routing +
                    (simple dispatch)                   fallback chains + circuit breaker +
                                                      health monitor + rate limiter +
                                                      vLLM graph-aware KV-cache pinning

AgentRunner         conversation loop +                ASK tool loop (10 iter, parallel dispatch) +
                    tool dispatch                      AUTONOMOUS (goal-directed) +
                                                      per-tool write-locking +
                                                      STM result storage

ToolRegistry        defineTool + 5 built-in            CapabilityRegistry + 4 built-in +
                    (simple registry)                   user tools + REGISTER_CAPABILITY +
                                                      approval/interceptor + JSON Schema

SharedMemory        global shared state                Physically shared Arc<MemorySystem> +
                    (simple dict)                      logical scoping via scope_id prefix +
                                                      global episodic memory

Compiler            N/A                                MLIR pipeline (O0-O3, 93 passes) +
                                                      FuseAskOps (100-400x ROI) +
                                                      SecurityManifest + artifact format

Nesting             runTasks (fork+join)               Recursive ExecutorEngine +
                                                      live DAG splicing +
                                                      3 patterns: FLOW_CALL/DELEGATE/COMMUNICATE
```

---

## Part 4: Recommended Implementation Order

### Sprint 1 (Quick Wins — High ROI, Low Effort)

1. **Wire compiler priority → scheduler** — `optimize.rs` parallelism_analysis sets per-node priority. ~30 lines.
2. **Fix RoundRobin stub** — `resolver.rs` add AtomicUsize counter. ~10 lines.
3. **Add streaming fallback** — `registry/mod.rs` retry on first-chunk error. ~50 lines.
4. **Fix ProcessTable cleanup** — Add `Runtime::shutdown()` that closes all agents. ~20 lines.

### Sprint 2 (Security & Correctness)

5. **Fix sandbox bypass** — Modify `capability/mod.rs` to skip `execute()` when sandbox backend selected. ~15 lines.
6. **Token-aware rate limiting** — Add cost parameter to `check_and_consume()`. ~20 lines.
7. **Sync health monitors** — Bridge backends health → runtime circuit breaker. ~40 lines.

### Sprint 3 (Architecture)

8. **Shared memory namespace** — Add `SHARED_SCOPE` to QMEM/UMEM handlers. ~15 lines.
9. **Cross-execution agent pool** — New `AgentPool` struct with warm session management. ~200 lines.
10. **Compiler warm-up hints** — Count SPAWN_AGENT profiles, emit pre-warm count. ~30 lines.

### Backlog (Nice-to-Have)

11. **Wire StreamAssembler into ASK tool loop** — Enable streaming tool calls.
12. **Wire EventBus or remove** — Decide on multi-subscriber fan-out.
13. **MLIR AgentPoolingPass** — Detect identical agent spawns at compile time.
14. **Proactive health polling** — Background task per backend.
15. **HTTP client timeout configuration** — reqwest builder with send/read timeouts.

---

## Part 5: Key File Reference

| Component | Primary File | Lines |
|---|---|---|
| Scheduler priority pipeline | `crates/runtime/apxm-runtime/src/scheduler/state.rs` | 147-152, 384-419 |
| Priority queue | `crates/runtime/apxm-runtime/src/scheduler/queue.rs` | 11-46 |
| Work stealing | `crates/runtime/apxm-runtime/src/scheduler/work_stealing.rs` | 56-120 |
| Parallelism analysis | `crates/apxm-graph/src/optimize.rs` | 116-281 |
| Artifact emitter (C++) | `crates/compiler/apxm-compiler/mlir/.../ArtifactEmitter.cpp` | 582-657 |
| LLM fallback chain | `crates/runtime/apxm-backends/src/llm/registry/mod.rs` | 285-330 |
| Streaming (no fallback) | `crates/runtime/apxm-backends/src/llm/registry/mod.rs` | 395-419 |
| Rate limiter | `crates/runtime/apxm-backends/src/llm/rate_limit.rs` | 163 |
| RoundRobin stub | `crates/runtime/apxm-backends/src/llm/registry/resolver.rs` | 164-170 |
| Circuit breaker | `crates/runtime/apxm-runtime/src/model_router/health.rs` | 24-122 |
| Sandbox routing | `crates/runtime/apxm-runtime/src/capability/mod.rs` | 304-353 |
| BashCapability bypass | `crates/apxm-tools/src/bash.rs` | 238, 281-283 |
| ProcessTable | `crates/runtime/apxm-runtime/src/process_table.rs` | 50-228 |
| Agent spawn lifecycle | `crates/orchestration/apxm-acp/src/session.rs` | 51-191 |
| ASK tool loop | `crates/runtime/apxm-runtime/src/executor/handlers/llm.rs` | 806-974 |
| Parallel tool dispatch | `crates/runtime/apxm-runtime/src/executor/handlers/llm.rs` | 310-347 |
| EventBus (unused) | `crates/apxm-events/src/bus.rs` | 1-96 |
| Memory scoping | `crates/runtime/apxm-runtime/src/memory/mod.rs` | 88-102, 186-211 |
| FlowRegistry | `crates/runtime/apxm-runtime/src/capability/flow_registry.rs` | — |
| DAG splicing | `crates/runtime/apxm-runtime/src/scheduler/splicing.rs` | — |
| TeamRegistry | `crates/runtime/apxm-runtime/src/team/registry.rs` | — |
