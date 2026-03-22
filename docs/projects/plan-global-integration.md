# Global Integration Plan: Codex & Gemini-CLI on the A-PXM Substrate

**Author:** Architecture Team
**Date:** 2026-03-20
**Status:** Draft v6 -- Revised with investigation findings
**Scope:** Step-by-step migration of Codex and Gemini-CLI onto the A-PXM Program Execution Model -- from LLM backend through tools, AAM state, and ultimately compiler integration

---

## Foundational Correction: What A-PXM Is

A-PXM is **not a runtime**. It is a **Program Execution Model** — a formal specification of how agent programs are represented, optimized, and executed, materialized as a compiler (MLIR-based) and a runtime (dataflow scheduler + AAM). The relationship is:

```
Von Neumann PXM:                    Agent PXM (A-PXM):
┌─────────────────┐                 ┌──────────────────────┐
│ Abstract Machine │                 │ Abstract Machine     │
│ = (PC, Regs, Mem)│                 │ = (B, G, C)          │
├─────────────────┤                 ├──────────────────────┤
│ Runtime =        │                 │ Runtime =            │
│  CPU silicon     │ ← implements    │  Dataflow scheduler  │
│  + RAM chips     │                 │  + Memory tiers      │
│  + bus           │                 │  + Capability system  │
├─────────────────┤                 ├──────────────────────┤
│ Compiler =       │                 │ Compiler =           │
│  gcc/clang       │ ← optimizes     │  MLIR passes         │
│  + linker        │                 │  + AIS → .apxmobj    │
└─────────────────┘                 └──────────────────────┘
```

The **punch line is the compiler**. Once Codex and Gemini-CLI express their agent loops as AIS graphs, those graphs can be compiled, analyzed, and optimized — FuseAskOps, CSE, DCE, parallelism extraction. Instead of a simple plan, you create an **apxm-graph that gets compiled and analyzed**.

But the compiler requires the runtime, the runtime requires the AAM, and the AAM requires the tools and LLM backends. So we build from the bottom up.

---

## APXM as External Dependency (NOT Relative Paths)

**Problem:** Current plans reference APXM crates via relative paths (e.g. `../../../../apxm/crates/apxm-core` from `openai/codex/codex-rs/core`). This is fragile, non-portable, and assumes a specific directory layout.

**Solution:** APXM must be installable as a proper external dependency. Both Codex and Gemini-CLI must auto-install it.

### Installation Mechanism

```bash
# APXM installs to ~/.apxm/ (already exists for credentials)
# Binary: ~/.apxm/bin/apxm
# Libraries: ~/.apxm/lib/ (shared objects)
# Crates: git dependency or local registry

# For Rust consumers (Codex):
[workspace.dependencies]
apxm-core     = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-backends = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-runtime  = { git = "https://github.com/user/apxm", tag = "v0.1.0" }
apxm-events   = { git = "https://github.com/user/apxm", tag = "v0.1.0" }

# For TypeScript consumers (Gemini-CLI):
# apxm-server binary at ~/.apxm/bin/apxm-server
# Auto-started as sidecar, discovered via APXM_HOME env var
```

### `~/.apxm/` Directory Structure (Current + Planned)

```
~/.apxm/                            # APXM_HOME
├── bin/                             # Installed binaries
│   ├── apxm                        # CLI (compiler + runtime)
│   └── apxm-server                 # HTTP+SSE LLM service
├── config.toml                      # Backend configuration
├── credentials.toml                 # LLM provider credentials (0600)
├── tools.toml                       # Registered tools/capabilities (new)
├── memory/                          # Default memory tier storage
│   ├── ltm.sqlite                   # Long-term memory (SQLite)
│   └── episodes.jsonl               # Episodic memory (append-only)
└── workspaces/                      # Hierarchical AAM state (future)
    └── <workspace-id>/              # Per-workspace AAM scope
        ├── scope.toml               # Scope metadata
        ├── data/                    # B (Beliefs)
        ├── goals/                   # G (Goals)
        └── tools/                   # C (Capabilities)
```

### Auto-Installation

Both consumers need a setup step:

```rust
// Codex: build.rs or first-run check
fn ensure_apxm_installed() {
    let apxm_home = apxm_core::paths::ApxmPaths::discover()
        .unwrap_or_else(|_| panic!("APXM not installed. Run: cargo install apxm-cli"));
    // Verify version compatibility
    // Verify apxm-server binary exists (for TypeScript consumers)
}
```

```typescript
// Gemini-CLI: startup check
async function ensureApxmInstalled(): Promise<string> {
  const apxmHome = process.env.APXM_HOME ?? path.join(os.homedir(), '.apxm');
  const serviceBin = path.join(apxmHome, 'bin', 'apxm-server');
  if (!fs.existsSync(serviceBin)) {
    throw new Error('APXM not installed. Run: cargo install apxm-cli');
  }
  return apxmHome;
}
```

---

## The Five-Phase Adoption Path

This follows the structure from the A-PXM Codex case study (`docs/projects/codex/plan.md`) and vision doc (`docs/pxm/vision.md`), adapted for both consumers:

```
Phase 1:  LLM Backend        — Replace LLM transport with apxm-backends
Phase 2:  Tool Migration      — Register consumer tools as APXM capabilities (INV)
Phase 3:  AAM State Model     — Map agent state to hierarchical (B, G, C)
Phase 4:  Agent Loop as Graph — Express turn loops as AIS graphs
Phase 5:  Compiler Integration — Compile, analyze, and optimize agent graphs
```

### Why This Order

Each phase builds on the previous:
- Phase 1 brings APXM into the project as a dependency (the "fork & baseline")
- Phase 2 populates the **C** (Capabilities) component of the AAM with real tools
- Phase 3 populates **B** (Beliefs) and **G** (Goals) — the agent's typed state
- Phase 4 replaces imperative code with typed AIS graphs that use B, G, C
- Phase 5 compiles those graphs — the punch line

Each phase is **independently valuable**. Phase 1 alone gives unified streaming. Phase 2 alone gives typed tool dispatch. Phase 3 alone gives inspectable agent state. You don't need to commit to Phase 5 to benefit from Phase 1.

---

## Current Agent Operations → AIS Mapping

### Codex Operations → AIS

| Codex Operation | Location | AIS Equivalent | AAM Effect | Phase |
|----------------|----------|---------------|------------|-------|
| LLM call (prompt → response) | `RegularTask::run()` → OpenAI API | **ASK** / **THINK** / **REASON** | Reads B, writes B | 1 |
| Extended thinking | `ReasoningEffortConfig` | **THINK** (thinking_budget) | Reads B, writes B | 1 |
| Shell execution | `ShellHandler`, `ShellCommandHandler` | **EXC** (sandboxed) | Reads C, writes B | 2 |
| File read/write | `ReadFileHandler`, `ApplyPatchHandler` | **INV** (capability) | Reads C, writes B | 2 |
| Tool search/suggest | `ToolSearchHandler`, `ToolSuggestHandler` | **QMEM** (capability lookup) | Reads C | 2 |
| MCP tool calls | `McpHandler`, `McpResourceHandler` | **INV** (MCP capability) | Reads C, writes B | 2 |
| Guardian approval | `GuardianReviewSessionManager` | **VERIFY** (claim → verdict) | Reads B, writes B | 2 |
| Multi-agent spawn/wait | `spawn.rs`, `wait.rs`, `resume_agent.rs` | **FLOW_CALL** / **WAIT_ALL** | Writes C, reads B | 4 |
| Context compaction (2-phase) | Phase 1: `gpt-5.1-codex-mini`, Phase 2: `gpt-5.3-codex` | **QMEM** + **UMEM** | Reads/writes B (memory tier) | 3 |
| Session/Turn state | `SessionState`, `TurnState` | **AAM Beliefs** | — | 3 |
| `submission_loop()` dispatch | `codex.rs:4138` | **AIS Graph** (DAG) | Full AAM transition | 4 |

### Gemini-CLI Operations → AIS

| Gemini-CLI Operation | Location | AIS Equivalent | AAM Effect | Phase |
|---------------------|----------|---------------|------------|-------|
| LLM call | `ContentGenerator.generateContentStream()` | **ASK** / **THINK** | Reads B, writes B | 1 |
| Tool execution | `processFunctionCalls()` | **INV** (capability) | Reads C, writes B | 2 |
| Tool confirmation | `onWaitingForConfirmation` | **VERIFY** (approval) | Reads B, writes B | 2 |
| Parallel tool calls | Scheduler via `Promise.all()` | **INV** × N + **WAIT_ALL** | Concurrent B writes | 2 |
| Context compression | `ChatCompressionService` | **QMEM** + **UMEM** | Reads/writes B | 3 |
| Loop detection | `LoopDetectedEvent` | **REFLECT** (pattern detection) | Reads Episodic | 3 |
| Max turns / timeout | `checkTermination()`, `DeadlineTimer` | **GUARD** (precondition) | — | 2 |
| Recovery turn | `executeFinalWarningTurn()` | **TRY_CATCH** (exception) | Reads B, writes B | 4 |
| Session state | `GeminiChat`, conversation history | **AAM Beliefs** | — | 3 |
| MCP tool discovery | `McpClientManager` | **AAM Capabilities** | Writes C | 2 |
| Hooks (11 event types) | BeforeAgent, AfterAgent, BeforeModel, AfterModel, BeforeToolCall, AfterToolCall, BeforeToolConfirmation, AfterToolConfirmation, OnError, OnRetry, OnContextWindowOverflow | **FENCE** (ordering) | — | 4 |
| `executeTurn()` loop | `local-executor.ts:~316` | **AIS Graph** (DAG) | Full AAM transition | 4 |

---

## Architecture Overview

### Phase 1: LLM Backend (Runtime Only)

```
┌─── Codex (Rust) ──────────┐     ┌─── Gemini-CLI (TS) ─────────┐
│  apxm_adapter (in-process) │     │  NAPI module (in-process)    │
│  fn(ApxmEvent) → CodexEvent│     │  fn(ApxmEvent) → GeminiEvent │
└────────────┬───────────────┘     └────────────┬──────────────────┘
             │ Rust API                         │ NAPI (napi-rs)
             ▼                                  ▼
┌─────────────────────── APXM (installed at ~/.apxm/) ───────────┐
│  apxm-events     apxm-backends                                  │
│  (33 payloads)   (StreamAssembler)                               │
│                  (5 LLM backends)                                │
└──────────────────────────────────────────────────────────────────┘
Note: apxm-server is NOT used in Phases 1-3. Each consumer embeds
its own Runtime via Rust API (Codex) or NAPI module (Gemini-CLI).
```

### Phase 2: Tool Migration (Capability System)

```
┌─── Codex ──────────────────┐     ┌─── Gemini-CLI ──────────────┐
│  21+5 ToolHandler impls     │     │  DeclarativeTool + MCP tools  │
│  Each registered as         │     │  Each registered as           │
│  APXM CapabilityExecutor   │     │  APXM capability via HTTP     │
└────────────┬───────────────┘     └────────────┬──────────────────┘
             │                                  │
             ▼                                  ▼
┌─────────────────────── APXM Capability System ─────────────────┐
│  CapabilitySystem (registry)                                    │
│  CapabilityExecutor (trait)                                     │
│  CapabilityMetadata (schema, cost, latency, read_only)          │
│  Interceptor pipeline (approval, sandbox, audit)                │
│  ~/.apxm/tools.toml (persistent registration)                  │
└────────────────────────────────────────────────────────────────┘
```

### Phase 3: AAM State Model (Hierarchical File Tree)

```
The File Tree IS the AAM:

~/.apxm/workspaces/codex-session-123/
├── data/                          B (Beliefs)
│   ├── conversation.json          - Chat history
│   ├── pending_tools.json         - In-flight tool calls
│   └── approvals.json             - Cached approval decisions
├── goals.toml                     G (Goals)
│   goal = "Fix the failing test"
│   priority = 1
└── tools/                         C (Capabilities)
    ├── shell.toml                 - Shell execution capability
    ├── read_file.toml             - File read capability
    └── mcp/                       - MCP-discovered tools
        └── filesystem.toml
```

### Phase 4: Agent Loop as Graph (apxm-server Enters Here)

```
┌─── Codex (Rust) ──────────┐     ┌─── Gemini-CLI (TS) ─────────┐
│  Turn graph builder         │     │  Turn graph builder           │
│  submission_loop() → JSON   │     │  executeTurn() → JSON         │
└────────────┬───────────────┘     └────────────┬──────────────────┘
             │ POST /v1/execute                 │ POST /v1/execute
             ▼                                  ▼
┌─────────────────────── apxm-server ──────────────────────────────┐
│  Per-session AAM isolation (child_scope per session_id)           │
│  Graph compilation + execution (MLIR O1+)                        │
│  SSE streaming of execution events                               │
│  MCP + A2A protocol support (multi-agent coordination)           │
│  Capability system (shared tool registry)                        │
└──────────────────────────────────────────────────────────────────┘
Note: This is where apxm-server adds real value -- one HTTP call
per turn (not per LLM call), streaming execution events, and
multi-consumer coordination with per-session state isolation.
```

### Phase 5 Vision: Compiler Integration (The Punch Line)

```
Instead of:                        You write:
┌──────────────────────┐           ┌──────────────────────┐
│ while (true) {       │           │ agent CodingAssistant│
│   prompt = build()   │           │   ask(prompt) -> resp│
│   resp = llm.call()  │  ────►    │   branch(resp.tools) │
│   if resp.tools {    │           │     inv(tool_a)  ──┐ │
│     for tool in ..   │           │     inv(tool_b)  ──┤ │
│     results = exec() │           │   wait_all ◄───────┘ │
│   }                  │           │   verify(approval)   │
│ }                    │           │   umem(save_result)   │
└──────────────────────┘           └──────────────────────┘
                                              │
                                    apxm compile graph.json
                                              │
                                              ▼
                                   ┌──────────────────────┐
                                   │ Compiler analysis:    │
                                   │ - 2 INVs are parallel │
                                   │ - FuseAsk applicable  │
                                   │ - No dead operations  │
                                   │ - Type-safe           │
                                   └──────────────────────┘
```

---

## Plan Structure: One Directory Per Phase

Each phase has its own directory with a README and three consumer-specific files (APXM, Codex, Gemini-CLI):

| Phase | Directory | APXM Scope | Codex Scope | Gemini-CLI Scope |
|-------|-----------|------------|-------------|------------------|
| **1** | [phase-1-llm-backend/](phase-1-llm-backend/) | apxm-events, backends, llm-service, packaging | Adapter, feature-gated integration | SSE client, event translator, generator upgrade |
| **2** | [phase-2-tool-migration/](phase-2-tool-migration/) | Capability system, interceptors, tools.toml | 21 standard + 5 multi_agents tool adapters, guardian interceptor | Capability registration, VERIFY, policy bridge |
| **3** | [phase-3-aam-state/](phase-3-aam-state/) | Hierarchical AAM, WorkspaceManager, memory | SessionState/TurnState → AAM | GeminiChat → AAM, three-tier memory |
| **4** | [phase-4-agent-loop/](phase-4-agent-loop/) | Graph execution, /v1/execute endpoint | submission_loop() as AIS graph | executeTurn() as AIS graph, hooks as FENCE |
| **5** | [phase-5-compiler/](phase-5-compiler/) | Compiler passes for consumer graphs | codex-turn.apxmobj | gemini-turn.apxmobj |

---

## Phase Dependencies & Timeline

```
Phase 1 — LLM Backend (Weeks 1-8)
  Plan 1: apxm-events + backends + llm-service + packaging
  Plan 2: Codex adapter (depends on Plan 1)
  Plan 3: Gemini-CLI SSE client (depends on Plan 1)

Phase 2 — Tool Migration (Weeks 9-14)
  Plan 1: Capability system extensions, interceptor pipeline
  Plan 2: Register 21 standard + 5 multi_agents Codex ToolHandlers as APXM capabilities
  Plan 3: Register Gemini-CLI tools as APXM capabilities via HTTP

Phase 3 — AAM State Model (Weeks 15-20)
  Plan 1: Hierarchical AAM (scoped state, file-backed)
  Plan 2: Map Codex SessionState/TurnState to AAM (B, G, C)
  Plan 3: Map Gemini-CLI GeminiChat state to AAM (B, G, C)

Phase 4 — Agent Loop as Graph (Weeks 21-28)
  Plan 2: Express submission_loop() as AIS graph
  Plan 3: Express executeTurn() as AIS graph

Phase 5 — Compiler Integration (Weeks 29+)
  All: Compile agent graphs, run optimization passes, measure improvement
```

---

## Design Decisions

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | APXM is an **external dependency**, not relative paths | Portable, installable, version-tracked |
| D2 | Codex consumes in-process (Rust), Gemini-CLI via NAPI module (Phases 1-3) then HTTP (Phase 4+) | Same language = zero overhead; cross-language = NAPI for performance, HTTP for coordination |
| D3 | Phase 1 uses **runtime only**, no compiler | LLM backend + streaming doesn't require MLIR |
| D4 | Phase 2 maps tools to **C** (Capabilities) | The AAM's C component IS the tool registry |
| D5 | Phase 3 maps state to **B** (Beliefs) backed by files | File-backed AAM = inspectable, diffable, persistent |
| D6 | Phase 4 replaces imperative loops with AIS graphs | The prerequisite for Phase 5 (compilation) |
| D7 | Phase 5 is the punch line — compiler analysis | Plans become compiled, optimized AIS graphs |
| D8 | The file tree IS the AAM | `data/` = B, `goals.toml` = G, `tools/` = C |
| D9 | `~/.apxm/` is the shared state root | Already used for credentials, memory, tools |
| D10 | `#[non_exhaustive]` on all public enums | Forward compatibility across phases |
| D11 | Feature-gated in Codex, env-var-gated in Gemini-CLI | Non-breaking adoption in both consumers |
| D12 | `apxm-server` enters at Phase 4, not Phase 1 | Server is a graph execution gateway, not an LLM proxy; no `/v1/generate-stream` endpoint exists |
| D13 | Per-session AAM isolation via `child_scope(Isolate)` | Prevents state leaks between independent consumers/sessions |
| D14 | Each session IS an AAM scope | Consumer session maps 1:1 to an isolated `(B, G, C)` triple |

---

## What Each Consumer Gets Per Phase

### Phase 1: LLM Backend
- **Codex**: Multi-provider fallback, unified credentials, real streaming
- **Gemini-CLI**: Real token-by-token streaming (currently single-yield), removes 400 lines of in-TS provider dispatch
- **APXM**: Real-world streaming validation, Responses API backend

### Phase 2: Tool Migration
- **Codex**: Typed tool dispatch via CapabilitySystem, interceptor pipeline (approval + sandbox + audit), tool metadata (latency, cost, read_only)
- **Gemini-CLI**: Typed tool dispatch, tool state machine formalized (pending → approved → executing → completed)
- **APXM**: 26 real-world tools (21 standard Phase 2 + 5 multi_agents Phase 4) exercising the capability system

### Phase 3: AAM State Model
- **Codex**: Inspectable state (`apxm state show session-123`), diffable (`git diff` on AAM), persistent across sessions
- **Gemini-CLI**: Same, plus three-tier memory (STM for current turn, LTM for project memory, Episodic for execution trace)
- **APXM**: File-tree-backed hierarchical AAM in production

### Phase 4: Agent Loop as Graph
- **Both**: Declarative agent loops, automatic parallelism from DAG topology, compile-time validation
- **Codex**: `submission_loop()` --> AIS graph (ASK --> BRANCH_ON_VALUE --> INV x N --> WAIT_ALL --> VERIFY --> UMEM)
- **Gemini-CLI**: `executeTurn()` --> AIS graph (GUARD --> ASK --> BRANCH_ON_VALUE --> INV x N --> WAIT_ALL --> TRY_CATCH --> REFLECT)

### Phase 5: Compiler Integration (The Punch Line)
- **Both**: FuseAskOps (fewer API calls), CSE (no duplicate LLM calls), DCE (remove unused operations), parallelism extraction, compile-time type checking (49x faster error detection), `.apxmobj` artifacts
- **Key insight**: Instead of a simple plan, you create an apxm-graph that gets compiled and analyzed

---

## Risk Matrix

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| APXM packaging complexity | Medium | High | Start with git dependencies, publish to crates.io later |
| Tool registration overhead for 26 handlers (21 standard + 5 multi_agents) | Medium | Medium | Thin adapter trait (~20 LOC per handler); Phase 2 handles 21 standard, Phase 4 handles 5 multi_agents |
| Hierarchical AAM not yet implemented | High | Medium | Start with flat AAM (already works), add hierarchy in Phase 3 |
| AIS graph authoring complexity | Medium | High | Start with simple single-ASK graphs, grow incrementally |
| MLIR dependency for compiler | Low | Medium | Compiler (Phase 5) is optional; Phases 1-4 use runtime only |
| Performance regression during migration | Low | High | Shadow mode comparison at each phase |

---

## Success Criteria

### Phase 1 Complete:
- [ ] APXM installable as external dependency (git or cargo install)
- [ ] `apxm-events` crate builds, all 33 variants serde round-trip
- [ ] Real streaming from OpenAI, Anthropic, Google backends
- [ ] Codex and Gemini-CLI use APXM for LLM calls (feature-gated)

### Phase 2 Complete:
- [ ] All 21 standard Codex ToolHandlers registered as APXM capabilities (5 multi_agents handlers deferred to Phase 4)
- [ ] Gemini-CLI tools registered via HTTP capability bridge
- [ ] Tool approval flows through APXM interceptor pipeline
- [ ] `~/.apxm/tools.toml` persists registrations

### Phase 3 Complete:
- [ ] Agent state inspectable: `apxm state show <session>`
- [ ] AAM (B, G, C) backed by file tree under `~/.apxm/workspaces/`
- [ ] Three-tier memory operational (STM, LTM, Episodic)

### Phase 4 Complete:
- [ ] Codex `submission_loop()` expressible as AIS graph
- [ ] Gemini-CLI `executeTurn()` expressible as AIS graph
- [ ] Automatic parallelism demonstrated on concurrent tool calls

### Phase 5 Complete:
- [ ] `apxm compile agent-graph.json` produces optimized `.apxmobj`
- [ ] FuseAskOps reduces API calls on real workflows
- [ ] Compile-time validation catches structural errors before any LLM call

---

## Beyond Codex and Gemini-CLI: APXM as Universal Substrate

APXM is not designed exclusively for Codex and Gemini-CLI. It is the **backend infrastructure for all agent frameworks**. Any agent system -- whether it is a Rust CLI, a Python SDK, a TypeScript framework, or a custom enterprise agent -- can adopt APXM as its execution substrate by following the same five-phase adoption path:

1. **Replace LLM transport** -- Route LLM calls through `apxm-backends` (in-process for Rust, via `apxm-server` HTTP+SSE for other languages). Gain unified streaming, multi-provider fallback, and credential management.
2. **Register tools** -- Wrap existing tool implementations as APXM `CapabilityExecutor` instances. Gain typed dispatch, interceptor pipelines (approval, sandbox, audit), and persistent registration via `~/.apxm/tools.toml`.
3. **Map state to AAM** -- Express agent state as the AAM triple `(B, G, C)` -- Beliefs, Goals, Capabilities -- backed by the hierarchical file tree under `~/.apxm/workspaces/`. Gain inspectable, diffable, persistent state.
4. **Express agent loop as graph** -- Replace imperative control flow with a typed AIS graph (DAG of operations). Gain automatic parallelism from DAG topology, compile-time validation, and structural analysis.
5. **Compile and optimize** -- Feed the graph to the MLIR-based compiler. Gain FuseAskOps (fewer API calls), CSE (no duplicate LLM calls), DCE (remove dead operations), and parallelism extraction. This is the punch line.

Codex and Gemini-CLI are the first two consumers. They validate each phase of the substrate with real-world complexity (26 tool handlers, 18 event types, 7-state tool state machine, 49-variant notification enums). Future consumers benefit from the infrastructure these two validate.

The key architectural insight is that **the same five-phase path applies regardless of the consumer's language or framework**. A Python agent framework would follow the same phases, using `apxm-server` HTTP endpoints instead of in-process Rust APIs. The substrate is language-agnostic; only the integration mechanism differs.

---

## Lessons from Investigation

Five investigation reports (one per phase) cross-referenced every plan claim against source code. The top findings:

### 1. Existing infrastructure is more mature than originally estimated

- **Scoping IS wired**: `Aam::child_scope()` is fully implemented and tested. `ExecutionContext` carries `scope_id` and `scope_registry`. The claim "types exist but are not yet wired" is partially incorrect -- in-memory scoping works. What is missing is file-tree backing and the `promote()` direction.
- **Workspace infrastructure exists**: `ScopeRegistry` and `WorkspaceManager` are implemented with tests. The file-system-backed directory management is what remains.
- **Memory scoping already works**: The `MemorySystem` has scoped key-prefix namespacing (`__scope__/<scope_id>/<key>`) with `read_scoped()`, `write_scoped()`, `search_scoped()`, `delete_scoped()`.
- **Capability system is production-ready**: `CapabilitySystem` has JSON Schema validation, timeout enforcement, interceptor pipeline, approval channel, and AAM integration.

### 2. Some claimed validations do not exist yet

- **Attribute validation**: `validate.rs` does NOT check required attributes per operation type. It validates structural properties (IDs, edges, acyclicity, parameters, providers) but not per-operation attribute requirements. The `OperationSpec.fields` array has `required` flags, but `validate.rs` does not use them.
- **Type checking on edges**: No type compatibility checks exist on Data edges. The graph validator does not annotate edge tokens with types.
- **Capability references**: INV nodes referencing unregistered tools are not caught at validation time -- only at runtime.

### 3. GUARD wire index is a hard blocker for Phase 5

GUARD has no wire index (`to_wire_index()` returns `None`), meaning graphs containing GUARD cannot be serialized to `.apxmobj` artifacts. The Gemini-CLI turn graph starts with GUARD. Wire indices 25-29 are reserved for Phase 1 ISA extensions but currently unmapped.

### 4. File sizes have grown significantly since plan was written

Six files are >50% larger than estimated, suggesting the plan was written against an older codebase revision:

| File | Plan Estimate | Actual | Delta |
|------|--------------|--------|-------|
| `codex.rs` | ~5400 | 7321 | +36% |
| `client.rs` | ~800 | 1823 | +128% |
| `common.rs` (app-server-protocol) | ~900 | 1720 | +91% |
| `v2.rs` (app-server-protocol) | ~300 | 7978 | +2559% |
| `local-executor.ts` | ~700+ | 1479 | +111% |
| `geminiChat.ts` | ~500 | 1075 | +115% |
| `scheduler.ts` | ~400 | 785 | +96% |
| `policy.ts` | ~100 | 260 | +160% |
| `confirmation.ts` | ~100 | 339 | +239% |
| `tools.ts` | ~400 | 1001 | +150% |
| `tool-registry.ts` | ~200 | 762 | +281% |
| `hookSystem.ts` | ~200 | 444 | +122% |

The integration points are in larger, more complex files than estimated. The actual changes remain well-isolated, but implementers should use current line numbers.

### 5. The compiler has 12 passes, not 4

The plan references "four passes" (FuseAskOps, CSE, DCE, Canonicalization). The actual O2 pipeline has 12 distinct passes: `normalize`, `build-prompt`, `template-specialization`, `unconsumed-value-warning`, `schema-narrowing`, `scheduling`, `fuse-ask-ops`, `condense-ops`, `dead-context-elimination`, `canonicalizer`, `cse`, `symbol-dce`. Pass names use kebab-case in code (e.g., `fuse-ask-ops` not `FuseAskOps`, `symbol-dce` not `DCE`).

Optimization levels:

| Level | Pass Count | Behavior |
|-------|-----------|----------|
| O0 | 0 | No optimization (passthrough) |
| O1 | 8 | Basic: normalize, build-prompt, scheduling, fusion, canonicalization, CSE, DCE |
| O2 | 12 | O1 + template-specialization, dead-context-elimination, schema-narrowing, condense-ops |
| O3 | 93 | O2 passes iterated 10 times for fixed-point convergence |

### 6. No real streaming exists on ANY production backend

Only `MockBackend` overrides `generate_stream()`. OpenAI, Anthropic, Google, and Ollama all use the default single-`Done`-chunk wrapper. This means Phase 1 streaming work (A2.5-A2.7) is genuinely new implementation, not modifications of existing streaming code. Ollama streaming is not addressed in the plan and should be documented as using the `Done` wrapper fallback.

### 7. Service rename is already complete

The crate is already named `apxm-server` (`Cargo.toml` line 2: `name = "apxm-server"`). The `POST /v1/execute` and `POST /v1/execute/stream` endpoints already exist. Routes are defined inline in `apxm-server/src/main.rs`. Plan documents have been updated to use `apxm-server` throughout.

### 8. InterceptDecision has no Escalate variant

The actual `InterceptDecision` enum has `Allow`, `Deny { reason }`, and `EditArgs { args }` -- no `Escalate`. The existing `ApprovalChannel` mechanism already handles the escalation pattern: interceptor returns `Deny`, `ApprovalChannel` routes to user, user can override. Plan pseudocode using `InterceptDecision::Escalate` should use `InterceptDecision::Deny` + `ApprovalChannel` instead.

---

## Session Isolation and apxm-server Positioning

### The Session Isolation Problem

The APXM Runtime is a singleton with a shared `Aam` (`Arc<RwLock<AamState>>`). When `build_context()` clones the AAM, it clones the `Arc` — giving a shared reference, NOT a deep copy. All concurrent executions write to the **same** underlying beliefs, goals, and capabilities. The `SessionLaneGuard` serializes same-session requests but does NOT isolate state between sessions.

This is correct for multi-agent collaboration within ONE agent system, but it is a problem for independent consumers (Codex and Gemini-CLI should NOT share beliefs/goals).

### Solution: Phase-by-Phase Isolation

| Phase | Isolation Model | Mechanism |
|-------|----------------|-----------|
| **1-3** | One Runtime per consumer | Library embedding (Codex: Rust API) or NAPI module (Gemini-CLI: `napi-rs`). Each consumer owns its own Runtime = automatic isolation. No server needed. |
| **4+** | Per-session AAM in `apxm-server` | Server creates isolated child AAM per `session_id` via `get_or_create_session_aam()` using existing `child_scope(ScopeSpec::isolate())` |
| **Multi-agent** | Per-agent AAM with inheritance | `child_scope(ScopeSpec::inherit())` for collaborating agents within a session |

The scoping infrastructure already exists (`child_scope`, `ScopeSpec`, `ScopeRegistry`, `SessionManager`). What is missing is wiring it into `apxm-server`'s `build_context()`.

### apxm-server: When It Adds Value

`apxm-server` is a **graph execution gateway** (2,595 lines, 25+ routes, MCP + A2A protocol support). It is NOT an LLM proxy — there is no `/v1/generate-stream` endpoint. The server only has `/v1/execute` (graph execution) and `/v1/execute/stream` (graph execution with SSE).

| Phase | Without Server | With Server | Server Value |
|-------|---------------|-------------|-------------|
| 1 (LLM) | Direct API or NAPI | Must add `/v1/generate-stream` | **Negative** (new work, no benefit) |
| 2 (Tools) | Direct Rust or NAPI | HTTP capability registration | Neutral |
| 3 (AAM) | Direct Rust or NAPI + file tree | HTTP state access | **Negative** (adds latency) |
| 4 (Graphs) | Subprocess `apxm execute` | `POST /v1/execute` + SSE | **Positive** (streaming, concurrent graphs) |
| 5 (Compiled) | Subprocess `apxm compile + run` | `POST /v1/execute` with compilation | **Positive** (integrated pipeline) |
| Multi-agent | N/A | Agent registry, COMMUNICATE, A2A | **Essential** |

**Decision:** `apxm-server` enters the migration at Phase 4, not Phase 1.

### Cross-Language Bridge for Gemini-CLI (Phases 1-3)

Since `apxm-server` is deferred to Phase 4, Gemini-CLI (TypeScript) needs a different bridge to APXM (Rust) for Phases 1-3:

| Option | Setup | Runtime | Streaming | Precedent |
|--------|-------|---------|-----------|-----------|
| **NAPI module** (recommended) | Medium (`napi-rs` build) | Minimal (in-process) | Callbacks | AgentMate uses PyO3 |
| Subprocess | Low (binary path) | Medium (process spawn) | stdout pipe | AgentMate fallback |
| HTTP server | High (sidecar lifecycle) | High (HTTP + serde) | SSE | `apxm-server` exists |

**Recommendation:** NAPI module for Phases 1-3 (best performance, single process, proven pattern). HTTP via `apxm-server` for Phase 4+ (graph execution, streaming, multi-agent coordination).

### Session → AAM Mapping

Each consumer session maps to an isolated AAM scope. The detailed field-by-field mapping is in [SESSION-AAM-MAPPING.md](SESSION-AAM-MAPPING.md). Summary:

- **Codex `SessionState`** → B: conversation, approvals, pending tools, config; G: user instruction; C: 21 tool handlers + MCP
- **Gemini-CLI `GeminiChat`** → B: conversation, compression, turn state; G: user query; C: built-in tools + MCP + agent delegation
- **Non-serializable exclusions**: Codex `TurnState` has 5 `oneshot::Sender` channels that CANNOT be mapped to AAM
- **Session lifecycle**: create (new AAM scope) → turn N (update beliefs, append episodic) → end (materialize to workspace)

### Pre-Phase 4 Server Work

Before `apxm-server` can serve multiple consumers at Phase 4:

1. **Per-session AAM** — `build_context()` uses `get_or_create_session_aam(session_id)` instead of cloning root AAM
2. **Per-session memory namespacing** — Use `read_scoped()`/`write_scoped()` with session_id as scope prefix
3. **Per-consumer capability isolation** — Some capabilities global (LLM backends), some per-consumer
4. **Session lifecycle API** — Create, checkpoint, restore, destroy sessions via HTTP endpoints

---

## Prerequisites Before Phase 5

Before Phase 5 (Compiler Integration) can begin, the following items must be resolved:

- [ ] **GUARD wire index assigned (26)** -- Without this, graphs containing GUARD cannot be compiled to `.apxmobj`. The Gemini-CLI turn graph starts with GUARD. Hard blocker.
- [ ] **Other wire indices assigned** -- UpdateGoal (25), Claim (27), Pause (28), Resume (29). These use the reserved Phase 1 ISA slots.
- [ ] **Required attribute validation in `validate.rs`** -- Use `OperationSpec.fields` to check required attributes per operation type. Without this, the "49x faster error detection" claim is not fully realized.
- [ ] **BRANCH_ON_VALUE naming in all consumer code** -- All graph JSON must use `"BRANCH_ON_VALUE"` not `"BRANCH"`. There is no serde alias for the short form.
- [ ] **TRY_CATCH MLIR subgraph verification** -- The TRY_CATCH handler is a thin passthrough; the scheduler's error routing for subgraph boundaries needs verification before Phase 5 can compile TRY_CATCH-containing graphs.
- [ ] **Type checking on data edges** -- Currently not implemented. Required for the "type mismatches on data edges" compile-time check promised in A8.3.
- [ ] **Capability reference validation** -- INV nodes referencing unregistered tools should be caught at validation time, not runtime.

---

## Related Documents

### Phase Plans
- [Phase 1: LLM Backend](phase-1-llm-backend/) — Streaming, events, packaging
- [Phase 2: Tool Migration](phase-2-tool-migration/) — Capabilities, interceptors
- [Phase 3: AAM State Model](phase-3-aam-state/) — Hierarchical file-backed (B, G, C)
- [Phase 4: Agent Loop as Graph](phase-4-agent-loop/) — Declarative AIS graphs
- [Phase 5: Compiler Integration](phase-5-compiler/) — The punch line

### A-PXM Theory & Architecture
- [PXM Foundations](../pxm/foundations.md) — Why agent workflows need a formal execution model
- [AAM: Agent Abstract Machine](../pxm/aam.md) — Formal state model
- [Vision: The LLVM for Agents](../pxm/vision.md) — File tree as AAM, compiler as punch line
- [Hierarchical AAM Diagrams](../implementation/runtime/hierarchical-aam.md) — 14 diagrams showing flat → hierarchical migration
- [AIS Operations](../pxm/ais.md) — 39 typed operations

### Investigation Reports
- [Phase 1 Investigation](phase-1-llm-backend/INVESTIGATION.md) -- LLM backend source code cross-reference
- [Phase 2 Investigation](phase-2-tool-migration/INVESTIGATION.md) -- Tool migration source code cross-reference
- [Phase 3 Investigation](phase-3-aam-state/INVESTIGATION.md) -- AAM state model source code cross-reference
- [Phase 4 Investigation](phase-4-agent-loop/INVESTIGATION.md) -- Agent loop as graph source code cross-reference
- [Phase 5 Investigation](phase-5-compiler/INVESTIGATION.md) -- Compiler integration source code cross-reference

### Session & Server Architecture
- [Session → AAM Mapping Design](SESSION-AAM-MAPPING.md) -- How consumer sessions map to AAM (B, G, C) with field-level detail
- [Sessions & Server Investigation](SESSIONS-AND-SERVER-INVESTIGATION.md) -- Runtime singleton, shared AAM, apxm-server analysis, NAPI feasibility

### Consumer Case Studies
- [Case Study: Codex on A-PXM](codex/case-study.md) — The LLVM-GCC parallel
- [Codex Plan](codex/plan.md) — 5-phase reconstruction plan
