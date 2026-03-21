# Phase 3: Codex Changes -- AAM State Model

**Source:** [Plan 2: Codex Changes](../plan-2-codex-changes.md), Phase 3 (C8-C9)
**Draft:** v6 -- Revised with investigation findings
**Timeline:** 5 days
**Dependencies:** Phase 2 (C5-C7: Tool adapters, Guardian interceptor, tool persistence) complete; Plan 1 delivers hierarchical AAM with scoped state, file-backed persistence, `apxm state show` CLI command

---

## Goal

Map Codex's `SessionState` and `TurnState` to the AAM triple `(B, G, C)`, backed by the file tree under `~/.apxm/workspaces/`. Agent state becomes inspectable, diffable, and persistent.

---

## C8: State Mapping (3 days)

Map Codex's existing state structures to AAM components:

| Codex State | AAM Component | File Representation | Notes |
|-------------|--------------|---------------------|-------|
| Conversation history | **B** (Beliefs) | `data/conversation.json` | Via `ContextManager` |
| Current model/provider | **C** (Capabilities) | `tools/llm.toml` | Via `SessionConfiguration` |
| Pending tool calls (metadata only) | **B** (Beliefs) | `data/pending_tools.json` | See warning below |
| Approval decisions | **B** (Beliefs) | `data/approvals.json` | `granted_permissions` |
| Tool call count (per turn) | **B** (Beliefs) | `data/session_state.json` | `TurnState.tool_calls: u64`; no explicit turn counter on `SessionState` |
| Active instructions | **G** (Goals) | `goals.toml` | Via `SessionConfiguration` |
| Available tools | **C** (Capabilities) | `tools/*.toml` | External tool registry |
| System prompt | **B** (Beliefs) | `data/system_prompt.md` | Via `SessionConfiguration` |

> **WARNING: Non-serializable TurnState channels.** `TurnState` stores `oneshot::Sender<ReviewDecision>` for pending approvals and `oneshot::Sender<DynamicToolResponse>` for pending dynamic tools (`state/turn.rs:77-87`). These are Tokio channel handles -- they are **not serializable**. The state bridge must separate serializable metadata (request info, timestamps, approval status) from live coordination handles. The `data/pending_tools.json` file should capture the request details and status, not the channel itself.
>
> **Note: No explicit turn counter.** `SessionState` has no `turn_count` field. `TurnState` has `tool_calls: u64` (a per-turn call counter). Turn counting happens at a different level (task/turn management). The bridge should either derive a turn count from session history length or add an explicit counter.

### The file tree IS the AAM

```
~/.apxm/workspaces/<codex-session-id>/
+-- data/                              B (Beliefs)
|   +-- conversation.json              - Chat history
|   +-- pending_tools.json             - In-flight tool calls
|   +-- approvals.json                 - Cached approval decisions
|   +-- session_state.json             - Turn count, model config, etc.
+-- goals.toml                         G (Goals)
|   goal = "Fix the failing test"
|   priority = 1
+-- tools/                             C (Capabilities)
    +-- shell.toml                     - Shell execution capability
    +-- read_file.toml                 - File read capability
    +-- apply_patch.toml               - File mutation capability
    +-- mcp/                           - MCP-discovered tools
        +-- filesystem.toml
```

### Implementation approach

1. At session start, create a workspace directory under `~/.apxm/workspaces/` keyed by Codex session ID
2. Write initial AAM state: beliefs from config, goals from instructions, capabilities from tool registry
3. On each state mutation (`SessionState` or `TurnState` update), write the corresponding file
4. On session resume, load AAM state from the workspace directory

### Deliverables

- `core/src/apxm_adapter/state_bridge.rs` -- bidirectional `SessionState` <-> AAM mapping (~200 lines)
- Workspace lifecycle management (create, populate, read) (~100 lines)
- `apxm state show <session-id>` works for Codex sessions

---

## C9: Memory Tier Integration (2 days)

Map Codex's context compaction to APXM's three-tier memory:

| Codex Memory Concept | APXM Memory Tier |
|---------------------|-----------------|
| Current turn working context | **STM** -- in-memory per-execution scratch |
| Session memories (persisted across turns) | **Episodic** -- append-only event log |
| Persistent memories (cross-session) | **LTM** -- SQLite persistence at `~/.apxm/memory/ltm.sqlite` |
| Context compaction (2-phase) | **QMEM** (query) + **UMEM** (update) on memory tiers |

Codex's 2-phase context compaction pipeline (extract with gpt-5.1-codex-mini, consolidate with gpt-5.3-codex) maps to QMEM + UMEM nodes in the AIS graph, making the compaction visible to the compiler as a first-class operation rather than an opaque function call.

### Deliverables

- STM populated from `TurnState` at turn start
- LTM bridge for persistent memories across sessions
- Episodic memory populated from conversation history
- QMEM/UMEM operations exposed for context compaction

---

## Phase 3 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C8: State mapping | 3 | 2 | 2 | ~400 |
| C9: Memory tiers | 2 | 1 | 1 | ~200 |
| **Total** | **5** | **3** | **3** | **~600** |

---

## Cross-references

- **APXM substrate (A6):** [apxm.md](apxm.md) -- WorkspaceManager, Materializer, StateProjector
- **Gemini-CLI consumer (G9-G10):** [gemini-cli.md](gemini-cli.md) -- parallel AAM mapping for Gemini-CLI
- **Session → AAM mapping:** [SESSION-AAM-MAPPING.md](../SESSION-AAM-MAPPING.md) -- detailed field-level mapping with non-serializable exclusions
- **Sessions & server investigation:** [SESSIONS-AND-SERVER-INVESTIGATION.md](../SESSIONS-AND-SERVER-INVESTIGATION.md) -- session isolation architecture
- **AAM formal model:** [pxm/aam.md](../../pxm/aam.md)
- **Hierarchical AAM design:** [implementation/runtime/hierarchical-aam.md](../../implementation/runtime/hierarchical-aam.md)
- **Phase overview:** [README.md](README.md)
