# Phase 3: AAM State Model

**Timeline:** Weeks 15-20
**Status:** Not started
**Draft:** v6 -- Revised with investigation findings
**Dependencies:** Phase 2 (Tool Migration) complete across all three plans

---

## Goal

Map each consumer's scattered internal state to the AAM triple `(B, G, C)`, backed by the hierarchical file tree under `~/.apxm/workspaces/`. Agent state becomes inspectable (`apxm state show <session>`), diffable (`git diff` on AAM files), and persistent across sessions.

**Key insight: "The file tree IS the AAM"** -- `data/` = Beliefs, `goals.toml` = Goals, `tools/` = Capabilities. Every piece of agent state maps to a concrete file that can be read, diffed, version-controlled, and restored.

**Universal AAM.** The AAM triple `(B, G, C)` and workspace file tree are framework-agnostic. Any agent framework can map its internal state to AAM, gaining inspectable (`apxm state show`), diffable (`git diff`), and persistent agent state. APXM provides the execution substrate; frameworks bring their own semantics.

```
~/.apxm/workspaces/<session-id>/
  scope.toml           # Scope metadata (id, parent, policy, revision)
  data/                # B (Beliefs) -- TOML/JSON data files
  goals/               # G (Goals) -- goal tree as TOML
    current.toml
  tools/               # C (Capabilities) -- tool definitions
    <registered-tools>.toml
```

---

## What Each Consumer Delivers

### APXM (Plan 1, A6) -- [apxm.md](apxm.md)

The substrate delivers the hierarchical AAM infrastructure that both consumers depend on:

- **WorkspaceManager** -- creates, opens, and archives workspace directories under `~/.apxm/workspaces/`
- **Materializer** -- writes in-memory AAM state to the file tree and hydrates it back
- **StateProjector** -- enforces scoping rules (Inherit, Isolate, Snapshot, Filter) between parent/child workspaces
- **Three-tier memory backing** -- STM (in-memory), LTM (per-workspace files), Episodic (per-workspace trace)
- **CLI `state` commands** -- `apxm state show`, `apxm state list`, `apxm state diff`

### Codex (Plan 2, C8-C9) -- [codex.md](codex.md)

Codex maps its `SessionState` and `TurnState` to the AAM triple:

- **C8: State mapping** -- bidirectional `SessionState` <-> AAM bridge; conversation history, pending tool calls, approval decisions, and instructions map to Beliefs/Goals/Capabilities files
- **C9: Memory tier integration** -- current turn context maps to STM, session memories to Episodic, persistent memories to LTM; Codex's 2-phase context compaction maps to QMEM + UMEM operations

### Gemini-CLI (Plan 3, G9-G10) -- [gemini-cli.md](gemini-cli.md)

Gemini-CLI maps its `GeminiChat` state to the AAM triple:

- **G9: GeminiChat -> AAM mapping** -- conversation history, model config, compression state, and tool confirmations map to Beliefs; user queries and sub-agent tasks map to Goals; built-in tools, MCP tools, and agent delegation map to Capabilities
- **G10: Three-tier memory** -- current turn context to STM, environment/project memory to LTM, execution trace to Episodic; loop detection formalized as REFLECT on episodic trace

---

## Gap Analysis: What Exists vs What Phase 3 Needs

### APXM Substrate (A6)

| Component | Current State | Phase 3 Target | Gap |
|-----------|--------------|----------------|-----|
| `ScopePolicy`, `ScopeSpec`, `GoalTree` types | COMPLETE | Same | None |
| `Aam::child_scope()` (in-memory scoping) | COMPLETE with 6 tests | Same | None |
| `ScopeRegistry` | COMPLETE with 8 tests | Same | None |
| `WorkspaceManager` | Stub (wraps ScopeRegistry) | Full directory management | Need `create_workspace`, `open_workspace`, `archive_workspace` with `~/.apxm/workspaces/` layout |
| `Materializer` | NOT STARTED | Write/read AAM to/from file tree | Full implementation needed |
| `StateProjector` | `child_scope()` handles creation direction | Also needs `promote()` (child -> parent) | `promote()` not implemented |
| Three-tier memory | COMPLETE (STM, LTM, Episodic all working) | Per-workspace scoping | Already has key-prefix scoping; needs per-workspace file paths |
| `SessionManager` | COMPLETE (checkpoint save/load) | Integrate with workspace dirs | Bridge between checkpoint JSON and workspace file tree |
| CLI `state` commands | NOT STARTED | `show`, `list`, `diff` | Full implementation needed |
| Effect analysis | COMPLETE (covers all 39 ops) | Same | None |

### Codex Consumer (C8-C9)

| Component | Current State | Phase 3 Target | Gap |
|-----------|--------------|----------------|-----|
| `SessionState` | Exists (`state/session.rs`) | Map to AAM | Need bridge adapter |
| `TurnState` | Exists (`state/turn.rs`) | Map to STM | Need adapter (excluding non-serializable channels) |
| `ContextManager` | Exists | Map history to Beliefs | Need serialization bridge |
| Turn counter | Not an explicit field | Add to `session_state.json` | Need to add or derive |
| Context compaction | Exists via `compact_remote.rs` | Map to QMEM/UMEM | Need AIS operation mapping |
| `state_bridge.rs` | NOT STARTED | Bidirectional mapping | Full implementation needed |

### Gemini-CLI Consumer (G9-G10)

| Component | Current State | Phase 3 Target | Gap |
|-----------|--------------|----------------|-----|
| `GeminiChat` | Exists (TypeScript) | Map to AAM | Need TypeScript/APXM bridge |
| `LocalAgentExecutor` | Exists (holds most state) | Map turn state | Need to centralize state |
| `ChatCompressionService` | Exists | Map to QMEM/UMEM | Need operation mapping |
| `LoopDetectionService` | Ad-hoc pattern matching (760 lines) | REFLECT on episodic trace | Significant refactor needed |
| Episodic trace | Not tracked | Append-only log | Full implementation needed |
| File-tree backing | NOT STARTED | `~/.apxm/workspaces/gemini-session-*` | Full implementation needed |

---

## Session → AAM Mapping

Each consumer session maps 1:1 to an isolated AAM scope. The detailed field-by-field mapping is in [SESSION-AAM-MAPPING.md](../SESSION-AAM-MAPPING.md). Key points:

- **Codex**: `SessionState` (conversation, approvals, pending tools, config) → B; user instruction → G; 21 tool handlers + MCP → C. Five `oneshot::Sender` channels in `TurnState` are non-serializable exclusions.
- **Gemini-CLI**: `GeminiChat` (conversation, compression, turn state) + `LocalAgentExecutor` (turn counter) → B; user query → G; built-in tools + MCP + agent delegation → C. Turn counter is currently a local variable that needs promotion to AAM.
- **Isolation**: Phases 1-3 use one Runtime per consumer (automatic isolation). Phase 4+ uses per-session `child_scope(ScopeSpec::isolate())` in `apxm-server`.

---

## Risk Matrix

### High Risk

1. **Non-serializable TurnState in Codex.** `TurnState` stores `oneshot::Sender<ReviewDecision>` for pending approvals -- these are Tokio channel handles, not serializable. The C8 state bridge must separate serializable metadata (request info, timestamps, status) from live coordination handles.

2. **Gemini-CLI loop detection refactor.** The current `LoopDetectionService` is 760 lines of carefully tuned heuristics (SHA-256 tool call tracking, sliding window content comparison, LLM-based detection). Replacing this with REFLECT on episodic trace is a significant behavioral change with regression risk.

3. **Cross-language bridge for Gemini-CLI.** Gemini-CLI is TypeScript; APXM workspaces are Rust-managed filesystem. **Recommended approach for Phase 3:** shared file format (both TypeScript and Rust read/write the same `~/.apxm/workspaces/` TOML/JSON files). NAPI module (`napi-rs`) for operations requiring Rust-side logic. `apxm-server` HTTP bridge deferred to Phase 4+ where it adds value for graph execution.

### Medium Risk

4. **Dual storage confusion.** The memory system already has key-prefix scoping (`__scope__/<id>/<key>` in a single store). Phase 3 adds per-workspace file paths. Both mechanisms active simultaneously could cause confusion about which is authoritative.

5. **A6 scope overestimate.** Since `child_scope()`, `ScopeRegistry`, and `WorkspaceManager` already exist in-memory, the 25-day estimate may be high. Actual new work is file-tree I/O, TOML serialization, `promote()`, and CLI commands.

### Low Risk

6. **AAM types stable.** `ScopePolicy`, `ScopeSpec`, `GoalTree`, `CompletionPolicy` are well-tested and unlikely to need changes.

7. **Memory system solid.** All three tiers (STM, LTM, Episodic) have clean abstractions, pluggable backends, and good test coverage.

---

## Integration Tests

19 integration tests organized by concern. See [INVESTIGATION.md](INVESTIGATION.md) Section 5 for detailed test specifications.

### AAM File Tree Materialization/Hydration (Tests 1-4)

| # | Test | Location |
|---|------|----------|
| 1 | `materialize_creates_directory_tree` -- AAM state produces correct file tree | `apxm-runtime/src/workspace/` |
| 2 | `hydrate_round_trip` -- materialize then hydrate produces identical AAM state | `apxm-runtime/src/workspace/` |
| 3 | `materialize_handles_empty_state` -- empty AAM creates minimal directory structure | `apxm-runtime/src/workspace/` |
| 4 | `hydrate_nonexistent_directory_fails_gracefully` -- clear error, no panic | `apxm-runtime/src/workspace/` |

### Scoping Rules (Tests 5-8)

| # | Test | Location |
|---|------|----------|
| 5 | `inherit_scope_shares_workspace_files` -- child reads parent beliefs | `apxm-runtime/src/workspace/` |
| 6 | `isolate_scope_empty_workspace` -- child starts with empty data/tools | `apxm-runtime/src/workspace/` |
| 7 | `filter_scope_selective_inheritance` -- only named beliefs inherited | `apxm-runtime/src/workspace/` |
| 8 | `promote_child_state_to_parent` -- only promoted keys appear in parent | `apxm-runtime/src/workspace/` |

### Consumer State Mapping -- Codex (Tests 9-11)

| # | Test | Location |
|---|------|----------|
| 9 | `codex_session_state_to_aam_round_trip` -- all mapped fields preserved | Integration test crate |
| 10 | `codex_turn_state_to_stm` -- STM keys match (excluding non-serializable channels) | Integration test crate |
| 11 | `codex_workspace_lifecycle` -- create, populate, mutate, archive | Integration test crate |

### Consumer State Mapping -- Gemini-CLI (Tests 12-14)

| # | Test | Location |
|---|------|----------|
| 12 | `gemini_chat_state_to_aam` -- history, tools, config mapped to files | Integration test crate |
| 13 | `gemini_loop_detection_reads_episodic` -- uses episodic trace, not counters | Integration test crate |
| 14 | `gemini_compression_maps_to_qmem_umem` -- compaction as AIS operations | Integration test crate |

### Three-Tier Memory (Tests 15-18)

| # | Test | Location |
|---|------|----------|
| 15 | `stm_volatile_across_restart` -- STM data gone after restart | `apxm-runtime/src/memory/` |
| 16 | `ltm_persistent_across_restart` -- LTM data survives restart | `apxm-runtime/src/memory/` |
| 17 | `episodic_append_only_persists` -- previous episodes loaded on restart | `apxm-runtime/src/memory/` |
| 18 | `per_workspace_memory_isolation` -- workspace A data invisible to workspace B | `apxm-runtime/src/memory/` |

### Phase Boundary (Test 19)

| # | Test | Location |
|---|------|----------|
| 19 | `full_phase_3_integration` -- end-to-end: create workspace, map both consumers, survive restart, verify scoping | Top-level integration test |

---

## Validation Criteria

### APXM Substrate (A6.7)

- [ ] `apxm state show <session>` displays B, G, C from the file tree
- [ ] AAM state backed by `~/.apxm/workspaces/<id>/` directories
- [ ] Scoping rules (Inherit, Isolate, Filter) enforced between parent/child scopes
- [ ] Three-tier memory operational (STM in-memory, LTM file-backed, Episodic append-only)
- [ ] State survives process restarts (persistent file backing)

### Codex (C8-C9)

- [ ] `apxm state show <session-id>` works for Codex sessions
- [ ] Codex `SessionState` and `TurnState` fully mapped to AAM (B, G, C)
- [ ] Workspace lifecycle (create, populate, read) operational
- [ ] STM populated from `TurnState` at turn start
- [ ] LTM bridge for persistent memories across sessions
- [ ] Episodic memory populated from conversation history
- [ ] QMEM/UMEM operations exposed for context compaction

### Gemini-CLI (G9-G10)

- [ ] `apxm state show <session>` works for Gemini-CLI sessions
- [ ] AAM (B, G, C) backed by file tree under `~/.apxm/workspaces/`
- [ ] Beliefs survive session restart (via file persistence)
- [ ] Episodic trace records every tool call and LLM response
- [ ] Loop detection reads from episodic trace (not ad-hoc pattern matching)
- [ ] Three-tier memory operational (STM for current turn, LTM for project memory, Episodic for trace)

---

## Effort Summary

| Consumer | Steps | Days | New Files | Modified Files | New Lines (est.) | Notes |
|----------|-------|------|-----------|----------------|-----------------|-------|
| APXM (A6) | Materializer, file-tree WorkspaceManager, `promote()`, memory backing, CLI | ~15-18 | 3 | 2 | ~1400 | Reduced from ~25: child_scope, ScopeRegistry, WorkspaceManager (in-memory) already exist |
| Codex (C8-C9) | State mapping, memory tiers | 5 | 3 | 3 | ~600 | TurnState bridge must exclude non-serializable channels |
| Gemini-CLI (G9-G10) | AAM mapping, three-tier memory, cross-language bridge | 6-8 | -- | -- | -- | Cross-language integration path TBD |

---

## References

- [Source Code Investigation](INVESTIGATION.md) -- cross-reference of plan claims against APXM, Codex, and Gemini-CLI source
- [Session → AAM Mapping Design](../SESSION-AAM-MAPPING.md) -- detailed field-level mapping from consumer sessions to AAM (B, G, C)
- [Sessions & Server Investigation](../SESSIONS-AND-SERVER-INVESTIGATION.md) -- Runtime singleton, session isolation architecture, apxm-server positioning
- [AAM Formal Model](../../pxm/aam.md) -- the abstract agent machine: Beliefs, Goals, Capabilities
- [Hierarchical AAM Implementation](../../implementation/runtime/hierarchical-aam.md) -- file-tree-backed AAM design
- [Plan 1: APXM Changes](../plan-1-apxm-changes.md) -- Phase A6
- [Plan 2: Codex Changes](../plan-2-codex-changes.md) -- Phase 3 (C8-C9)
- [Plan 3: Gemini-CLI Changes](../plan-3-gemini-changes.md) -- Phase 3 (G9-G10)
