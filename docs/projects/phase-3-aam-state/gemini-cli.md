# Phase 3: Gemini-CLI Changes -- AAM State Model

**Source:** [Plan 3: Gemini-CLI Changes](../plan-3-gemini-changes.md), Phase 3 (G9-G10)
**Draft:** v6 -- Revised with investigation findings
**Timeline:** 6-8 days (increased from 6; cross-language bridge adds complexity)
**Dependencies:** Phase 2 (G6-G8: Capability registration, parallel calls, PolicyEngine interceptors) complete; Plan 1 delivers hierarchical AAM (scoped state, file-backed)

---

## Goal

Map Gemini-CLI's scattered internal state to AAM's formal `(B, G, C)` triple, backed by the hierarchical file tree under `~/.apxm/workspaces/`. Enable `apxm state show <session>` for any Gemini-CLI session.

### What Changes (Phase 3)

- Session state becomes typed, inspectable AAM
- Three-tier memory replaces ad-hoc in-memory state
- Agent sessions become diffable, persistent, auditable

### What Is NOT Changed (Phase 3)

- `executeTurn()` loop structure (still imperative -- graph form is Phase 4)
- Tool dispatch mechanism (still via APXM capabilities from Phase 2)
- TUI rendering

---

## G9: Map Gemini-CLI session state to AAM (B, G, C) (4 days)

> **Note: State is spread across multiple classes.** The relevant state lives on both `GeminiChat` AND `LocalAgentExecutor`, not just `GeminiChat`:
>
> - **`GeminiChat`** (`geminiChat.ts:249`): `history: Content[]`, `systemInstruction: string`, `tools: Tool[]`
> - **`LocalAgentExecutor`** (`local-executor.ts`): `turnCounter` (local variable at line 319), `compressionService: ChatCompressionService`, `toolRegistry: ToolRegistry`, `promptRegistry`, `resourceRegistry`
> - **`LoopDetectionService`** (`loopDetectionService.ts`): loop thresholds, counters, LLM check intervals
>
> The AAM mapping must pull from all three classes, not just `GeminiChat`.

### G9.1 Beliefs mapping

Every piece of Gemini-CLI session state maps to a typed AAM Belief:

| Gemini-CLI State | AAM Component | Typed Key | Source |
|-----------------|--------------|-----------|--------|
| Conversation history | **B** (Beliefs) | `conversation: Vec<Content>` | `GeminiChat` |
| Active model / provider | **B** (Beliefs) | `model: ModelConfig` | `apxmConfig` resolution |
| Pending tool confirmations | **B** (Beliefs) | `pending_confirms: Vec<ToolCallId>` | Scheduler state |
| Compression state | **B** (Beliefs) | `compression: CompressionStatus` | `ChatCompressionService` |
| Turn counter | **B** (Beliefs) | `turn_count: number` | `LocalAgentExecutor` |
| User hints / injections | **B** (Beliefs) | `user_hints: Vec<String>` | Injection service |
| Background completions | **B** (Beliefs) | `bg_completions: Vec<String>` | Fast-ack helper |

### G9.2 Goals mapping

| Gemini-CLI Concept | AAM Component | Representation |
|-------------------|--------------|----------------|
| User query / task | **G** (Goals) | `Goal("answer_user_query", priority=1)` |
| Sub-agent task | **G** (Goals) | `Goal(definition.query, parent=root_goal)` |
| `complete_task` tool | **G** (Goals) | Goal completion signal |

### G9.3 Capabilities mapping

| Gemini-CLI Concept | AAM Component | Representation |
|-------------------|--------------|----------------|
| Built-in tools | **C** (Capabilities) | `Map<ToolName, ToolSchema>` |
| MCP-discovered tools | **C** (Capabilities) | `Map<McpToolName, McpToolSchema>` |
| Agent delegation | **C** (Capabilities) | `Map<AgentName, AgentSignature>` |
| LLM provider | **C** (Capabilities) | `Map<"llm", ModelCapability>` |

### G9.4 File tree structure

The file tree IS the AAM. A Gemini-CLI session produces:

```
~/.apxm/workspaces/gemini-session-<id>/
  data/                         B (Beliefs)
    conversation.json           - Chat history
    pending_tools.json          - In-flight tool calls
    compression.json            - Compression state
    turn_state.json             - Turn counter, hints, bg completions
  goals.toml                    G (Goals)
    goal = "Fix the failing test"
    priority = 1
  tools/                        C (Capabilities)
    shell_command.toml           - Shell execution capability
    read_file.toml              - File read capability
    mcp/                        - MCP-discovered tools
      filesystem.toml
```

**Deliverable:** `apxm state show gemini-session-<id>` prints the full AAM snapshot for any Gemini-CLI session.

---

## G10: Three-tier memory (2 days)

APXM's memory hierarchy maps directly to Gemini-CLI's data patterns:

| Memory Tier | Gemini-CLI Equivalent | What It Holds |
|------------|----------------------|--------------|
| **STM** (Short-Term, ~us) | Current turn context, recent tool output | Working memory for the active turn -- intermediate results that don't survive session restart |
| **LTM** (Long-Term, ~ms) | Environment memory, project memory, JIT context | Persistent user preferences, project facts, cached tool results that survive across sessions |
| **Episodic** (append-only) | *(not currently tracked)* | Execution trace: every tool call, LLM response, and state transition -- the substrate for REFLECT and loop detection |

### G10.1 STM integration

Current turn context (working message, partial tool results) stores in APXM STM. Microsecond access. Volatile.

### G10.2 LTM integration

Gemini-CLI's environment memory and project memory map to APXM LTM (SQLite-backed at `~/.apxm/memory/ltm.sqlite`). Persistent across sessions.

### G10.3 Episodic memory

Every tool call, LLM response, and state transition is appended to the episodic trace (`~/.apxm/memory/episodes.jsonl`). This is the substrate for:
- Loop detection (see warning below) --> formalized as REFLECT on episodic trace
- Debugging (currently "attach debugger") --> `apxm state show` on episodic trace
- Audit (currently ad-hoc logging) --> typed, structured execution history

> **WARNING: Loop detection refactor is high risk.** The current `LoopDetectionService` (`loopDetectionService.ts`) is 760 lines of carefully tuned heuristics:
>
> - SHA-256 tool call tracking (`checkToolCallLoop`, line 314) -- detects consecutive identical tool invocations
> - Sliding window content comparison (`checkContentLoop`, line 339) -- detects content chanting/repetition
> - LLM-based loop detection with confidence scoring (`checkForLoopWithLLM`, line 540) -- catches subtle semantic loops
> - Three event types: `CONSECUTIVE_IDENTICAL_TOOL_CALLS`, `CONTENT_CHANTING_LOOP`, `LLM_DETECTED_LOOP`
>
> Replacing this with REFLECT on episodic trace is a **significant behavioral change** with regression risk. The existing heuristics have been tuned in production. A phased approach is recommended: (1) first, populate episodic trace alongside existing loop detection, (2) validate that REFLECT-based detection matches existing detection rates, (3) only then cut over.

---

## Cross-Language Bridge

**RESOLVED:** The cross-language bridge uses a tiered approach matched to each phase:

| Phase | Bridge | Rationale |
|-------|--------|-----------|
| **1-3** | **Shared file format** + **NAPI module** | TypeScript reads/writes `~/.apxm/workspaces/` files (TOML/JSON) directly. For operations requiring Rust-side logic (scoping, materialization), use `napi-rs` to bind APXM Rust APIs into Node.js in-process. Precedent: AgentMate uses PyO3 for Python. |
| **1-3 (fallback)** | **Subprocess** | Shell out to `apxm state show <id> --json`, `apxm state write <id> --json`. Simple, reliable, process-per-call overhead. |
| **4+** | **HTTP via apxm-server** | `POST /v1/execute` for graph execution, `POST /v1/state/<session>` for state operations. Adds latency but enables multi-consumer coordination and SSE streaming. |

**Phase 3 specifically** uses the shared file format approach: the workspace schema is documented, stable, and human-readable (TOML/JSON). TypeScript can read/write these files directly without any bridge overhead. NAPI bindings are optional for Phase 3 — they become more important in Phases 1-2 where `LLMRegistry` and `CapabilitySystem` APIs must be called from TypeScript.

**`apxm-server` is NOT used in Phases 1-3.** The server is a graph execution gateway (not an LLM proxy — there is no `/v1/generate-stream` endpoint). It enters the migration at Phase 4 where graph execution over HTTP adds genuine value. See [Sessions & Server Investigation](../SESSIONS-AND-SERVER-INVESTIGATION.md) for the full analysis.

---

## Phase 3 Summary

| Task | Days | Key Deliverable |
|------|------|----------------|
| G9: AAM state mapping | 4 | Full (B, G, C) mapping from GeminiChat + LocalAgentExecutor, file-tree backing |
| G10: Three-tier memory | 2-4 | STM/LTM/Episodic for Gemini-CLI sessions; cross-language bridge |
| **Total** | **6-8** | -- |

---

## Phase 3 Validation

- [ ] `apxm state show <session>` works for Gemini-CLI sessions
- [ ] AAM (B, G, C) backed by file tree under `~/.apxm/workspaces/`
- [ ] Beliefs survive session restart (via file persistence)
- [ ] Episodic trace records every tool call and LLM response
- [ ] Loop detection reads from episodic trace (not ad-hoc pattern matching)
- [ ] Three-tier memory operational (STM for current turn, LTM for project memory, Episodic for trace)

---

## Cross-references

- **APXM substrate (A6):** [apxm.md](apxm.md) -- WorkspaceManager, Materializer, StateProjector
- **Codex consumer (C8-C9):** [codex.md](codex.md) -- parallel AAM mapping for Codex
- **Session → AAM mapping:** [SESSION-AAM-MAPPING.md](../SESSION-AAM-MAPPING.md) -- detailed field-level mapping for both consumers
- **Sessions & server investigation:** [SESSIONS-AND-SERVER-INVESTIGATION.md](../SESSIONS-AND-SERVER-INVESTIGATION.md) -- session isolation, cross-language bridge analysis
- **AAM formal model:** [pxm/aam.md](../../pxm/aam.md)
- **Hierarchical AAM design:** [implementation/runtime/hierarchical-aam.md](../../implementation/runtime/hierarchical-aam.md)
- **Phase overview:** [README.md](README.md)
