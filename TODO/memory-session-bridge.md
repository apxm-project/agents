# Memory ↔ Session Infrastructure: Connection Analysis & Enhancement Proposal

**Date**: 2026-04-07
**Method**: 5 deep-dive agents investigating episodic memory, session output, bridge gaps, back-pointers, and CLAUDE.md assembly

---

## Part 1: Current Architecture — The Two-Track System

APXM has two parallel data recording systems that operate **independently** during execution:

### Track 1: Dataflow Tokens (In-Memory, All Nodes)

```
Node A completes → publishes output to DashMap<TokenId, TokenState>
  → Node B's input tokens become ready → scheduler dequeues Node B
    → Worker calls collect_inputs() → reads token values from DashMap
      → Node B executes with upstream values as input Vec<Value>
```

- **Storage**: In-memory `DashMap<TokenId, TokenState>` in scheduler
- **Scope**: All nodes during a single execution
- **Lifetime**: Exists only during execution; discarded after
- **Direction**: Push-based (producer publishes, consumer auto-unblocks)

### Track 2: Session Files (Disk, Per-Node Workspaces)

```
~/.apxm/sessions/<execution-id>/
├── manifest.json              ← Static metadata, updated at finalization
├── trace.ndjson               ← Global NDJSON event stream (written live)
├── live.json                  ← Atomic progress snapshot (updated every 1s)
├── results.json               ← All node outputs (written at finalization)
├── graph_summary.json         ← Graph structure for context enrichment
└── nodes/
    ├── 01_architect_design/
    │   ├── node.json          ← Static node metadata
    │   ├── output.json        ← Node result (READ BY DOWNSTREAM via ContextAssembler)
    │   ├── prompt.txt         ← LLM prompt sent
    │   ├── response.txt       ← LLM response chunks
    │   ├── status.json        ← Completion status + duration
    │   ├── live.json          ← Running/completed state
    │   └── trace.ndjson       ← Per-node event stream
    ├── 02_spawn_coder/
    │   ├── CLAUDE.md          ← DYNAMICALLY GENERATED (includes upstream outputs)
    │   ├── output.json
    │   ├── skills/            ← Pre-copied skill prompts
    │   │   ├── extend/SKILL.md
    │   │   └── code_gen/SKILL.md
    │   └── ...
```

- **Storage**: Filesystem at `~/.apxm/sessions/<exec-id>/`
- **Scope**: Per-execution, per-node
- **Lifetime**: Persists after execution (replayable via `apxm replay`)
- **Direction**: Pull-based (ContextAssembler reads upstream output.json on demand)

### Track 3: Episodic Memory (In-Memory Ring Buffer + JSONL File)

```
~/.apxm/memory/episodes.jsonl
```

- **Storage**: VecDeque<EpisodicEntry> (max 10,000) + full-file JSONL persistence
- **Scope**: Global across ALL executions (not scoped to one execution)
- **Lifetime**: Persistent, FIFO eviction when buffer full
- **Structure per entry**:
  ```rust
  EpisodicEntry {
      id: String,              // UUID v7
      timestamp: DateTime<Utc>,
      event_type: String,      // "operation_completed:Ask", "llm_call", "flow_call:agent:flow"
      payload: Value,          // Operation result or metadata
      execution_id: String,    // Links to execution context
      // NOTE: NO node_id field
  }
  ```
- **Recorded by**: Dispatcher (op success/failure), Worker (retry events), LLM handler (token usage), UMEM handler (when tier=episodic)
- **Queried by**: REFLECT handler (for self-analysis), any QMEM with space=episodic

---

## Part 2: How the Tracks Connect (and Don't)

### What IS Connected

```
Track 1 (Tokens) ──writes──→ Track 2 (Session Files)
                                ↓
                         ContextAssembler reads output.json
                                ↓
                         CLAUDE.md assembled for downstream SPAWN_AGENT
                                ↓
                         ACP system preamble via aam_bridge
```

**The Context Enrichment Pipeline** (working correctly):

1. **Node N completes** → SessionEventEmitter writes `output.json` to `nodes/{N}_{name}/`
2. **Node N+1 (SPAWN_AGENT) starts** → `ensure_node_workspace()` triggers ContextAssembler
3. **ContextAssembler.load_upstream_outputs()** walks graph edges, reads each upstream `output.json`
4. **ContextAssembler.render_agent_doc()** builds CLAUDE.md with sections:
   - `## Task` — from node attributes (task_spec, prompt, goal) or upstream output
   - `## Role` — from agent profile description
   - `## Upstream Outputs` — literal content from upstream output.json (truncated to 2000 chars)
   - `## Available Skills` — references to `skills/*/SKILL.md`
   - `## Session` — execution_id + project root
   - `## Constraints` — from agent profile
5. **ContextStack.assemble()** builds demand-paged system prompt:
   - Session frame (execution metadata + graph summary)
   - Local frame (current node info)
   - Upstream frames (N predecessors, depth-limited by profile)
   - Budget-allocated (per-frame token limits)
   - Profile-specific scoping rules:
     - claude: depth 3, 4000 tokens/frame, no prompts
     - coder: depth 2, 3000 tokens/frame, no prompts
     - reviewer: depth unlimited, 2000 tokens/frame, includes prompts
6. **aam_bridge.render_system_prompt()** wraps with AAM sections:
   - `### Beliefs` (filtered, no `_`-prefixed internals)
   - `### Goals (by priority)` (active only, sorted by priority)
   - `### Available Capabilities`
7. **AcpSession::spawn()** sends as system preamble (turn 0)
8. **Agent reads** CLAUDE.md from `$APXM_NODE_WORKSPACE` + receives preamble via ACP

### What is NOT Connected

```
Track 3 (Episodic) ────╳────→ Track 2 (Session Files)
                       ↑
                   No bridge
                       ↓
Track 2 (Session Files) ────╳────→ Track 3 (Episodic)
```

**The Disconnections:**

| Connection | Status | Impact |
|---|---|---|
| Episodic → Session export | **Missing** | Episodic entries not in session dir; `apxm replay` can't show them |
| Session → Episodic import | **Missing** | Can't learn from previous sessions' traces |
| Episodic entry has node_id | **Missing** | Can't filter episodic by node; only by execution_id |
| Session trace has episodic_id | **Missing** | Can't cross-reference trace events to episodic entries |
| Results.json links to episodic | **Missing** | No provenance chain from result to episodic history |
| ContextAssembler reads episodic | **Missing** | CLAUDE.md doesn't include agent's learning from past executions |
| REFLECT reads session files | **Partial** | REFLECT uses episodic memory, but COULD also use session trace.ndjson for richer context |

---

## Part 3: The Full Data Flow Diagram

```
                    ┌─────────────────────────────────────────┐
                    │           Graph Execution                │
                    │                                          │
  ┌─────────┐      │   ┌──────────┐      ┌──────────┐       │
  │ Compiler │──────┼──→│ Scheduler│──────→│ Worker   │       │
  │ (artifact)│      │   │ (tokens) │      │ (execute)│       │
  └─────────┘      │   └──────────┘      └────┬─────┘       │
                    │                          │              │
                    │          ┌────────────────┼──────────┐  │
                    │          ▼                ▼          ▼  │
                    │   ┌──────────┐    ┌──────────┐  ┌─────┐│
                    │   │ Dispatcher│    │ Event    │  │ AAM ││
                    │   │(op handler)│   │ Emitter  │  │(B,G,C)│
                    │   └────┬─────┘    └────┬─────┘  └──┬──┘│
                    │        │               │           │    │
                    └────────┼───────────────┼───────────┼────┘
                             │               │           │
              ┌──────────────┼───────────────┼───────────┼──────────┐
              ▼              ▼               ▼           ▼          │
     ┌──────────────┐ ┌──────────┐  ┌──────────────┐ ┌──────────┐ │
     │   Episodic   │ │   STM    │  │ Session Files│ │ Context  │ │
     │   Memory     │ │ (scoped) │  │ (per-node)   │ │  Stack   │ │
     │              │ │          │  │              │ │          │ │
     │ episodes.jsonl│ │in-memory │  │ trace.ndjson │ │ reads    │ │
     │ ring buffer  │ │ DashMap  │  │ output.json  │ │ output   │ │
     │ 10K entries  │ │          │  │ prompt.txt   │ │ .json    │ │
     │              │ │          │  │ CLAUDE.md    │ │ files    │ │
     │ NO node_id   │ │          │  │ response.txt │ │          │ │
     └──────────────┘ └──────────┘  └──────────────┘ └────┬─────┘ │
              ╳                            ▲               │       │
              ╳ no connection              │               ▼       │
              ╳                            │        ┌──────────┐   │
              ╳────────────────────────────╳        │ CLAUDE.md│   │
                                                    │ Assembly │   │
                                                    │          │   │
                                                    │ → Agent  │   │
                                                    │ subprocess│  │
                                                    └──────────┘   │
                                                                   │
```

---

## Part 4: Proposed Enhancements

### Enhancement A: Enrich EpisodicEntry with node_id and session_path

**Current**:
```rust
pub struct EpisodicEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub payload: Value,
    pub execution_id: String,
    // Missing: node_id, session_dir
}
```

**Proposed**:
```rust
pub struct EpisodicEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub payload: Value,
    pub execution_id: String,
    pub node_id: Option<u64>,           // NEW: Which node produced this
    pub session_dir: Option<PathBuf>,   // NEW: Back-pointer to session
}
```

**Effort**: Low (~10 lines changed in episodic.rs + recording call sites)
**Impact**: Enables node-level episodic queries and session cross-references

---

### Enhancement B: Export Episodic Snapshot to Session Directory

After execution completes, export relevant episodic entries to session:

```
~/.apxm/sessions/<execution-id>/
├── episodic.ndjson    ← NEW: Episodic entries for this execution
```

**Implementation**: In session finalization (`session_output.rs`), after results.json is written:
```rust
// Filter episodic entries for this execution
let entries = memory.query_episodes(&execution_id).await?;
let path = session_dir.join("episodic.ndjson");
write_ndjson(&path, &entries)?;
```

**Effort**: Low (~15 lines)
**Impact**: Session directories become self-contained (reproducible + auditable)

---

### Enhancement C: Feed Episodic Context into CLAUDE.md Assembly

Currently ContextAssembler only reads `output.json` from upstream nodes. It could also include relevant episodic history:

```markdown
## Upstream Outputs
### architect_plan
{"plan": "Step 1: analyze\nStep 2: implement"}

## Agent History                    ← NEW SECTION
- Previous execution patterns:
  - operation_completed:Ask → 95% success rate
  - average response time: 2.3s
  - common tools used: bash (42%), read (31%)
```

**Implementation**: ContextAssembler queries episodic memory for recent entries matching the current agent profile or operation type.

**Effort**: Medium (~40 lines in context_assembler.rs)
**Impact**: Agents get historical context, enabling self-improvement across executions

---

### Enhancement D: Session Trace → Episodic Import (Cross-Execution Learning)

Enable loading a previous session's trace into episodic memory:

```bash
apxm replay --import-episodes ~/.apxm/sessions/exec-123/
```

This would parse `trace.ndjson` and `results.json`, creating episodic entries for each node's execution. Enables learning from replayed sessions without re-executing.

**Effort**: Medium (~50 lines in CLI + episodic.rs)
**Impact**: Cross-execution knowledge transfer; agent improves over time

---

### Enhancement E: Unified Event Pipeline

Replace the parallel recording pattern (episodic + session events independently) with a single pipeline:

```
Operation completes
  → UnifiedEventEmitter
    → writes to session trace.ndjson (per-execution, per-node)
    → writes to episodic memory (global ring buffer)
    → publishes to EventBus (if subscribers exist)
```

Currently:
- Dispatcher records to episodic memory (lines 147-154)
- SessionEventEmitter writes to trace.ndjson (lines 681-722)
- These happen independently with no shared event ID

**With unification**:
- Single event ID shared across all three sinks
- Cross-referencing: `episodic.id == trace_event.id == eventbus_event.id`
- EventBus finally gets wired (currently orphaned)

**Effort**: Medium-High (~100 lines across dispatcher, session_output, events)
**Impact**: Complete observability; every event traceable across all three systems

---

### Enhancement F: ContextStack Reads from Memory, Not Just Files

Currently `ContextStack::upstream_frame_content()` reads `output.json` from session files. It could also read from STM/LTM for richer context:

```rust
fn upstream_frame_content(&self, node_id: u64, ...) -> Option<String> {
    // Current: file-based only
    let output = load_node_output(&self.session_dir, node_id, &name);

    // Enhanced: also query memory for additional context
    let memory_context = self.memory.read_scoped(Stm, scope_id, &format!("node:{}", node_id)).await;
    let episodic_context = self.memory.query_episodes_for_node(node_id).await;

    // Combine file output + memory context + episodic history
}
```

**Effort**: Medium (~30 lines)
**Impact**: Richer agent context; includes runtime beliefs alongside static outputs

---

## Part 5: Priority Ranking

| # | Enhancement | Effort | Impact | Priority |
|---|---|---|---|---|
| A | Add node_id to EpisodicEntry | Low | High (enables all other enhancements) | **P0** |
| B | Export episodic to session dir | Low | Medium (self-contained sessions) | **P1** |
| E | Unified event pipeline | Medium-High | High (complete observability) | **P1** |
| C | Episodic context in CLAUDE.md | Medium | Medium (cross-execution learning) | **P2** |
| F | ContextStack reads memory | Medium | Medium (richer context) | **P2** |
| D | Session → Episodic import | Medium | Low (replay learning) | **P3** |

**Recommendation**: Start with Enhancement A (node_id in EpisodicEntry) as it's low effort and enables everything else. Then B (export to session) makes sessions self-contained. Then E (unified pipeline) closes the architectural gap permanently.

---

## Part 6: Key File Reference

| Component | File | Lines |
|---|---|---|
| EpisodicEntry struct | `crates/runtime/apxm-runtime/src/memory/episodic.rs` | — |
| Episodic recording | `crates/runtime/apxm-runtime/src/executor/dispatcher.rs` | 147-154 |
| Session event emission | `crates/orchestration/apxm-driver/src/session_output.rs` | 608-832 |
| Node workspace creation | `crates/orchestration/apxm-driver/src/session_output.rs` | 404-483 |
| ContextAssembler | `crates/orchestration/apxm-driver/src/context_assembler.rs` | 47-177 |
| ContextStack | `crates/runtime/apxm-runtime/src/context_stack/mod.rs` | 87-149 |
| ScopeRules (per-profile) | `crates/runtime/apxm-runtime/src/context_stack/policy.rs` | — |
| AAM bridge rendering | `crates/orchestration/apxm-acp/src/aam_bridge.rs` | 9-73 |
| SPAWN_AGENT workspace | `crates/runtime/apxm-runtime/src/executor/handlers/spawn_agent.rs` | 106-154, 247-301 |
| EventBus (orphaned) | `crates/apxm-events/src/bus.rs` | 1-96 |
| REFLECT (uses episodic) | `crates/runtime/apxm-runtime/src/executor/handlers/reflect.rs` | 123-130 |
| Worker event recording | `crates/runtime/apxm-runtime/src/scheduler/worker.rs` | 522-562 |
