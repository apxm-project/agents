# Sessions + Memory Architecture

## Problem

APXM graphs execute statelessly: each `apxm.run()` starts with a blank AAM. Multi-turn agent loops (chatbots, iterative planners, research agents) need conversation history and learned facts to survive across invocations. OpenAI Agents SDK solves this with a `Session` protocol (`get_items`, `add_items`, `pop_item`, `clear_session`) backed by pluggable stores. APXM needs an equivalent that maps cleanly onto the existing AAM and memory subsystems rather than bolting on a parallel state layer.

## Authoring API

```python
from apxm import InMemorySession, SQLiteSession, RedisSession

# Constructors
session = InMemorySession()
session = SQLiteSession(session_id="user-42", db_path="./sessions.db")
session = RedisSession(session_id="user-42", url="redis://localhost:6379")

# Passed at invocation time
result = apxm.run(graph, input="hello", session=session)
result = agent.ask("hello", session=session)
```

`session=` is a **runtime argument**, never compiled into the graph. The Python frontend passes the session ID and backend type as execution metadata alongside the input payload. If `session=None` (default), the runtime allocates a throwaway in-memory session scoped to that execution -- identical to today's behavior.

## AAM Mapping

The AAM already tracks beliefs, goals, capabilities, and episodic transitions (`crates/runtime/apxm-runtime/src/aam/mod.rs`). Sessions are **persistent AAM snapshots** -- not a separate concept:

- **Conversation history** maps to a belief key `_session:history` (a `Value::Array` of turn objects).
- **Learned facts** map to LTM facts with `session_id` tagging (`crates/runtime/apxm-runtime/src/memory/facts.rs:26` -- the field already exists).
- **Agent state** (goals, capabilities) persists via the existing `AamCheckpoint` mechanism (`crates/runtime/apxm-runtime/src/aam/session.rs`).

On execution start, the runtime loads the session's `AamCheckpoint` and history into the AAM. On execution end, it snapshots back. The `SessionManager` at `aam/session.rs` already does checkpoint save/load to disk -- sessions generalize this to pluggable backends.

## AIS Operations

No new ops required. The existing `QMEM` / `UMEM` pair (`crates/core/apxm-ais/src/operations/definitions.rs:758-819`) with `memory_tier` already handles scoped reads and writes. Session history is just a well-known key in STM that the runtime pre-populates and post-flushes.

What changes is the **runtime handler**, not the AIS surface. The executor's QMEM handler (`crates/runtime/apxm-runtime/src/executor/handlers/qmem.rs`) already calls `memory.search_scoped(space, scope_id, ...)`. Sessions set `scope_id` to the session ID, so memory reads/writes are automatically session-scoped.

If the compiler detects `Agent.with_session=True` in the Python frontend, it stamps a `session_aware = true` attribute on the AGENT node. The runtime uses this to gate session load/flush at execution boundaries -- no new op nodes are inserted.

## Backend Trait

```rust
// crates/runtime/apxm-runtime/src/session/backend.rs
#[async_trait]
pub trait SessionBackend: Send + Sync {
    async fn load(&self, session_id: &str) -> Result<Option<SessionState>>;
    async fn save(&self, session_id: &str, state: &SessionState) -> Result<()>;
    async fn delete(&self, session_id: &str) -> Result<()>;
    async fn list_sessions(&self) -> Result<Vec<String>>;
}

pub struct SessionState {
    pub checkpoint: AamCheckpoint,
    pub history: Vec<Value>,  // conversation turns
    pub metadata: HashMap<String, Value>,
}
```

Wire this via a `SessionRegistry` following the same `RwLock<HashMap<String, Arc<dyn SessionBackend>>>` pattern used by `LLMRegistry` (`crates/runtime/apxm-backends/src/llm/registry/mod.rs`). Register backends at driver startup (`crates/compiler/apxm-driver/src/runtime/llm.rs` is the analog).

Implementations:
- **InMemoryBackend**: `DashMap<String, SessionState>`. No persistence. Testing and ephemeral use.
- **FileBackend**: Wraps the existing `SessionManager` at `aam/session.rs`. JSON files in `.apxm/sessions/`.
- **SQLiteBackend**: Reuses `apxm-backends`' `SqliteBackend` (`crates/runtime/apxm-backends/src/storage/sqlite.rs`) with a `sessions` table. Single WAL-mode database. This is the default.
- **RedisBackend**: New. `redis-rs` with JSON serialization. For multi-process / distributed deployments.

## Compile-Time vs Runtime

| Concern | Where | Mechanism |
|---|---|---|
| `session_aware` attribute | Compile-time | Stamped on AGENT node by Python `@compile` decorator |
| Session ID, backend selection | Runtime | Passed as execution metadata in the run request |
| History injection into LLM context | Runtime | Executor prepends `_session:history` to LLM message arrays |
| Checkpoint save/load | Runtime | Pre/post hooks in `WorkflowRunner` (`crates/runtime/apxm-runtime/src/workflow/runner.rs`) |

The compiler emits no session-specific ops. It only validates that `session_aware` graphs don't use `ScopePolicy::Isolate` on beliefs (which would prevent history propagation).

## Hard Problems

**Session ID collision.** UUIDv7 for auto-generated IDs. User-supplied IDs are accepted as-is -- collisions are the caller's responsibility. Document this.

**Concurrent runs sharing a session.** Two `apxm.run()` calls with the same `session_id` race on checkpoint save. Solution: optimistic locking with a `version` field in `SessionState`. The SQLite backend uses `UPDATE ... WHERE version = ?`; Redis uses `WATCH`/`MULTI`. If the version check fails, the runtime retries load-merge-save once, then errors with `E_SESSION_CONFLICT`.

**Schema migrations.** `SessionState` is versioned (`schema_version: u32`). On load, if the stored version is older, the backend runs forward migrations in-place. Breaking changes (removing a field from `AamCheckpoint`) bump the major version and require explicit `apxm session migrate` CLI action.

**Tombstoned items.** Deleted history items are soft-deleted (`_tombstoned: true`) and excluded from `get_items()`. Hard deletion happens on compaction (triggered by item count threshold or explicit `session.compact()`). This prevents resurrection during concurrent-merge retries.

**History growth.** Unbounded history blows up LLM context. Default cap: 100 turns. Configurable via `SessionSettings(max_history=N)`. Older turns are evicted FIFO. For long-running agents, integrate with the existing LTM facts system -- summarize old turns into facts, then evict the raw turns.

## Phased Rollout

**Phase 1 -- In-Memory (MVP).** `InMemorySession` + `FileBackend` (reuse existing `SessionManager`). History as a belief key. No concurrency controls. Unblocks `Agent.ask()` multi-turn loops.

**Phase 2 -- SQLite.** `SQLiteBackend` with WAL mode, optimistic locking, schema versioning. `apxm session list/show/delete` CLI commands. Default backend. Compaction support.

**Phase 3 -- Redis.** `RedisBackend` for distributed deployments. TTL-based expiry. Pub/sub session events for multi-node coordination. Optional encryption-at-rest for session state.
