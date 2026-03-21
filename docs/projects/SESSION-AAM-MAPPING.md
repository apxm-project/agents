# Session → AAM Mapping Design

**Date:** 2026-03-20
**Status:** Technical design (derived from investigation agents)
**Scope:** How consumer session/chat/context state maps to the APXM AAM model `(B, G, C)`

---

## 1. Core Insight: Session IS an AAM Scope

Each consumer session (Codex `SessionState`, Gemini-CLI `GeminiChat`) maps to a **scoped AAM instance** — an isolated `(B, G, C)` triple with its own beliefs, goals, and capabilities. The APXM runtime already has the infrastructure for this (`child_scope`, `ScopeSpec`, `ScopeRegistry`), but the server does not wire it.

```
Consumer Session              APXM AAM Scope
┌──────────────────┐          ┌──────────────────────┐
│ Codex Session    │  ──────► │ AAM(B, G, C)         │
│  - history       │          │  B: conversation,    │
│  - pending tools │          │     pending_tools,   │
│  - approvals     │          │     approvals, config│
│  - instructions  │          │  G: current_task     │
│  - tool registry │          │  C: shell, read, mcp │
└──────────────────┘          └──────────────────────┘

┌──────────────────┐          ┌──────────────────────┐
│ Gemini Chat      │  ──────► │ AAM(B, G, C)         │
│  - history       │          │  B: conversation,    │
│  - model config  │          │     compression,     │
│  - tools         │          │     turn_state       │
│  - compression   │          │  G: user_query       │
│  - turn counter  │          │  C: tools, mcp, llm  │
└──────────────────┘          └──────────────────────┘
```

---

## 2. Codex SessionState → AAM Field Mapping

Source: `codex-rs/core/src/state/session.rs` (SessionState), `state/turn.rs` (TurnState)

### 2.1 Beliefs (B)

| Codex Field | Type | AAM Key | Notes |
|-------------|------|---------|-------|
| Conversation history | `Vec<Message>` | `B["conversation"]` | Via ContextManager; serialized to `data/conversation.json` |
| Model/provider config | `SessionConfiguration` | `B["model_config"]` | Model name, provider, temperature |
| Approval decisions | `HashMap<String, Permission>` | `B["approvals"]` | Cached guardian decisions |
| System prompt | `String` | `B["system_prompt"]` | Via SessionConfiguration |
| Turn tool call count | `u64` (TurnState) | `B["turn_tool_calls"]` | Per-turn counter |
| Pending tool metadata | Serializable subset | `B["pending_tools"]` | Request info + status only |

### 2.2 Goals (G)

| Codex Field | AAM Goal | Notes |
|-------------|----------|-------|
| User instruction | `Goal("user_instruction", priority=1)` | The user's current task |
| Active configuration instructions | `Goal("config_instructions", priority=2)` | From `.codex/instructions.md` |

### 2.3 Capabilities (C)

| Codex Field | AAM Capability | Notes |
|-------------|---------------|-------|
| 21 standard ToolHandlers | `C["shell"]`, `C["read_file"]`, ... | Registered via Phase 2 adapters |
| MCP-discovered tools | `C["mcp/<tool>"]` | Via McpHandler |
| LLM backend | `C["llm"]` | Model + provider capability |

### 2.4 Non-Serializable Exclusions

These `TurnState` fields are Tokio channel handles and CANNOT be mapped to AAM:

| Field | Type | Why Excluded |
|-------|------|-------------|
| `pending_review` | `oneshot::Sender<ReviewDecision>` | Live channel handle |
| `pending_dynamic_tool` | `oneshot::Sender<DynamicToolResponse>` | Live channel handle |
| `pending_approval` | `oneshot::Sender<ApprovalDecision>` | Live channel handle |
| `pending_mcp_approval` | `oneshot::Sender<McpApprovalDecision>` | Live channel handle |
| `pending_notification` | `oneshot::Sender<NotificationResponse>` | Live channel handle |

These represent in-flight coordination that only exists during active execution. The bridge stores the *metadata* (what tool is pending, when it was requested, current status) in beliefs, not the channel itself.

---

## 3. Gemini-CLI GeminiChat → AAM Field Mapping

Source: `gemini-cli/packages/core/src/core/geminiChat.ts`, `local-executor.ts`

### 3.1 Beliefs (B)

| Gemini-CLI Field | Source Class | AAM Key | Notes |
|-----------------|-------------|---------|-------|
| Conversation history | `GeminiChat.history` | `B["conversation"]` | `Content[]` array |
| Model/provider | Config resolution | `B["model_config"]` | Model name + provider |
| Compression state | `ChatCompressionService` | `B["compression"]` | Status, thresholds |
| Turn counter | `LocalAgentExecutor` (local var) | `B["turn_count"]` | Currently a local variable, needs promotion |
| Pending confirmations | Scheduler state | `B["pending_confirms"]` | Tool calls awaiting user approval |
| User hints | Injection service | `B["user_hints"]` | In-session user injections |
| Background completions | Fast-ack helper | `B["bg_completions"]` | Pending background results |
| System instruction | `GeminiChat.systemInstruction` | `B["system_instruction"]` | System prompt text |

### 3.2 Goals (G)

| Gemini-CLI Concept | AAM Goal | Notes |
|-------------------|----------|-------|
| User query | `Goal("answer_user_query", priority=1)` | Primary task |
| Sub-agent task | `Goal(sub_task, parent=root_goal)` | Via agent delegation |
| `complete_task` tool call | Goal completion signal | Marks goal as achieved |

### 3.3 Capabilities (C)

| Gemini-CLI Concept | AAM Capability | Notes |
|-------------------|---------------|-------|
| Built-in tools | `C["shell_command"]`, `C["read_file"]`, ... | `DeclarativeTool` instances |
| MCP-discovered tools | `C["mcp/<tool>"]` | Via `McpClientManager` |
| Agent delegation | `C["agent/<name>"]` | Sub-agent as capability |
| LLM provider | `C["llm"]` | Model + provider capability |

---

## 4. Session Lifecycle → AAM Operations

### 4.1 Session Creation

```
Consumer: session = new Session(config)
APXM:    aam = runtime.get_or_create_session_aam(session_id)
         aam.set_belief("model_config", config.model)
         aam.set_belief("system_prompt", config.prompt)
         for tool in config.tools:
             aam.register_capability(tool.name, tool.schema)
         aam.add_goal(Goal::new("user_task", priority=1))
```

### 4.2 Turn N Execution

```
Consumer: response = session.execute_turn(user_message)
APXM:    aam.update_belief("conversation", append(user_message))
         aam.update_belief("turn_count", n)
         # STM: populated with turn-local context
         stm.write("current_input", user_message)
         # Execute graph (Phase 4) or imperative loop (Phases 1-3)
         result = runtime.execute(turn_graph, session_aam)
         # Post-turn
         aam.update_belief("conversation", append(result))
         episodic.append(TurnRecord { input, output, tools_called, timestamp })
```

### 4.3 Session End

```
Consumer: session.close()
APXM:    workspace.materialize(aam)    # Write to ~/.apxm/workspaces/<id>/
         stm.clear(session_scope)      # Volatile STM cleared
         # LTM and Episodic persist in workspace directory
```

---

## 5. Conversation History Storage: Four Tiers

| Tier | What | Lifespan | Access Pattern |
|------|------|----------|---------------|
| `B["conversation"]` | Full message array (current session) | Session | Read/append per turn |
| Episodic | Per-turn records (input, output, tools, timing) | Permanent | Append-only, REFLECT queries |
| STM | Current turn working context | Turn | Read/write, volatile |
| LTM | Cross-session facts, user preferences | Permanent | Keyword search, fact retrieval |

The conversation array in Beliefs is the "live" history for the current session. Episodic memory is the structured trace for analysis (loop detection, debugging, audit). STM is scratch space. LTM is long-term knowledge.

---

## 6. Session Isolation Architecture

### 6.1 The Problem (Current)

`Runtime::build_context()` clones `Aam` as `Arc<RwLock<AamState>>` — a shared reference. All concurrent sessions write to the **same** underlying beliefs, goals, and capabilities. This is by design for multi-agent collaboration within ONE system, but breaks for independent consumers.

### 6.2 Solution: Per-Session AAM via child_scope

```rust
// Proposed change to Runtime
impl Runtime {
    /// Get or create an isolated AAM scope for a session.
    /// Each session gets its own (B, G, C) triple via child_scope(Isolate).
    pub fn get_or_create_session_aam(&self, session_id: &str) -> Aam {
        // Check if session already has a scope
        if let Some(scope) = self.scope_registry.get(session_id) {
            return scope.aam.clone();
        }
        // Create isolated child scope
        let session_aam = self.aam.child_scope(&ScopeSpec::isolate());
        self.scope_registry.register(
            session_id.to_string(),
            None, // no parent session
            session_aam.clone(),
            ScopeSpec::isolate(),
        );
        session_aam
    }
}
```

### 6.3 Phase-by-Phase Isolation Strategy

| Phase | Isolation Model | Mechanism |
|-------|----------------|-----------|
| 1-3 | One Runtime per consumer | Library embedding (Codex) or NAPI module (Gemini-CLI). Automatic isolation — separate process, separate AAM. |
| 4+ | Per-session AAM in shared server | `apxm-server` creates isolated child AAM per session_id via `get_or_create_session_aam()` |
| Multi-agent | Per-agent AAM with inheritance | `child_scope(ScopeSpec::inherit())` for collaborating agents within a session |

### 6.4 apxm-server Session Isolation Work (Pre-Phase 4)

Before `apxm-server` can serve multiple consumers:

1. **Per-session AAM** — `build_context()` should use `get_or_create_session_aam(session_id)` instead of cloning the root AAM
2. **Per-session memory namespacing** — Use `read_scoped()`/`write_scoped()` with session_id as scope prefix
3. **Per-consumer capability isolation** — Some capabilities global (LLM backends), some per-consumer (shell tools)
4. **Session lifecycle API** — Create, checkpoint, restore, destroy sessions via HTTP endpoints

The building blocks exist (`ScopeSpec`, `ScopeRegistry`, `SessionManager`, scoped memory ops). The wiring is missing.

---

## 7. Workspace File Tree for Sessions

Each session materializes to a workspace directory:

```
~/.apxm/workspaces/
├── codex-session-abc123/
│   ├── scope.toml                    # id, parent, policy=Isolate, revision
│   ├── data/                         # B (Beliefs)
│   │   ├── conversation.json         #   Chat history
│   │   ├── pending_tools.json        #   In-flight tool call metadata
│   │   ├── approvals.json            #   Guardian decisions
│   │   ├── session_state.json        #   Turn count, model config
│   │   └── system_prompt.md          #   System prompt
│   ├── goals/                        # G (Goals)
│   │   └── current.toml              #   Active goal(s)
│   └── tools/                        # C (Capabilities)
│       ├── shell.toml
│       ├── read_file.toml
│       └── mcp/
│           └── filesystem.toml
├── gemini-session-def456/
│   ├── scope.toml
│   ├── data/
│   │   ├── conversation.json
│   │   ├── compression.json
│   │   ├── turn_state.json
│   │   └── system_instruction.md
│   ├── goals/
│   │   └── current.toml
│   └── tools/
│       ├── shell_command.toml
│       ├── read_file.toml
│       └── mcp/
│           └── browser.toml
└── .archive/                         # Completed sessions
    └── codex-session-xyz789/
```

---

## 8. SessionManager → Workspace Evolution

The existing `SessionManager` (`aam/session.rs`) saves/loads `AamCheckpoint` as JSON files per session_id. This evolves into workspace-backed persistence:

| Current (SessionManager) | Future (WorkspaceManager) |
|--------------------------|--------------------------|
| `save_checkpoint(id, checkpoint)` → single JSON file | `materialize(workspace, aam)` → directory tree |
| `load_checkpoint(id)` → parse JSON | `hydrate(workspace)` → reconstruct from files |
| `list_sessions()` → scan dir for JSON files | `list_workspaces()` → scan `~/.apxm/workspaces/` |
| `delete_checkpoint(id)` → remove JSON file | `archive_workspace(id)` → move to `.archive/` |

The evolution path is additive: `SessionManager` continues to work during Phases 1-2, while `WorkspaceManager` is built during Phase 3 (A6).

---

## 9. Cross-Language Bridge for Gemini-CLI

Gemini-CLI (TypeScript) needs to read/write the AAM workspace. Three options, recommended in order:

| Phase | Bridge | How |
|-------|--------|-----|
| 1-3 | **NAPI module** | `napi-rs` binds APXM Rust directly into Node.js. In-process, zero serialization overhead. Precedent: AgentMate uses PyO3 for Python. |
| 1-3 (fallback) | **Subprocess** | Shell out to `apxm state show <id> --json`, `apxm state write <id> --json`. Simple, reliable, process-per-call overhead. |
| 4+ | **HTTP via apxm-server** | `POST /v1/state/<session>` for reads/writes. Adds latency but enables multi-consumer coordination. |

For Phase 3 specifically (AAM state mapping), the **shared file format** approach works best: both TypeScript and Rust read/write the same `~/.apxm/workspaces/` files (TOML/JSON). The workspace schema is documented, stable, and human-readable.

---

## Related Documents

- [Sessions & Server Investigation](SESSIONS-AND-SERVER-INVESTIGATION.md) — Runtime singleton, shared AAM, server architecture
- [Phase 3: AAM State Model](phase-3-aam-state/README.md) — Phase plan for hierarchical AAM
- [Phase 3: Codex](phase-3-aam-state/codex.md) — Codex SessionState → AAM mapping
- [Phase 3: Gemini-CLI](phase-3-aam-state/gemini-cli.md) — GeminiChat → AAM mapping
- [Phase 4: Agent Loop as Graph](phase-4-agent-loop/README.md) — Where apxm-server enters
- [AAM Formal Model](../pxm/aam.md) — The abstract agent machine: (B, G, C)
