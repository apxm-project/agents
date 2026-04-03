# APXM Host Integration Guide — Codex & Gemini CLI

**Date**: 2026-03-23
**Status**: Reference Architecture + Implementation Status

---

## 1. APXM as Agnostic Connector

APXM is a **Program Execution Model** — a formal specification of how agent
programs are represented, optimized, and executed. It is composed of a compiler
(MLIR-based optimization) and a runtime (dataflow scheduler + AAM). The
relationship is analogous to LLVM: shared infrastructure where improvements
benefit every agent built on it.

> "A-PXM is not a framework, not just a compiler, and not just a runtime. It is
> a Program Execution Model — a formal specification of how agent programs are
> represented, optimized, and executed."
> — `docs/pxm/foundations.md`

For host integration (Codex, Gemini), the current focus is on APXM's **runtime
capabilities**: the three pluggable backend interfaces. The compiler is used
when executing AIS graphs (`apxm compile` → `.apxmobj` → `apxm run`), applying
MLIR passes like FuseAskOps, CSE, and DCE for optimization.

APXM defines three orthogonal pluggable interfaces that host applications
implement:

```
                    ┌─────────────────────────────┐
                    │     AIS Graph (.json)        │
                    │  [ASK] → [INV:bash] → [ASK]  │
                    └──────────┬──────────────────┘
                               │ compile + execute
                               ▼
┌──────────────────────────────────────────────────────────────┐
│              APXM (Program Execution Model)                     │
│                                                               │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────┐   │
│  │ LLMRegistry │  │ CapabilitySys│  │ SandboxRegistry    │   │
│  │ (who thinks)│  │ (what to do) │  │ (how to isolate)   │   │
│  └──────┬──────┘  └──────┬───────┘  └────────┬───────────┘   │
│         │                │                    │               │
│    trait LLMBackend  trait CapabilityExecutor  trait SandboxBackend
│         │            + to_exec_request()      │               │
│         │                │                    │               │
│    APXM defines      APXM defines         APXM defines      │
│    INTERFACES        INTERFACES           INTERFACES         │
└────┼─────────────────┼────────────────────┼───────────────────┘
     │                 │                    │
     ▼                 ▼                    ▼
 ┌───────────┐  ┌──────────────┐  ┌─────────────────────┐
 │ Host LLM  │  │ Host Tools   │  │ Host Sandbox        │
 ├───────────┤  ├──────────────┤  ├─────────────────────┤
 │ OpenAI    │  │ bash         │  │ APXM: ProcessSbx    │
 │ Anthropic │  │ read_file    │  │ Codex: bwrap+seccomp│
 │ Ollama    │  │ web_search   │  │ Gemini: seatbelt    │
 │ Codex LLM │  │ user_tools   │  │ Docker: container   │
 │ Gemini LLM│  │ mcp_tools    │  │ Custom: anything    │
 └───────────┘  └──────────────┘  └─────────────────────┘
```

**Key principle**: APXM ships zero platform-specific code. Each host injects
its own implementations of the three traits at startup.

### 1.1 The Fourth Axis: AAM (Agent Abstract Machine)

Beyond the three pluggable backend interfaces, APXM's PXM defines a **formal
state model** — the Agent Abstract Machine (AAM) — that hosts can leverage for
agent coordination:

```
AAM = (B, G, C)

B (Beliefs):      HashMap<String, Value>        — what the agent knows
G (Goals):        PriorityQueue<Goal>           — what the agent wants
C (Capabilities): HashMap<String, CapRecord>    — what the agent can do
```

Every AIS instruction executes as a pure state transition: `δ(AAM, Instr) → AAM'`

This separation of state from compute means:
- **Compiler** can analyze state dependencies for optimization
- **Runtime** records transitions for audit/reflection
- **Hosts** can checkpoint/restore agent state across sessions
- **Multi-agent** scoping (Inherit/Isolate/Snapshot/Filter) enables safe delegation

---

## 2. Codex Integration (Rust-Native)

### 2.1 Architecture

Codex is Rust — same language as APXM. Integration is in-process via direct
trait implementation, gated behind `#[cfg(feature = "apxm-llm")]`.

```
┌─────────────────────────────────────────────────────────────┐
│ CODEX CLI (Rust)                                             │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐    │
│  │ Session                                               │    │
│  │  ├── apxm_registry: OnceCell<Arc<LLMRegistry>>       │    │
│  │  ├── tool_runtime: ToolCallRuntime                    │    │
│  │  └── sandbox_manager: SandboxManager                  │    │
│  └──────────┬───────────────────────────────────────────┘    │
│             │                                                │
│     ┌───────┴───────────────────────────────┐                │
│     │ apxm_enabled?                         │                │
│     │  YES → run_apxm_sampling_request()    │                │
│     │  NO  → normal Responses API path      │                │
│     └───────┬───────────────────────────────┘                │
│             │                                                │
│  ┌──────────┴──────────────────────────────────────────┐     │
│  │ APXM Adapter Layer (codex-rs/core/src/apxm_adapter/)│     │
│  │                                                      │     │
│  │  request_translator.rs                               │     │
│  │    Codex Prompt → APXM LLMRequest                    │     │
│  │    • BaseInstructions → system prompt                │     │
│  │    • ResponseItem[] → Message[]                      │     │
│  │    • ToolSpec[] → ToolDefinition[]                   │     │
│  │                                                      │     │
│  │  client.rs                                           │     │
│  │    ApxmModelClient wraps Arc<LLMRegistry>            │     │
│  │    → stream_turn() returns Stream<CodexApxmEvent>    │     │
│  │                                                      │     │
│  │  event_translator.rs                                 │     │
│  │    APXM StreamChunk → CodexApxmEvent                 │     │
│  │    • Token → TextDelta                               │     │
│  │    • ToolCallStart → ToolCallStarted                 │     │
│  │    • Done(response) → ResponseCompleted              │     │
│  │                                                      │     │
│  │  sandbox_bridge.rs                                   │     │
│  │    CodexSandboxBridge implements SandboxBackend       │     │
│  │    ExecRequest → CommandSpec → SandboxManager         │     │
│  │    → transform() → execute_env() → ExecResult        │     │
│  │                                                      │     │
│  │  notification_bridge.rs                              │     │
│  │    CodexApxmEvent → ServerNotification (for TUI)     │     │
│  └──────────────────────────────────────────────────────┘     │
│                                                              │
│  Three Axes Wired:                                           │
│  ┌─────────┐ ┌────────────────┐ ┌──────────────────────┐     │
│  │LLMBackend│ │CapabilityExec  │ │SandboxBackend        │     │
│  │          │ │                │ │                      │     │
│  │OpenAI/   │ │Codex tools via │ │CodexSandboxBridge    │     │
│  │Anthropic/│ │ToolCallRuntime │ │  bwrap + seccomp     │     │
│  │Ollama    │ │                │ │  Landlock + Seatbelt │     │
│  │          │ │                │ │  Container isolation │     │
│  └─────────┘ └────────────────┘ └──────────────────────┘     │
└──────────────────────────────────────────────────────────────┘
```

### 2.2 Three Axes Status

| Axis | How It Connects | Status |
|------|----------------|--------|
| **LLMBackend** | `ApxmModelClient` wraps `LLMRegistry`. Codex `Prompt` → `LLMRequest` via `request_translator`. `StreamChunk` → `CodexApxmEvent` via `event_translator`. | Connected (streaming works) |
| **SandboxBackend** | `CodexSandboxBridge` implements `SandboxBackend`. Translates `ExecRequest` → `CommandSpec` → `SandboxManager.transform()` → `execute_env()`. Reports `IsolationLevel::Container`. | Connected |
| **Capabilities** | Codex tools (`Function`, `Freeform`) forwarded in `request_translator`. Built-in tools (LocalShell, WebSearch) handled by Codex layer. | Partial — tool calls from APXM not yet dispatched |

### 2.3 Known Bug: Tool Call Discarding

**Location**: `codex.rs`, `run_apxm_sampling_request()`, lines ~7538-7557

```rust
CodexApxmEvent::ResponseCompleted { tool_calls, content, .. } => {
    if !tool_calls.is_empty() {
        has_tool_calls = true;  // ← flag set
        // ← but tool_calls Vec<CodexToolCall> is DROPPED
        // ← no ToolRouter::build_tool_call()
        // ← no ToolCallRuntime.handle_tool_call()
        // ← no conversation history recording
    }
}
```

**Impact**: APXM-backed models cannot execute tools. The fix requires routing
each `CodexToolCall` through Codex's existing `ToolRouter` → `ToolCallRuntime`
pipeline, matching the non-APXM path in `stream_events_utils.rs:196-227`.

### 2.4 AAM Potential for Codex

| AAM Component | Codex Mapping | Benefit |
|--------------|---------------|---------|
| **Beliefs** | Editor state (file contents, cursor, project structure) | Persistent context across turns; recoverable on crash |
| **Goals** | Refactoring objectives, bug fix targets | Formal goal tracking with completion/failure status |
| **Capabilities** | Registered tools (shell, read, write, search) | Dynamic tool discovery; capability-aware planning |
| **Transitions** | Every state change recorded with timestamp | Explainable reasoning; audit trail for code changes |
| **Checkpoint** | `AamCheckpoint.save_to_file()` | Resume after IDE restart with full agent context |
| **Scoping** | Sub-agent delegation (e.g., research task) | Isolated state for delegated work; results merged back |

---

## 3. Gemini CLI Integration (TypeScript + IPC Bridge)

### 3.1 Architecture

Gemini is TypeScript — requires IPC bridges to APXM's Rust runtime. Two
communication channels: subprocess for sandbox, HTTP+SSE for LLM.

```
┌──────────────────────────────────────────────────────────────┐
│ GEMINI CLI (TypeScript/Node.js)                               │
│                                                               │
│  ┌───────────────────────────────────────────────────────┐    │
│  │ Session (GeminiChat + LocalAgentExecutor)              │    │
│  │  ├── contentGenerator: ContentGenerator               │    │
│  │  ├── toolRegistry: ToolRegistry                       │    │
│  │  └── sandboxManager: SandboxManager                   │    │
│  └──────────┬────────────────────────────────────────────┘    │
│             │                                                 │
│  ┌──────────┴──────────────────────────────────────────┐      │
│  │ APXM TypeScript Layer                                │      │
│  │  packages/core/src/core/apxm/                        │      │
│  │                                                      │      │
│  │  client.ts          → HTTP+SSE to apxm-server        │      │
│  │  service-manager.ts → spawns apxm-server process     │      │
│  │  event-translator.ts→ ApxmEvent → GeminiStreamEvent  │      │
│  │  types.ts           → ApxmGenerateRequest types      │      │
│  └──────────────────────────────────────────────────────┘      │
│                                                               │
│  ┌──────────────────────────────────────────────────────┐      │
│  │ apxmContentGenerator.ts (1312 lines)                 │      │
│  │  Reads ~/.apxm/config.toml        │      │
│  │  Routes to OpenAI/Anthropic/Google/Ollama providers   │      │
│  └──────────────────────────────────────────────────────┘      │
└───────────┬──────────────────────────┬────────────────────────┘
            │ HTTP+SSE                  │ subprocess IPC
            ▼                          ▼
┌───────────────────┐    ┌────────────────────────────────────┐
│ apxm-server       │    │ gemini-sandbox-bridge (Rust)       │
│ (Rust binary)     │    │                                    │
│                   │    │  lib.rs: GeminiSandboxBridge       │
│ POST /v1/generate │    │    implements SandboxBackend       │
│ Accept: text/sse  │    │                                    │
│                   │    │  ipc.rs: BridgeConfig, probe(),    │
│ Routes to APXM    │    │    prepare_command()               │
│ LLMRegistry       │    │                                    │
│ backends           │    │  translate.rs: ExecRequest ↔      │
│                   │    │    GeminiSandboxRequest             │
└───────────────────┘    │                                    │
                         │  scripts/bridge.mjs:               │
                         │    --probe → platform detection     │
                         │    --prepare → SandboxManager       │
                         │      .prepareCommand()              │
                         └────────────────────────────────────┘
                                        │
                                        ▼
                         ┌────────────────────────────────────┐
                         │ Gemini SandboxManager (TypeScript)  │
                         │                                    │
                         │  Linux:   LinuxSandboxManager      │
                         │           bwrap + seccomp BPF      │
                         │  macOS:   MacOsSandboxManager      │
                         │           Seatbelt profiles        │
                         │  Windows: WindowsSandboxManager    │
                         │           Restricted tokens        │
                         │  Noop:    NoopSandboxManager       │
                         │           Env sanitization only    │
                         └────────────────────────────────────┘
```

### 3.2 Three Axes Status

| Axis | How It Connects | Status |
|------|----------------|--------|
| **LLMBackend** | `ApxmServiceClient` → HTTP+SSE to `apxm-server`. `ApxmGenerateRequest` → `LLMRequest`. `ApxmEvent` → `ServerGeminiStreamEvent` via `event-translator.ts`. Also: `apxmContentGenerator.ts` for direct provider dispatch. | 80% (streaming incomplete) |
| **SandboxBackend** | `GeminiSandboxBridge` (Rust) → subprocess IPC to `bridge.mjs` → Gemini's TypeScript `SandboxManager.prepareCommand()`. Two-phase: probe (platform detection) + prepare (command wrapping). | Complete |
| **Capabilities** | `DeclarativeTool` + `ToolRegistry` in Gemini. Not yet connected to APXM's `CapabilitySystem`. | 0% (planned Phase 2) |

### 3.3 IPC Protocol Details

**Sandbox IPC** (subprocess, per-execution):

```
Rust (GeminiSandboxBridge)          Node.js (bridge.mjs)
        │                                 │
        ├─── spawn: node bridge.mjs ──────┤
        │    --probe                       │
        │                                 │
        │◄── stdout JSON ─────────────────┤
        │    {"platform":"linux",          │
        │     "sandbox_type":"bubblewrap", │
        │     "version":"1.0"}            │
        │                                 │
        ├─── spawn: node bridge.mjs ──────┤
        │    --prepare                     │
        │    stdin: ExecRequest JSON       │
        │                                 │
        │◄── stdout JSON ─────────────────┤
        │    {"ok":true,                   │
        │     "command":"bwrap",           │
        │     "args":["--unshare-all",...],│
        │     "env":{"PATH":"/usr/bin"}}  │
        │                                 │
        ├─── spawn: bwrap ... ────────────X (actual sandboxed execution)
        │    capture stdout/stderr         │
        │    enforce timeout               │
        │    → ExecResult                  │
```

**LLM IPC** (HTTP+SSE, persistent service):

```
Gemini CLI (TypeScript)             apxm-server (Rust)
        │                                 │
        ├─── POST /v1/generate ───────────┤
        │    Content-Type: application/json│
        │    Accept: text/event-stream     │
        │                                 │
        │    {"messages":[...],            │
        │     "model":"claude-sonnet-4-6",│
        │     "backend":"anthropic",       │
        │     "tools":[...]}              │
        │                                 │
        │◄── SSE stream ─────────────────┤
        │    data: {"type":"content_delta",│
        │           "text":"Hello"}        │
        │    data: {"type":"tool_call_...",│
        │           "id":"tc_1","name":..}│
        │    data: {"type":"done",         │
        │           "response":{...}}      │
        │                                 │
```

### 3.4 AAM Potential for Gemini

| AAM Component | Gemini Current State | AAM Mapping |
|--------------|---------------------|-------------|
| **Beliefs** | Scattered: GeminiChat (history), LocalAgentExecutor (turn count), scheduler (pending confirms) | Unified `B` map: `conversation`, `model`, `turn_count`, `pending_confirms`, `compression_state` |
| **Goals** | Implicit: user query is the goal, no formal tracking | Explicit `G` queue: root goal = user query, sub-goals for delegated tasks |
| **Capabilities** | ToolRegistry + MCP discovery | Unified `C` map: built-in tools + MCP + agent delegation |
| **Transitions** | No audit trail | Every belief/goal/capability change recorded with timestamp |
| **Checkpoint** | Chat history in files, no formal snapshot | `AamCheckpoint` for full state persistence |
| **Scoping** | Sub-agents get full parent context | Configurable: Inherit/Isolate/Snapshot/Filter per scope |

---

## 4. Codex vs Gemini: Integration Comparison

```
                    CODEX (Rust)              GEMINI (TypeScript)
                    ────────────              ───────────────────
Language match      Same (Rust)               Different (TS↔Rust)
                    Direct trait impl         Subprocess/HTTP IPC

LLM Backend         In-process               HTTP+SSE sidecar
                    Arc<dyn LLMBackend>       POST /v1/generate
                    ~0 IPC overhead           ~1-5ms overhead

Sandbox Backend     In-process               Subprocess IPC
                    impl SandboxBackend       node bridge.mjs
                    CommandSpec → execute_env  JSON stdin → stdout
                    Container isolation       Platform-dependent

Tool Pipeline       ToolRouter →              DeclarativeTool →
                    ToolCallRuntime →          scheduler →
                    handle_tool_call()        confirmation flow

Failure Mode        Monolithic crash          Graceful degradation
                    Restart IDE               Sidecar auto-restart

AAM Potential       Persistent session state  Unified scattered state
                    Checkpoint on crash       File-tree backed

Integration Files   7 files                  4 TS + 3 Rust files
                    apxm_adapter/ module      apxm/ + apxm-bridge/
```

### 4.1 Performance Characteristics

| Operation | Codex | Gemini | Notes |
|-----------|-------|--------|-------|
| LLM request setup | ~1μs (in-process) | ~1-5ms (HTTP) | Dominated by LLM latency (100-2000ms) |
| Sandbox exec | ~1μs (in-process) | ~10-100ms (spawn node) | One-time per INV; tool exec dominates |
| State read/write | ~100ns (Arc<RwLock>) | N/A (not yet integrated) | Would require IPC for Gemini |
| Streaming tokens | Direct Stream<Item> | SSE parse per event | Both negligible vs LLM generation |

---

## 5. AAM Deep Dive: The State Separation

### 5.1 Why Separate State from Compute

APXM's PXM cleanly separates five concerns that current frameworks entangle
(Compute, Memory, State, Optimization, Scheduling). Each separation has deep
roots in program execution model research:

```
┌───────────────────────────────────────────────────────────┐
│              AIS GRAPH → Compiler → Runtime                 │
│                                                           │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐     │
│  │  ASK    │→│  THINK  │→│  INV    │→│  VERIFY │     │
│  │(compute)│  │(compute)│  │(compute)│  │(compute)│     │
│  └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘     │
│       │            │            │            │           │
│  ┌────┴────────────┴────────────┴────────────┴────┐      │
│  │              AAM STATE (B, G, C)                │      │
│  │                                                 │      │
│  │  Beliefs: {"task": "fix bug", "file": "x.rs"}  │      │
│  │  Goals:   [Goal("fix_bug", Active, priority=1)] │      │
│  │  Caps:    {"bash": ..., "read_file": ...}       │      │
│  │                                                 │      │
│  │  Every op is a transition: δ(AAM, op) → AAM'    │      │
│  └────────────────────────┬────────────────────────┘      │
│                           │                               │
│  ┌────────────────────────┴────────────────────────┐      │
│  │              MEMORY (STM / LTM / Episodic)       │      │
│  │                                                  │      │
│  │  STM: working memory for current turn            │      │
│  │  LTM: persistent knowledge across sessions       │      │
│  │  Episodic: transition log for REFLECT            │      │
│  └──────────────────────────────────────────────────┘      │
└───────────────────────────────────────────────────────────┘
```

**Benefits of explicit state**:

1. **Compiler optimization**: If two operations touch disjoint belief keys,
   they can execute in parallel (no data dependency)
2. **Checkpointing**: `AamCheckpoint.save_to_file()` captures full agent
   state for crash recovery
3. **Multi-agent isolation**: `ScopeSpec` controls state visibility between
   parent/child agents (Inherit/Isolate/Snapshot/Filter)
4. **Auditability**: `TransitionRecord` logs every mutation with timestamp,
   enabling REFLECT operations to analyze behavior

### 5.2 AAM Scoping for Multi-Agent

```
┌─────────────────────────────────────────────────┐
│ Root Agent (parent scope)                        │
│                                                  │
│  AAM = (B, G, C)                                 │
│  B: {"task": "refactor auth", "codebase": "..."}│
│  G: [Goal("refactor_auth", Active)]              │
│  C: {"bash", "read_file", "write_file"}          │
│                                                  │
│  DELEGATE("research", task="find auth patterns") │
│      │                                           │
│      ▼ ScopeSpec::snapshot_all()                 │
│  ┌───────────────────────────────────────┐       │
│  │ Child Agent (snapshot scope)           │       │
│  │                                       │       │
│  │  AAM' = snapshot of parent AAM        │       │
│  │  B': copy of parent beliefs           │       │
│  │  G': [Goal("find_auth_patterns")]     │       │
│  │  C': inherited capabilities           │       │
│  │                                       │       │
│  │  Writes to B' don't affect parent B   │       │
│  │  Result returned via DELEGATE output  │       │
│  └───────────────────────────────────────┘       │
│                                                  │
│  SPAWN("parallel_worker", ...)                   │
│      │                                           │
│      ▼ ScopeSpec::isolate_all()                  │
│  ┌───────────────────────────────────────┐       │
│  │ Spawned Agent (isolated scope)         │       │
│  │                                       │       │
│  │  AAM'' = empty (fresh state)          │       │
│  │  B'': {}                              │       │
│  │  G'': [Goal from SPAWN params]        │       │
│  │  C'': {}                              │       │
│  │                                       │       │
│  │  Cannot see parent state at all       │       │
│  │  Fully sandboxed execution            │       │
│  └───────────────────────────────────────┘       │
└─────────────────────────────────────────────────┘
```

### 5.3 AAM API for Hosts

```rust
// Read state
let beliefs = aam.beliefs();                    // HashMap<String, Value>
let top_goal = aam.top_goal();                  // Option<Goal>
let capabilities = aam.capabilities();          // HashMap<String, CapRecord>

// Write state (every write produces a TransitionRecord)
aam.set_belief("key", Value::String("val"), label);
aam.add_goal(goal, label);
aam.register_capability("bash", record, label);

// Scoping
let child_aam = aam.child_scope(&ScopeSpec::snapshot_all());

// Persistence
let checkpoint = aam.checkpoint();              // AamCheckpoint
checkpoint.save_to_file(path)?;                 // JSON serialization
let restored = AamCheckpoint::load_from_file(path)?;
aam.restore(&restored);

// Episodic memory
let history = aam.recent_transitions(10);       // Vec<TransitionRecord>
```

---

## 6. The Complete Integration Picture

```
┌──────────────────── HOST APPLICATION ────────────────────────┐
│                                                               │
│  Codex CLI (Rust)          OR          Gemini CLI (TypeScript) │
│  ┌─────────────────┐                  ┌─────────────────────┐ │
│  │ Session/Turn     │                  │ GeminiChat/Turn     │ │
│  │ ToolCallRuntime  │                  │ ToolRegistry        │ │
│  │ SandboxManager   │                  │ SandboxManager      │ │
│  └────────┬─────────┘                  └─────────┬───────────┘ │
│           │                                      │             │
│  Injects implementations:              Injects via IPC:       │
│  ┌────────┴─────────┐                  ┌─────────┴───────────┐ │
│  │ ApxmModelClient   │                  │ ApxmServiceClient   │ │
│  │ (in-process)      │                  │ (HTTP+SSE)          │ │
│  │                   │                  │                     │ │
│  │ CodexSandboxBridge│                  │ GeminiSandboxBridge │ │
│  │ (in-process)      │                  │ (subprocess IPC)    │ │
│  │                   │                  │                     │ │
│  │ Codex tools       │                  │ Gemini tools        │ │
│  │ (ToolCallRuntime) │                  │ (DeclarativeTool)   │ │
│  └────────┬──────────┘                  └─────────┬───────────┘ │
└───────────┼──────────────────────────────────────┼─────────────┘
            │                                      │
            └──────────────┬───────────────────────┘
                           │
                           ▼
            ┌──────────────────────────────┐
            │    APXM (Compiler + Runtime)  │
            │                              │
            │  ┌─ Compiler (MLIR) ───────┐ │
            │  │  Graph → .apxmobj       │ │
            │  │  FuseAskOps, CSE, DCE   │ │
            │  │  Parallelism extraction  │ │
            │  └─────────────────────────┘ │
            │                              │
            │  ┌─ Runtime ───────────────┐ │
            │  │ LLMRegistry─LLMBackend  │ │
            │  │   (OpenAI, Anthropic..) │ │
            │  │                         │ │
            │  │ CapabilitySystem        │ │
            │  │   ├─ to_exec_request()? │ │
            │  │   │  YES→SandboxBackend │◄── Host sandbox
            │  │   │  NO →direct exec    │ │
            │  │                         │ │
            │  │ SandboxRegistry         │ │
            │  │   └─ select(isolation)  │ │
            │  │                         │ │
            │  │ AAM (B, G, C)           │ │
            │  │   └─ transitions        │ │
            │  │   └─ scoping            │ │
            │  │   └─ checkpoint/restore │ │
            │  │                         │ │
            │  │ Memory (STM/LTM/Episod.)│ │
            │  │   └─ backs beliefs      │ │
            │  │   └─ REFLECT reads      │ │
            │  └─────────────────────────┘ │
            └──────────────────────────────┘
```

---

## 7. Current Status & Remaining Work

### Completed

| Item | Tests | Location |
|------|-------|----------|
| `apxm-sandbox` crate (trait + types) | 29 | `apxm/crates/apxm-sandbox/` (src: 15 unit, tests: 14 integration) |
| `ProcessSandboxBackend` (APXM default) | 7 | `apxm-driver/src/runtime/sandbox.rs` |
| Sandbox registry integration tests | 14 | `apxm-sandbox/tests/sandbox_integration.rs` |
| Sandbox e2e tests | 7 | `apxm-driver/tests/sandbox_e2e.rs` |
| Sandbox wiring fix (INV → SandboxBackend) | — | `CapabilitySystem.invoke_with_timeout()` routes through backend |
| `CodexSandboxBridge` | — | `codex-rs/core/src/apxm_adapter/sandbox_bridge.rs` (tests in `tests.rs`) |
| `GeminiSandboxBridge` | 19 | `google/gemini-cli/packages/apxm-bridge/src/lib.rs` |
| Codex LLM adapter (request/event/notification) | 65 | `codex-rs/core/src/apxm_adapter/` (7 files, tests in `tests.rs`) |
| Gemini LLM adapter (TypeScript) | — | `gemini-cli/packages/core/src/core/apxm/` |

### Remaining

| Item | Priority | Description |
|------|----------|-------------|
| **Codex tool call dispatch** | P0 | Fix `run_apxm_sampling_request()` — route `CodexToolCall` through `ToolRouter` → `ToolCallRuntime` |
| **Gemini streaming** | P1 | Complete HTTP+SSE streaming in `ApxmServiceClient` |
| **Gemini capability registration** | P2 | Register `DeclarativeTool`s with APXM `CapabilitySystem` |
| **AAM ↔ Codex session** | P2 | Expose AAM beliefs/goals to Codex session for persistent state |
| **AAM ↔ Gemini state** | P3 | Unify scattered state (GeminiChat, LocalAgentExecutor) into AAM |
| **AgentMate sandbox** | P3 | Replace `am-sandbox` internals with `apxm-sandbox` trait |
| **Compiler integration** | P3 | Express host agent loops as AIS graphs for MLIR optimization (FuseAskOps, parallelism extraction) |
