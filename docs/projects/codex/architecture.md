# Codex-on-APXM Architecture

How a Codex-class coding agent maps onto A-PXM as the substrate, with AgentMate or other authoring layers as optional frontends.

## Layer Mapping

```
┌─────────────────────────────────────────────────┐
│ User Interface                                  │
│ CLI / TUI / IDE extension          [planned]    │
├─────────────────────────────────────────────────┤
│ Authoring Frontend                  [optional]  │
│ AgentMate, custom DSLs, future Codex shims      │
│ Graph authoring, helpers, UX glue               │
├─────────────────────────────────────────────────┤
│ A-PXM Compiler                                  │
│ Graph → MLIR → Optimization passes → Artifact   │
├─────────────────────────────────────────────────┤
│ A-PXM Runtime                      [exists]     │
│ Dataflow scheduler, AAM state, Memory hierarchy │
├──────────────────────┬──────────────────────────┤
│ LLM Backends         │ Capabilities (Tools)     │
│ OpenAI, Anthropic,   │ CapabilitySystem with    │
│ Ollama, OpenRouter   │ interceptor pipeline     │
├──────────────────────┴──────────────────────────┤
│ Sandbox (OS-level isolation)       [P0 gap]     │
│ Seatbelt (macOS), Landlock (Linux)              │
└─────────────────────────────────────────────────┘
```

## Codex Concepts → A-PXM Components

| Codex Concept | A-PXM Equivalent | Component | Status |
|---------------|---------------------------|-----------|--------|
| `run_turn` agent loop | `ASK` node with tool iteration (llm.rs tool loop) | apxm-runtime handler | Implemented |
| Conversation history | Episodic memory (append-only log) | apxm-runtime memory | Implemented |
| Session memories | STM (per-execution `RwLock<HashMap>` via InMemoryBackend) | apxm-runtime memory | Implemented |
| Persistent memories | LTM (SQLite or Redb store) | apxm-runtime memory | Implemented |
| System prompt / instructions | AAM Beliefs (`HashMap<String, Value>`) | apxm-runtime AAM | Implemented |
| `ApprovalStore` | Capability interceptor pipeline (pre/post hooks, Deny/Allow/EditArgs) | apxm-runtime capability | Implemented (no async user approval yet -- P1 gap) |
| Platform sandbox | OS-level isolation | Not implemented | P0 gap -- no sandbox crate exists |
| Tool dispatch (`FuturesOrdered`) | Parallel `INV` nodes via dataflow scheduler | apxm-runtime scheduler | Implemented (but tool calls within ASK tool loop are sequential -- P0 gap) |
| `SessionState` (SQLite) | AAM checkpoint/restore (`AamCheckpoint`: beliefs + goals) | apxm-runtime AAM | Partial (checkpoint excludes capabilities -- P1 gap) |
| Streaming token output | Event emitter (`LlmToken`, `ToolStart`, `ToolEnd`) | apxm-runtime events | Partial (emits after full response, no true token streaming -- P0 gap) |
| Multi-turn sessions | Multi-node AIS graph | apxm-compiler | Implemented |
| Child agent state isolation | Child `ExecutionContext` + scoped child AAM | apxm-runtime AAM | Partial (snapshot-scoped child AAMs wired for sub-flows; file-backed workspace projection still pending) |

## Workflow Graph Structure

A Codex-class coding agent session as an AIS graph:

```
                    ┌──────────────┐
                    │ QMEM         │ Load project context
                    │ (read memory)│ from LTM
                    └──────┬───────┘
                           │
                    ┌──────▼───────┐
                    │ ASK          │ Main inference loop
                    │ (with tools) │ (iterates until done)
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
        ┌─────▼─────┐ ┌───▼───┐ ┌─────▼─────┐
        │ INV       │ │ INV   │ │ INV       │ Parallel
        │ (read)    │ │ (bash)│ │ (write)   │ tool dispatch
        └─────┬─────┘ └───┬───┘ └─────┬─────┘
              │            │            │
              └────────────┼────────────┘
                           │
                    ┌──────▼───────┐
                    │ VERIFY       │ Check test results
                    └──────┬───────┘
                           │
                    ┌──────▼───────┐
                    │ BRANCH       │ Tests pass?
                    └───┬──────┬───┘
                   yes  │      │ no
                        │      │
                 ┌──────▼─┐ ┌──▼──────────┐
                 │ UMEM   │ │ ASK         │ Fix and retry
                 │ (save) │ │ (with tools)│
                 └────────┘ └─────────────┘
```

## What A-PXM Adds Over Raw Codex

1. **Compiler sees the full workflow** — can fuse operations, eliminate dead paths, extract parallelism
2. **Formal state model** — AAM transitions record belief/goal changes, and child flows/sub-agents can now execute in snapshot-scoped child AAMs
3. **Shared optimizations** — prompt caching, model routing, token compaction benefit all agents (prompt caching and model routing not yet implemented -- see `advantages/hypotheses.md` H3, H5)
4. **Checkpoint/resume** — AAM state can be serialized and restored (`AamCheckpoint` covers beliefs + goals; capabilities not included yet)
5. **Tool isolation** — capability interceptors provide auditable, policy-driven tool access (no OS-level sandboxing yet -- P0 gap)

## Implementation Strategy

Build incrementally on A-PXM, reusing frontend pieces only where they shorten the path:

1. **Phase 1**: Single-turn agent (graph + tools + interceptors; sandbox deferred until P0 gap closed)
2. **Phase 2**: Multi-turn with memory (add QMEM/UMEM + LTM persistence)
3. **Phase 3**: Multi-step workflows (AIS graphs with BRANCH/VERIFY)
4. **Phase 4**: Multi-agent coordination (FLOW_CALL + Communicate for sub-agents)
5. **Phase 5**: Optimization integration (compiler passes + runtime caching)
