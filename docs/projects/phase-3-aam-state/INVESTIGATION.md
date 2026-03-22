# Phase 3 AAM State Model -- Source Code Investigation

**Date:** 2026-03-20
**Investigator:** Claude Opus 4.6
**Scope:** Cross-reference Phase 3 plan documents against APXM, Codex, and Gemini-CLI source code

---

## 1. Verified Claims

### 1.1 AAM Core Types (`apxm-core/src/types/aam.rs`)

All claims about AAM core types in `apxm.md` lines 17-22 are **VERIFIED**.

| Claim | Status | Location |
|-------|--------|----------|
| `ScopePolicy` enum with `Inherit`, `Isolate`, `Snapshot`, `Filter(Vec<String>)` | VERIFIED | `apxm-core/src/types/aam.rs:77-86` |
| `ScopeSpec` with per-dimension `beliefs`, `capabilities`, `goals` fields | VERIFIED | `apxm-core/src/types/aam.rs:89-94` |
| `GoalTree` with parent-child relationships | VERIFIED | `apxm-core/src/types/aam.rs:29-73` |
| `CompletionPolicy` with `AllChildren`, `AnyChild`, `Manual` | VERIFIED | `apxm-core/src/types/aam.rs:17-26` |
| `CapabilityRecord` with name, description, schema, cost_estimate | VERIFIED | `apxm-core/src/types/aam.rs:8-14` |

### 1.2 AAM Runtime Storage (`apxm-runtime/src/aam/`)

Claims about current flat in-memory structures in `apxm.md` lines 14-16 are **VERIFIED**.

| Claim | Status | Actual Implementation |
|-------|--------|----------------------|
| Beliefs: `HashMap<String, Value>` | VERIFIED | `beliefs.rs:11` -- `pub type BeliefMap = HashMap<String, Value>;` |
| Goals: `PriorityQueue<Goal>` with `GoalTree` | VERIFIED | `goals.rs:9-12` -- `GoalQueue = PriorityQueue<GoalId, u32>`, `GoalDetailMap = HashMap<GoalId, Goal>`, plus `GoalTree` in `AamState` |
| Capabilities: `HashMap<String, CapabilityRecord>` | VERIFIED | `capabilities.rs:7` -- `pub type CapabilityMap = HashMap<String, CapabilityRecord>;` |

### 1.3 Scoping Implementation Already Exists

The plan states "These types exist but are not yet wired into runtime execution" (`apxm.md:23`). This is **PARTIALLY INCORRECT** -- scoping IS wired into runtime execution:

- `Aam::child_scope(&self, spec: &ScopeSpec) -> Aam` is fully implemented at `apxm-runtime/src/aam/mod.rs:295-347`
- All four scope policies (Inherit, Isolate, Snapshot, Filter) have working implementations
- Six unit tests verify all scoping behaviors (`mod.rs:762-956`)
- `ExecutionContext` carries `scope_id: String` and `scope_registry: Arc<ScopeRegistry>` (`executor/context.rs:49-51`)

**What IS still missing:** file-tree backing, workspace directory creation/materialization, and the `StateProjector::promote()` direction (child -> parent).

### 1.4 Workspace Infrastructure Already Exists

The plan proposes `WorkspaceManager` as a new deliverable (`apxm.md:126`). A **partial implementation already exists** at `apxm-runtime/src/workspace/mod.rs`:

- `ScopeRegistry` -- HashMap-based registry of active scopes (lines 49-121)
- `WorkspaceManager` -- wrapper around `ScopeRegistry` (lines 132-157)
- `ScopeEntry` -- associates scope_id, parent_id, AAM, and ScopeSpec (lines 20-29)
- 8 unit tests covering register, get, children_of, remove, overwrite (lines 164-287)

**What IS still missing:** The file-system-backed workspace directory management (`create_workspace`, `open_workspace`, `archive_workspace`), the `~/.apxm/workspaces/` directory layout, and the `scope.toml` / `data/` / `goals/` / `tools/` materialization.

### 1.5 Three-Tier Memory System

Claims about the memory system in `apxm.md` lines 98-106 are **VERIFIED** with nuance.

| Claim | Status | Actual |
|-------|--------|--------|
| STM: In-memory DashMap | PARTIALLY CORRECT | `memory/stm.rs:13` uses `InMemoryBackend` (not DashMap directly; the `InMemoryBackend` from `apxm-backends` may use DashMap internally) |
| LTM: `~/.apxm/memory/ltm.sqlite` (global) | VERIFIED | `memory/config.rs:141` -- `memory_dir_path("ltm.sqlite", "apxm_ltm.sqlite")` |
| Episodic: `~/.apxm/memory/episodes.jsonl` (global) | VERIFIED | `memory/config.rs:145` -- `memory_dir_path("episodes.jsonl", "apxm_episodes.jsonl")` |
| Scoped memory operations | VERIFIED | `memory/mod.rs:89-213` -- `scope_prefix()`, `scoped_key()`, `read_scoped()`, `write_scoped()`, `search_scoped()`, `delete_scoped()` all implemented |

**Key finding:** The memory system already has scoped key-prefix namespacing (`__scope__/<scope_id>/<key>`) for STM and LTM. The plan's proposal to move from global to per-workspace files is an **additional** change on top of already-working scoped keys.

### 1.6 Episodic Memory Persistence

Episodic memory supports **file-backed persistence** via JSONL:

- `EpisodicMemory::new(config)` reads existing JSONL file on startup (`episodic.rs:57-71`)
- `persist_entries()` writes all entries to the JSONL file after each `record()` call (`episodic.rs:214-250`)
- Both in-memory-only and file-backed modes are supported via `EpisodicConfig.path`

### 1.7 Session Checkpoint/Restore

`SessionManager` at `apxm-runtime/src/aam/session.rs:22-108` provides:

- Save/load `AamCheckpoint` (beliefs + goals + capabilities + goal_tree) to/from JSON files
- List sessions, delete checkpoints
- 10 unit tests including full AAM round-trip test

This is a **foundation for Phase 3** but uses flat JSON files, not the hierarchical workspace directory structure proposed in the plan.

---

## 2. Codex State Verification

### 2.1 SessionState Structure

Located at `openai/codex/codex-rs/core/src/state/session.rs:20-36`.

| Plan's Mapped Field | Actual Codex Field | Exists? |
|---------------------|-------------------|---------|
| Conversation history | `history: ContextManager` | YES |
| Current model/provider | `session_configuration: SessionConfiguration` | YES (via config, not a direct field) |
| Pending tool calls | Not a direct field on SessionState | PARTIALLY -- managed via `TurnState.pending_approvals` etc. |
| Approval decisions | `granted_permissions: Option<PermissionProfile>` | YES |
| Turn count | Not on SessionState | NO -- TurnState has `tool_calls: u64` but no explicit turn counter |
| Active instructions | Part of `session_configuration` | YES (indirect) |
| Available tools | Not on SessionState | Managed externally via tool registry |
| System prompt | Part of `session_configuration` | YES (indirect) |

### 2.2 TurnState Structure

Located at `openai/codex/codex-rs/core/src/state/turn.rs:77-87`.

| Plan's Mapped Field | Actual Codex Field | Notes |
|---------------------|-------------------|-------|
| Pending approvals | `pending_approvals: HashMap<String, oneshot::Sender<ReviewDecision>>` | Transient -- oneshot channels, not serializable |
| Tool calls count | `tool_calls: u64` | Simple counter |
| Token usage at turn start | `token_usage_at_turn_start: TokenUsage` | Present |
| Pending input | `pending_input: Vec<ResponseInputItem>` | Present |
| Pending dynamic tools | `pending_dynamic_tools: HashMap<String, oneshot::Sender<DynamicToolResponse>>` | Transient channels |

### 2.3 Discrepancies in Codex Mapping

1. **Pending approvals are transient channels:** `TurnState` stores `oneshot::Sender<ReviewDecision>` values for pending approvals -- these are Tokio channel handles, not serializable state. The plan's mapping of "pending tool calls -> `data/pending_tools.json`" assumes these can be materialized to disk, but they represent live async coordination points.

2. **No explicit turn counter on SessionState:** The plan maps "turn count" to `data/session_state.json`, but there is no `turn_count` field on `SessionState`. The `TurnState` has `tool_calls` (a call counter per turn), and turn counting appears to happen at a different level.

3. **Context compaction:** Codex has a `ContextManager` (`codex-rs/core/src/context_manager/mod.rs`) that handles history management and token accounting. The plan references "2-phase context compaction" with specific model names (`gpt-5.1-codex-mini`, `gpt-5.3-codex`), but the actual compaction implementation should be verified against `compact_remote.rs` for accuracy of model references.

---

## 3. Gemini-CLI State Verification

### 3.1 GeminiChat Class

Located at `google/gemini-cli/packages/core/src/core/geminiChat.ts:249`.

| Plan's Mapped Field | Actual Field | Exists? |
|---------------------|-------------|---------|
| Conversation history | `private history: Content[]` (constructor param) | YES |
| System instruction | `private systemInstruction: string` | YES |
| Tools | `private tools: Tool[]` | YES |
| Model / provider | Via `context: AgentLoopContext` | YES (indirect) |

**Key discrepancy:** `GeminiChat` is a relatively thin wrapper around the Gemini API chat interface. Most of the state the plan references lives on `LocalAgentExecutor`, not `GeminiChat` directly:

- Turn counter: lives in `LocalAgentExecutor.executeTurn()` as a local `turnCounter` variable (`local-executor.ts:319`)
- Compression service: `LocalAgentExecutor` owns the `ChatCompressionService` (`local-executor.ts:117`)
- Tool registry: `LocalAgentExecutor` owns `ToolRegistry`, `PromptRegistry`, `ResourceRegistry` (`local-executor.ts:112-114`)

### 3.2 ChatCompressionService

Located at `google/gemini-cli/packages/core/src/services/chatCompressionService.ts:233`.

- It is a **class** (not a TypeScript type/interface), confirmed at line 233
- The `compress()` method takes a `GeminiChat`, config, and returns `{ newHistory, info }`
- Uses a token-threshold-based approach (50% of model token limit by default)
- The plan's claim that "compression state" maps to a Belief is accurate -- the compression status and token counts could be serialized

### 3.3 Loop Detection

Located at `google/gemini-cli/packages/core/src/services/loopDetectionService.ts`.

The plan states: "Loop detection reads from episodic trace (not ad-hoc pattern matching)" as a validation criterion (`gemini-cli.md:129`). This is a **future state** goal, not current behavior:

- **Current implementation:** `LoopDetectionService` (line 133) uses **ad-hoc pattern matching**:
  - Tool call repetition tracking via SHA-256 hashing (`checkToolCallLoop`, line 314)
  - Content chanting detection via sliding window chunk comparison (`checkContentLoop`, line 339)
  - LLM-based loop detection with confidence scoring (`checkForLoopWithLLM`, line 540)
- **No episodic memory integration:** The service reads directly from `context.geminiClient.getHistory()` (line 545-546), not from any structured episodic trace
- Event types: `LoopType.CONSECUTIVE_IDENTICAL_TOOL_CALLS`, `LoopType.CONTENT_CHANTING_LOOP`, `LoopType.LLM_DETECTED_LOOP`

This confirms the plan correctly identifies this as a gap to be filled by Phase 3.

---

## 4. Discrepancies Found

### 4.1 Scoping Is Already Wired (Moderate Impact)

`apxm.md:23` states: "These types exist but are not yet wired into runtime execution."

**Reality:** `Aam::child_scope()` is fully implemented and tested. `ExecutionContext` carries `scope_id` and `scope_registry`. The `ScopeRegistry` and `WorkspaceManager` exist. What's missing is the file-tree backing, not the in-memory scoping infrastructure.

**Impact:** Phase 3 effort for A6.3 (StateProjector) may be overestimated. The in-memory `child_scope()` already handles Inherit/Isolate/Snapshot/Filter. The work is primarily adding file-tree materialization and the promote direction.

### 4.2 STM Is InMemoryBackend, Not DashMap (Low Impact)

`apxm.md:104` states STM uses "In-memory DashMap."

**Reality:** STM uses `InMemoryBackend` from `apxm-backends` crate. DashMap is used extensively in the scheduler (`scheduler/state.rs`, `scheduler/ready_set.rs`, `scheduler/lane_queue.rs`) and capability registry, but STM itself wraps a backend abstraction. The distinction matters only for documentation accuracy.

### 4.3 Codex Turn Counter Missing (Moderate Impact)

The plan maps "turn count" as a Belief (`codex.md:25`), but `SessionState` has no `turn_count` field. Turn counting appears to happen at the turn-level (`ActiveTurn` / task management), not as persistent session state. The mapping needs adjustment or a turn counter needs to be explicitly added.

### 4.4 Codex TurnState Contains Non-Serializable State (High Impact)

`TurnState` contains `oneshot::Sender` channels for approvals, permissions, elicitations, and dynamic tools. These cannot be serialized to AAM files. The plan's mapping of "pending tool calls" to `data/pending_tools.json` would need to capture the request metadata without the channel handles.

### 4.5 Gemini-CLI State Is Spread Across Multiple Classes (Moderate Impact)

The plan's `gemini-cli.md` titles its mapping "Map GeminiChat state to AAM" but much of the relevant state lives on `LocalAgentExecutor`, not `GeminiChat`:
- `GeminiChat`: history, systemInstruction, tools, chatRecordingService
- `LocalAgentExecutor`: turnCounter (local var), compressionService, toolRegistry, promptRegistry, loopDetection (via AgentLoopContext)
- `LoopDetectionService`: all loop state (thresholds, counters, LLM check intervals)
- `ChatCompressionService`: stateless (operates on GeminiChat)

### 4.6 CLI `state` Commands Do Not Exist Yet (Expected)

No `apxm state` commands exist in the CLI. The `apxm-cli/src/` directory has no `commands/` subdirectory, and no grep matches for "state show" or "state list". This is correctly identified as a deliverable in A6.5.

### 4.7 Memory Scoping Already Implemented (Low-Moderate Impact)

The plan proposes "Per-workspace `data/*.toml` files" for LTM (`apxm.md:105`) and "Per-workspace `episodes.jsonl`" for Episodic (`apxm.md:106`). However, **scoped memory already exists** via key-prefix namespacing in the `MemorySystem`:

- `MemorySystem::scoped_key(scope_id, key)` -> `__scope__/<scope_id>/<key>`
- `read_scoped()`, `write_scoped()`, `search_scoped()`, `delete_scoped()`
- Unit tests verify scope isolation (`memory/mod.rs:344-412`)

The proposed change would move from logical namespacing (same physical store) to physical file separation (per-workspace directories). These are complementary, not contradictory.

---

## 5. Integration Test Recommendations

### 5.1 AAM File Tree Materialization/Hydration Tests

**Location:** `apxm-runtime/src/workspace/` (new test module)

```
Test 1: materialize_creates_directory_tree
  - Create AamState with beliefs, goals, capabilities
  - Call Materializer::materialize() to a temp directory
  - Assert: scope.toml, data/*.toml, goals/current.toml, tools/*.toml all exist
  - Assert: file contents match in-memory state

Test 2: hydrate_round_trip
  - Materialize an AamState
  - Hydrate from the same directory into a new AamState
  - Assert: beliefs, goals, capabilities match original

Test 3: materialize_handles_empty_state
  - Materialize an empty AamState
  - Assert: directory structure exists but data files are empty/minimal

Test 4: hydrate_nonexistent_directory_fails_gracefully
  - Attempt to hydrate from a path that doesn't exist
  - Assert: returns clear error, does not panic
```

### 5.2 Scoping Rule Tests

**Location:** `apxm-runtime/src/workspace/` (extend existing tests)

Most scoping tests already exist in `aam/mod.rs:762-956`. Additional file-tree-level tests needed:

```
Test 5: inherit_scope_shares_workspace_files
  - Create parent workspace, write beliefs
  - Create child workspace with Inherit policy
  - Assert: child workspace reads parent's belief files
  - Write new belief in child
  - Assert: parent can see child's write (shared backing)

Test 6: isolate_scope_empty_workspace
  - Create parent with beliefs + capabilities
  - Create child workspace with Isolate policy
  - Assert: child workspace data/ and tools/ dirs are empty
  - Assert: parent state unchanged

Test 7: filter_scope_selective_inheritance
  - Create parent with beliefs {A, B, C}
  - Create child with Filter(["A"]) policy
  - Assert: child workspace data/ contains only A
  - Assert: modifications to A in child don't affect parent

Test 8: promote_child_state_to_parent
  - Create child scope, add new beliefs
  - Call StateProjector::promote() with specific keys
  - Assert: only promoted keys appear in parent
  - Assert: non-promoted keys remain local to child
```

### 5.3 Consumer State Mapping Tests

**Location:** New integration test crates or test modules

#### Codex Integration Tests

```
Test 9: codex_session_state_to_aam_round_trip
  - Create a mock Codex SessionState with history, config, permissions
  - Map to AAM (B, G, C)
  - Write to workspace directory
  - Read back from workspace directory
  - Map back to SessionState-compatible structure
  - Assert: all mapped fields preserved

Test 10: codex_turn_state_to_stm
  - Create TurnState with tool_calls count and token_usage
  - Map to STM entries
  - Assert: STM contains expected keys
  - Assert: values match (excluding non-serializable channels)

Test 11: codex_workspace_lifecycle
  - Create workspace for Codex session
  - Simulate session start (populate initial AAM)
  - Simulate state mutation (add belief, complete goal)
  - Assert: workspace files reflect mutations
  - Simulate session end (archive workspace)
  - Assert: workspace archived correctly
```

#### Gemini-CLI Integration Tests

```
Test 12: gemini_chat_state_to_aam
  - Create GeminiChat-equivalent state (history, model config, tools)
  - Map to AAM (B, G, C)
  - Assert: conversation history -> data/conversation.json
  - Assert: tools -> tools/*.toml
  - Assert: model config -> data/model_config.toml or similar

Test 13: gemini_loop_detection_reads_episodic
  - Record tool calls and LLM responses to episodic memory
  - Invoke loop detection (new REFLECT-based implementation)
  - Assert: loop detection uses episodic trace, not ad-hoc counters

Test 14: gemini_compression_maps_to_qmem_umem
  - Simulate context compaction
  - Assert: compaction is represented as QMEM + UMEM operations
  - Assert: episodic trace records the compaction event
```

### 5.4 Three-Tier Memory Tests

**Location:** `apxm-runtime/src/memory/` (extend existing tests)

```
Test 15: stm_volatile_across_restart
  - Write to STM
  - "Restart" (create new MemorySystem)
  - Assert: STM data is gone

Test 16: ltm_persistent_across_restart
  - Write to LTM with SQLite backend
  - Create new MemorySystem pointing to same path
  - Assert: LTM data survives

Test 17: episodic_append_only_persists
  - Record episodes with file-backed config
  - Create new EpisodicMemory from same file
  - Assert: previous episodes are loaded

Test 18: per_workspace_memory_isolation
  - Create two workspaces
  - Write to LTM in workspace A
  - Assert: LTM in workspace B does not see A's data
  - Assert: each workspace's episodic trace is independent
```

### 5.5 Phase Boundary Integration Test

**Location:** Top-level integration test

```
Test 19: full_phase_3_integration
  - Create APXM workspace (A6)
  - Map Codex-like state to AAM (C8)
  - Map Gemini-CLI-like state to AAM (G9)
  - Assert: apxm state show <session> produces valid output for both
  - Assert: state survives simulated process restart
  - Assert: three-tier memory operational for both consumers
  - Assert: scoping rules enforced between parent and child scopes
```

---

## 6. Gap Analysis: What Exists vs What Phase 3 Needs

### 6.1 APXM Substrate (A6)

| Component | Current State | Phase 3 Target | Gap |
|-----------|--------------|----------------|-----|
| `ScopePolicy`, `ScopeSpec`, `GoalTree` types | COMPLETE | Same | None |
| `Aam::child_scope()` (in-memory scoping) | COMPLETE with tests | Same | None |
| `ScopeRegistry` | COMPLETE with tests | Same | None |
| `WorkspaceManager` | Stub (wraps ScopeRegistry) | Full directory management | Need `create_workspace`, `open_workspace`, `archive_workspace` with `~/.apxm/workspaces/` layout |
| `Materializer` | NOT STARTED | Write/read AAM to/from file tree | Full implementation needed |
| `StateProjector` | `child_scope()` handles creation direction | Also needs `promote()` (child -> parent) | promote() not implemented |
| Three-tier memory | COMPLETE (STM, LTM, Episodic all working) | Per-workspace scoping | Already has key-prefix scoping; needs per-workspace file paths |
| `SessionManager` | COMPLETE (checkpoint save/load) | Integrate with workspace dirs | Bridge between checkpoint JSON and workspace file tree |
| CLI `state` commands | NOT STARTED | `show`, `list`, `diff` | Full implementation needed |
| Effect analysis | COMPLETE (`effects.rs` covers all 39 ops) | Same | None |

### 6.2 Codex Consumer (C8-C9)

| Component | Current State | Phase 3 Target | Gap |
|-----------|--------------|----------------|-----|
| `SessionState` | Exists (`state/session.rs`) | Map to AAM | Need bridge adapter |
| `TurnState` | Exists (`state/turn.rs`) | Map to STM | Need adapter (excluding non-serializable channels) |
| `ContextManager` | Exists | Map history to Beliefs | Need serialization bridge |
| Turn counter | Not an explicit field | Add to `session_state.json` | Need to add or derive |
| Context compaction | Exists via `compact_remote.rs` | Map to QMEM/UMEM | Need AIS operation mapping |
| `state_bridge.rs` | NOT STARTED | Bidirectional mapping | Full implementation needed |

### 6.3 Gemini-CLI Consumer (G9-G10)

| Component | Current State | Phase 3 Target | Gap |
|-----------|--------------|----------------|-----|
| `GeminiChat` | Exists (TypeScript) | Map to AAM | Need TypeScript/APXM bridge |
| `LocalAgentExecutor` | Exists (holds most state) | Map turn state | Need to centralize state |
| `ChatCompressionService` | Exists | Map to QMEM/UMEM | Need operation mapping |
| `LoopDetectionService` | Ad-hoc pattern matching | REFLECT on episodic trace | Significant refactor needed |
| Episodic trace | Not tracked | Append-only log | Full implementation needed |
| File-tree backing | NOT STARTED | `~/.apxm/workspaces/gemini-session-*` | Full implementation needed |

---

## 7. Risk Assessment

### 7.1 High Risk

1. **Non-serializable TurnState in Codex:** `oneshot::Sender` channels cannot be materialized to files. The C8 state bridge must carefully separate serializable metadata (what was requested, when, status) from live coordination handles. Failing to do this cleanly would break either persistence or live operation.

2. **Gemini-CLI loop detection refactor:** Moving from ad-hoc pattern matching to episodic-trace-based REFLECT is a significant behavioral change in production code. The current `LoopDetectionService` has 760 lines of carefully tuned heuristics. Replacing it with a different approach risks regression.

3. **Cross-language bridge (Gemini-CLI is TypeScript):** The plan assumes Gemini-CLI can read/write `~/.apxm/workspaces/` and interact with APXM's file tree. Since Gemini-CLI is TypeScript/Node.js and APXM is Rust, the integration path is either: (a) subprocess calls to `apxm` CLI, (b) a shared file format, or (c) an MCP bridge. The plan does not specify which.

### 7.2 Medium Risk

4. **Dual storage confusion:** The memory system already has key-prefix scoping (`__scope__/<id>/<key>` in a single SQLite/in-memory store). Phase 3 adds per-workspace file paths. Having both mechanisms active simultaneously could cause confusion about which is authoritative.

5. **Plan overestimates A6 scope:** Since `child_scope()`, `ScopeRegistry`, and `WorkspaceManager` already exist, the 25-day estimate for APXM may be high. But this depends on how much file-tree I/O, TOML serialization, and CLI work is actually needed.

### 7.3 Low Risk

6. **AAM types are stable:** The `ScopePolicy`, `ScopeSpec`, `GoalTree`, `CompletionPolicy` types are well-tested and unlikely to need changes.

7. **Memory system is solid:** All three tiers (STM, LTM, Episodic) have clean abstractions, pluggable backends, and good test coverage.

---

## 8. File Reference Index

### APXM Source Files

| File | Lines | Role |
|------|-------|------|
| `apxm/crates/apxm-core/src/types/aam.rs` | 116 | Canonical AAM type definitions |
| `apxm/crates/apxm-runtime/src/aam/mod.rs` | 957 | AAM runtime (Aam handle, AamState, child_scope, transitions) |
| `apxm/crates/apxm-runtime/src/aam/beliefs.rs` | 41 | BeliefMap, diff, snapshot_subset |
| `apxm/crates/apxm-runtime/src/aam/goals.rs` | 37 | GoalQueue, GoalDetailMap, filtered_state |
| `apxm/crates/apxm-runtime/src/aam/capabilities.rs` | 23 | CapabilityMap, snapshot_subset |
| `apxm/crates/apxm-runtime/src/aam/scope.rs` | 3 | Re-exports ScopePolicy, ScopeSpec |
| `apxm/crates/apxm-runtime/src/aam/session.rs` | 296 | SessionManager checkpoint persistence |
| `apxm/crates/apxm-runtime/src/aam/effects.rs` | 165 | Operation effect declarations (read/write sets) |
| `apxm/crates/apxm-runtime/src/workspace/mod.rs` | 288 | ScopeRegistry, WorkspaceManager, ScopeEntry |
| `apxm/crates/apxm-runtime/src/memory/mod.rs` | 413 | MemorySystem (STM+LTM+Episodic), scoped operations |
| `apxm/crates/apxm-runtime/src/memory/stm.rs` | 179 | ShortTermMemory (InMemoryBackend) |
| `apxm/crates/apxm-runtime/src/memory/ltm.rs` | 179 | LongTermMemory (SQLite/Redb/Memory backends) |
| `apxm/crates/apxm-runtime/src/memory/episodic.rs` | 404 | EpisodicMemory (ring buffer + JSONL persistence) |
| `apxm/crates/apxm-runtime/src/memory/config.rs` | 147 | Memory configuration (StmConfig, LtmConfig, EpisodicConfig) |
| `apxm/crates/apxm-runtime/src/executor/context.rs` | 78+ | ExecutionContext (carries scope_id, scope_registry, memory, aam) |
| `apxm/docs/implementation/runtime/hierarchical-aam.md` | 866 | Hierarchical AAM design doc with 14 diagrams |

### Codex Source Files

| File | Lines | Role |
|------|-------|------|
| `openai/codex/codex-rs/core/src/state/session.rs` | 241 | SessionState (history, config, rate limits, permissions) |
| `openai/codex/codex-rs/core/src/state/turn.rs` | 222 | TurnState (pending approvals, channels, tool_calls counter) |
| `openai/codex/codex-rs/core/src/context_manager/mod.rs` | -- | ContextManager (history management) |

### Gemini-CLI Source Files

| File | Lines | Role |
|------|-------|------|
| `google/gemini-cli/packages/core/src/core/geminiChat.ts` | 249+ | GeminiChat class (history, systemInstruction, tools) |
| `google/gemini-cli/packages/core/src/services/chatCompressionService.ts` | 233+ | ChatCompressionService (token-threshold compression) |
| `google/gemini-cli/packages/core/src/services/loopDetectionService.ts` | 760 | LoopDetectionService (tool call, content, LLM-based detection) |
| `google/gemini-cli/packages/core/src/agents/local-executor.ts` | 108+ | LocalAgentExecutor (executeTurn loop, owns compression+tools) |

### Plan Documents

| File | Role |
|------|------|
| `apxm/docs/projects/phase-3-aam-state/README.md` | Phase 3 overview |
| `apxm/docs/projects/phase-3-aam-state/apxm.md` | APXM substrate changes (A6) |
| `apxm/docs/projects/phase-3-aam-state/codex.md` | Codex consumer changes (C8-C9) |
| `apxm/docs/projects/phase-3-aam-state/gemini-cli.md` | Gemini-CLI consumer changes (G9-G10) |
