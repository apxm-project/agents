# Investigation: APXM Sessions, State Isolation, and apxm-server Architecture

**Date:** 2026-03-20
**Scope:** How APXM handles sessions, multi-consumer state isolation, and whether `apxm-server` belongs in the migration critical path

---

## 1. The Session and State Isolation Model (Current)

### 1.1 Runtime is a Singleton

`apxm-runtime/src/runtime.rs:77-88` -- The `Runtime` struct holds:

```rust
pub struct Runtime {
    config: RuntimeConfig,
    memory: Arc<MemorySystem>,         // SHARED across all executions
    llm_registry: Arc<LLMRegistry>,    // SHARED across all executions
    capability_system: Arc<CapabilitySystem>, // SHARED across all executions
    flow_registry: Arc<FlowRegistry>,  // SHARED across all executions
    aam: Aam,                          // SHARED across all executions (!)
    scheduler: DataflowScheduler,
    session_lane_guard: SessionLaneGuard,
    ...
}
```

**Critical finding:** The `Aam` (Agent Abstract Machine state) is shared across ALL executions. When `apxm-server` creates one `Runtime`, every HTTP request reads/writes the **same** beliefs, goals, and capabilities.

### 1.2 ExecutionContext Creates Per-Execution Scoping

`apxm-runtime/src/executor/context.rs:35-78` -- Each execution gets its own `ExecutionContext`:

```rust
pub struct ExecutionContext {
    pub execution_id: String,        // Unique per execution (UUID v7)
    pub session_id: Option<String>,  // Optional, set by caller
    pub aam: Aam,                    // Cloned from Runtime.aam (Arc<RwLock<AamState>>)
    pub scope_id: String,            // Unique per execution (UUID v7)
    pub scope_registry: Arc<ScopeRegistry>,  // New per execution
    ...
}
```

**However:** `build_context()` at `runtime.rs:135-154` clones the Runtime's AAM:

```rust
fn build_context(&self, ...) -> ExecutionContext {
    let mut ctx = ExecutionContext::new(
        Arc::clone(&self.memory),        // same memory
        Arc::clone(&self.llm_registry),  // same LLM registry
        Arc::clone(&self.capability_system), // same capabilities
        self.aam.clone(),                // CLONE of AAM -- but Aam is Arc<RwLock<AamState>>!
    );
    ...
}
```

Since `Aam` is `Arc<RwLock<AamState>>`, cloning it gives a **shared reference**, not a copy. All executions write to the **same** underlying `AamState`. Beliefs set by one execution are visible to all concurrent executions.

### 1.3 SessionLaneGuard Serializes Same-Session Requests

`apxm-runtime/src/scheduler/lane_queue.rs` -- The `SessionLaneGuard` ensures:
- Requests with the **same** `session_id` execute serially (mutex per session)
- Requests with **different** `session_id`s run concurrently

This prevents race conditions within a session but does NOT isolate state between sessions. Two different sessions still share the same AAM.

### 1.4 Child Scoping EXISTS but Is Not Used for Session Isolation

`context.rs:208-218` -- `child_with_scope()` creates a child AAM with configurable isolation:

```rust
pub fn child_with_scope(&self, scope: ScopeSpec) -> Self {
    let child_aam = self.aam.child_scope(&scope);
    // Supports: Inherit, Isolate, Snapshot, Filter
}
```

The scoping infrastructure is fully implemented (`aam/mod.rs:295-347`, 6 tests, `ScopePolicy` with 4 variants). But `build_context()` does NOT use it -- it always passes the root AAM. There is no per-session AAM isolation in the current server.

### 1.5 Session Checkpointing EXISTS

`aam/session.rs` -- `SessionManager` provides checkpoint persistence:
- `save_checkpoint(session_id, checkpoint)` -- serialize AAM state to `<dir>/<session_id>.json`
- `load_checkpoint(session_id)` -- restore AAM state
- `list_sessions()`, `delete_checkpoint()`

This is checkpoint/restore, not live session isolation. A session can be saved and restored, but during execution, all sessions share the same AAM.

### 1.6 Memory System Is Globally Shared

`apxm-runtime/src/memory/mod.rs` -- STM, LTM, and Episodic memory are all `Arc`-shared:
- STM: `InMemoryBackend` -- single global key-value store
- LTM: `SqliteBackend` -- single SQLite database
- Episodic: Single episode log

The `read_scoped()`/`write_scoped()` methods use key-prefix namespacing (`scope_id::key`), providing soft isolation at the key level. But the underlying storage is shared.

### 1.7 Capabilities Are Globally Shared

`register_capability()` in `apxm-server/main.rs:1024-1059` registers capabilities into the shared `CapabilitySystem`. Once registered, a capability is visible to ALL subsequent executions. There is no per-session capability scoping.

---

## 2. Implications for Multi-Consumer Migration

### 2.1 The Problem: Shared AAM

If Codex and Gemini-CLI both use the same `apxm-server`:
- Codex sets belief `"user_intent" = "fix bug"` -- Gemini-CLI sees it
- Gemini-CLI sets goal `"generate code"` -- Codex's goal queue contains it
- Codex registers `ShellCapability` -- Gemini-CLI can invoke it

This is **by design** for multi-agent collaboration within ONE agent system. But it's a **problem** for independent consumers that should NOT share state.

### 2.2 The Solution: Per-Consumer Runtime Instances

**Option A: Separate Runtime per consumer (library embedding)**
- Codex creates its own `Runtime` with its own `Aam`, `MemorySystem`, `CapabilitySystem`
- Gemini-CLI creates its own `Runtime` with its own everything
- No state sharing. No coordination needed. Simplest model.
- This is what happens naturally when each consumer links APXM as a library.

**Option B: Per-session AAM scoping (server model)**
- `apxm-server` creates a fresh `Aam` per session (or per consumer)
- `build_context()` uses `child_with_scope(ScopeSpec::isolate())` instead of cloning the root AAM
- Requires changes to `Runtime` and `apxm-server`

**Option C: Multiple Runtime instances in one server**
- `apxm-server` maintains a `HashMap<ConsumerId, Runtime>`
- Each consumer gets its own isolated Runtime
- The `LLMRegistry` (credentials) can be shared; the AAM is per-consumer

### 2.3 Recommendation

**For the migration (Phases 1-4): Use Option A -- each consumer embeds its own Runtime.**

This is the simplest, safest approach. No state isolation bugs. No server architecture changes. Each consumer owns its own AAM, memory, and capabilities.

**For the universal substrate vision (Phase 4+): Evolve to Option C.**

When `apxm-server` becomes a shared gateway (multi-agent coordination, A2A protocol, MCP), it needs per-session or per-consumer Runtime isolation. The scoping infrastructure exists (`child_scope`, `ScopeSpec`, `ScopePolicy`) but needs to be wired into the server's request handling.

---

## 3. apxm-server: What It Actually Is

### 3.1 Current Capabilities (2,595 lines, fully functional)

The plans describe `apxm-server` as a "thin HTTP+SSE LLM proxy." This dramatically understates it.

**Actual endpoint inventory:**

| Group | Routes | Purpose |
|-------|--------|---------|
| Health | `GET /health` | Server status, uptime |
| Models | `GET /v1/models` | List available LLM backends |
| Execution | `POST /v1/execute`, `POST /v1/execute/stream` | Graph compilation + execution |
| Memory | `POST /v1/memory/facts/{store,search,delete}` | LTM fact management |
| Capabilities | `GET /v1/capabilities`, `POST /v1/capabilities/register` | Tool registry |
| Messaging | `POST /v1/receive` | COMMUNICATE operation target |
| Agents | `GET/POST/DELETE /v1/agents/*` | Multi-agent registry |
| Tasks | `POST /v1/tasks`, `GET/POST /v1/tasks/:queue/*` | CLAIM operation backend |
| Checkpoints | `POST /v1/checkpoints`, `GET/POST /v1/checkpoints/:id/*` | PAUSE/RESUME HITL |
| MCP | `POST /v1/mcp` | Full MCP 2025-11-05 JSON-RPC |
| A2A | `POST /a2a/*`, `GET /.well-known/agent.json` | Google A2A v0.3 protocol |

This is a **full agent runtime gateway** with protocol interop (MCP + A2A), not a proxy.

### 3.2 What It Does Per Request

`/v1/execute` flow:
1. Parse graph JSON from request body
2. Apply runtime attributes (token budget, output schema)
3. **Compile graph** to artifact via MLIR (O1 optimization level)
4. Execute artifact on the shared Runtime
5. Return results

Key: The server **compiles AND executes** graphs. It's not just proxying LLM calls -- it's running the full APXM pipeline (graph validation, MLIR compilation, dataflow scheduling, parallel execution).

### 3.3 What It Does NOT Do

- **No per-session AAM isolation** -- all requests share one AAM
- **No LLM call proxying** -- there is no `POST /v1/generate` or `POST /v1/generate-stream` endpoint. The server executes GRAPHS, not individual LLM calls.
- **No authentication/authorization** -- any client can register capabilities, execute graphs, access memory
- **No multi-tenant isolation** -- single Runtime, single AAM, single memory

### 3.4 The LLM Proxy Gap

The migration plans (Phase 1) describe Gemini-CLI sending LLM requests to `apxm-server`:
```
POST /v1/generate-stream { messages, model, temperature, ... }
→ SSE stream of tokens
```

**This endpoint does not exist.** The server only has `/v1/execute` (graph execution) and `/v1/execute/stream` (graph execution with SSE events). There is no way to make a single LLM call through the server without wrapping it in a graph.

This means Phase 1 as currently planned would require either:
- Adding a `POST /v1/generate-stream` endpoint to `apxm-server` (new work)
- OR having Gemini-CLI construct a single-ASK-node graph for every LLM call (awkward but works)

---

## 4. Revised Phase Architecture

### 4.1 The Key Insight

`apxm-server` is designed for **graph execution** (Phase 4+), not for **LLM call proxying** (Phase 1). Forcing it into Phase 1 requires either building new proxy endpoints or wrapping every LLM call in a graph.

### 4.2 Proposed Phase Restructure

```
Phase 1 (LLM Backend):
  Codex:      Cargo dependency → LLMRegistry::generate() directly
  Gemini-CLI: NAPI module OR subprocess bridge (NOT HTTP server)

Phase 2 (Tool Migration):
  Codex:      CapabilitySystem::register() directly (Rust API)
  Gemini-CLI: NAPI module for capability registration (NOT HTTP)

Phase 3 (AAM State):
  Codex:      Aam API directly (Rust)
  Gemini-CLI: NAPI module for AAM access, OR file-based ("file tree IS the AAM")

Phase 4 (Agent Loop as Graph):
  ALL:        apxm-server enters here ← this is where it makes sense
              POST /v1/execute { graph: turn-graph-JSON }
              One HTTP call per turn, not per LLM call
              SSE streaming of execution events

Phase 5 (Compiler Integration):
  ALL:        apxm-server compiles + executes
              POST /v1/execute { graph: ... } with O2 optimization
```

### 4.3 When apxm-server Adds Real Value

| Phase | Without Server | With Server | Server Value |
|-------|---------------|-------------|--------------|
| 1 (LLM) | Direct API or NAPI | Must add /v1/generate-stream | Negative (new work, no benefit) |
| 2 (Tools) | Direct Rust or NAPI | HTTP capability registration | Neutral (either works) |
| 3 (AAM) | Direct Rust or NAPI + file tree | HTTP state access | Negative (adds latency to state reads) |
| 4 (Graphs) | Subprocess `apxm execute` | POST /v1/execute + SSE | **Positive** (streaming, concurrent graphs) |
| 5 (Compiled) | Subprocess `apxm compile + run` | POST /v1/execute with compilation | **Positive** (integrated pipeline) |
| Multi-agent | N/A | Agent registry, COMMUNICATE, A2A | **Essential** (multi-agent coordination) |
| Ecosystem | N/A | MCP, A2A, any-language access | **Essential** (universal substrate) |

### 4.4 Session Isolation Work Needed

Before `apxm-server` can serve multiple consumers (Phase 4+), it needs:

1. **Per-session AAM** -- `build_context()` should create isolated AAM per session_id (using existing `child_scope(ScopeSpec::isolate())`)
2. **Per-session memory namespacing** -- Use `read_scoped()`/`write_scoped()` with session_id as scope prefix
3. **Per-consumer capability isolation** -- Optional: some capabilities are global (LLM backends), some are per-consumer (Codex shell vs Gemini-CLI tools)
4. **Session lifecycle** -- Create, checkpoint, restore, destroy sessions via API

The building blocks exist (ScopeSpec, ScopeRegistry, SessionManager, scoped memory ops). The wiring is missing.

---

## 5. NAPI vs Subprocess vs HTTP: Decision Matrix

### For Gemini-CLI (TypeScript consumer):

| Criterion | NAPI Module | Subprocess | HTTP Server |
|-----------|-------------|------------|-------------|
| Setup complexity | Medium (napi-rs build) | Low (binary path) | High (sidecar lifecycle) |
| Runtime overhead | Minimal (in-process) | Medium (process spawn) | High (HTTP + serialization) |
| Streaming support | Callbacks | stdout pipe | SSE |
| State sharing | In-process Arc | File-based | Shared Runtime |
| Debugging | Single process | Log files | Two processes |
| Multi-consumer | No (embedded) | No (per-process) | Yes (shared server) |
| Protocol interop | No | No | Yes (MCP, A2A) |
| Existing precedent | AgentMate uses PyO3 | AgentMate subprocess fallback | apxm-server exists |

**Phase 1-3 recommendation:** NAPI module (best performance, simplest debugging, precedent from AgentMate's PyO3 pattern). Subprocess as fallback.

**Phase 4+ recommendation:** HTTP server (graph execution, streaming, multi-agent coordination, protocol interop).

---

## 6. Summary

1. **apxm-server is a graph execution gateway**, not an LLM proxy. It has no `/v1/generate-stream` endpoint.

2. **All sessions share one AAM** in the current server. This is fine for single-consumer use but breaks with multiple independent consumers.

3. **Session isolation infrastructure exists** (child_scope, ScopeSpec, ScopeRegistry, SessionManager) but is **not wired into the server**.

4. **For Phases 1-3:** Embed APXM as a library (Codex) or NAPI module (Gemini-CLI). No server needed. Each consumer owns its own Runtime = automatic isolation.

5. **For Phase 4+:** Introduce `apxm-server` for graph execution. Wire up per-session AAM isolation first. The server's MCP + A2A + agent registry capabilities become essential for the universal substrate vision.

6. **Pre-Phase 4 server work:** Add per-session AAM isolation, session lifecycle API, and optionally a `/v1/generate-stream` convenience endpoint (wraps a single-ASK-node graph).
