# Runtime Primitives for Coding Agents

Every coding agent — Codex, Claude Code, Aider, Cursor — needs the same seven runtime primitives. They differ in how they compose them, not in what they need.

## The Seven Primitives

### 1. Inference Loop

Call an LLM, process its response, call it again with updated context. This is the core agent loop.

**AIS mapping**: `ASK` with tool capabilities. The `llm.rs` handler iterates (up to `MAX_TOOL_ITERATIONS = 10`): call LLM → extract tool calls → execute tools sequentially → append results → call LLM again → repeat until the model stops requesting tools.

**What A-PXM adds**: The loop mechanics are handled by the runtime. The developer declares the `ASK` node with its tools; the handler manages iteration, token budget enforcement via `charge_tokens()`, and retries with exponential backoff. Note: the max iteration count is currently hardcoded at 10 (P1 gap -- Codex-class agents may need 25+).

### 2. Tool Dispatch

Execute file operations, shell commands, search, and other capabilities. This is what makes an agent agentic — it can act, not just talk.

**AIS mapping**: `INV` nodes invoke registered capabilities via `CapabilitySystem.invoke_with_timeout()`. Independent `INV` nodes run in parallel automatically (the dataflow scheduler sees no data edge between them and dispatches them to concurrent workers).

**What A-PXM adds**: Automatic inter-node parallelism (no manual `FuturesOrdered`), capability interceptors (auditable pre/post hooks with Allow/Deny/EditArgs), and typed tool schemas (validated against JSON Schema before execution). Note: tool calls *within* an ASK's tool loop still execute sequentially (P0 gap in `implementation/TODO.md`).

### 3. Context Management

Track token usage, enforce budgets, compact history when it grows too large. Without this, agents exhaust context windows and produce garbage.

**AIS mapping**: `token_budget` attribute on `ASK` nodes. The runtime enforces budget via `charge_tokens()` which tracks cumulative usage in an `AtomicU64` and returns an error when exceeded.

**What A-PXM adds**: Compiler-visible constraints (catch impossible budgets before execution). Note: automatic context compaction (summarizing intermediate results when budget is tight) is not yet implemented.

### 4. Memory

Maintain session state (what happened this conversation) and persistent knowledge (what the project looks like). Agents without memory repeat mistakes and lose context.

**AIS mapping**: Three-tier hierarchy — STM (in-memory `RwLock<HashMap>` via `InMemoryBackend`, per-execution), LTM (pluggable: SQLite, Redb, or in-memory, cross-session), Episodic (append-only event log). `QMEM` reads, `UMEM` writes.

**What A-PXM adds**: Formal memory tiers with defined semantics (not ad-hoc key-value stores) and checkpoint/restore of AAM state (beliefs + goals). Note: compiler-visible memory access pattern optimization (reorder reads, eliminate redundant writes) is not yet implemented.

### 5. Sandboxing

Isolate tool execution so a shell command cannot damage the host system. Essential for any agent that runs code.

**AIS mapping**: Capability interceptors (`CapabilityInterceptor` trait with `pre_invoke`/`post_invoke` hooks and `InterceptDecision::Allow/Deny/EditArgs`).

**What A-PXM adds**: Policy-driven access control as part of the execution model (not bolted on as a wrapper). Note: OS-level sandboxing (Seatbelt on macOS, Landlock on Linux) does not exist yet -- this is a P0 gap in `implementation/TODO.md`. Only application-level interception is implemented.

### 6. Multi-Agent Coordination

Spawn sub-agents, divide work, aggregate results. Complex tasks require multiple agents with different specializations.

**AIS mapping**: `FLOW_CALL` (invoke a sub-workflow), `Communicate` (inter-agent messaging with local/HTTP/broadcast protocols), `WAIT_ALL` / `MERGE` (synchronization barriers).

**What A-PXM adds**: The scheduler handles synchronization automatically with deadlock detection (watchdog timer). `Communicate` supports local in-process dispatch via `FlowRegistry`, HTTP dispatch to remote agents, and broadcast fan-out. Note: compiler-level topology validation (orphan agents, deadlock cycles) and hierarchical AAM scoping between agents are not yet implemented (P2 gap in `implementation/TODO.md`).

### 7. Streaming Output

Deliver tokens to the user as they arrive, not after the full response completes. This is a UX requirement — users need to see progress.

**AIS mapping**: `ExecutionEventEmitter` trait with `LlmToken`, `ToolStart`, `ToolEnd` events (3 event types in `executor/events.rs`).

**What A-PXM adds**: Structured event types (not raw text) and a pluggable emitter trait. Note: `LlmToken` is currently emitted after the full LLM response arrives, not during streaming (the `LLMBackend` trait has no `generate_stream` method -- P0 gap). Event type coverage is minimal (3 of ~15 needed types). Mid-stream cancellation is not implemented (no `CancellationToken` -- P1 gap).

## Why These Seven Are Universal

| Primitive | Codex | Claude Code | Aider | Cursor |
|-----------|-------|-------------|-------|--------|
| Inference loop | `run_turn` | `Agent.run()` | `Coder.run()` | `generate()` |
| Tool dispatch | `FuturesOrdered` | Sequential | File edit parser | LSP + shell |
| Context management | Token counting | Conversation trim | Chat history limit | Embedding retrieval |
| Memory | SQLite `SessionState` | `~/.claude/` files | `.aider.chat.history` | VS Code state |
| Sandboxing | Seatbelt/Landlock | None (trusted) | None (trusted) | VS Code sandbox |
| Multi-agent | Single agent | Single agent | Single agent | Single agent |
| Streaming | SSE events | Terminal stream | Terminal stream | Editor inline |

All four implement the same primitives with different mechanisms. A-PXM provides all seven as shared infrastructure — an improvement to any primitive benefits every agent on the platform.
