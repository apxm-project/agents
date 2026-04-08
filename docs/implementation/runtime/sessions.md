# Sessions

Sessions manage the lifecycle of agent execution state -- checkpointing the AAM for restart continuity, tracking live agent processes, coordinating ACP subprocess lifecycles, and scoping hierarchical multi-agent execution. This is the implementation counterpart to the formal [Agent Abstract Machine (AAM)](../../pxm/aam.md) specification.

## AAM Checkpoints

An `AamCheckpoint` (`apxm-runtime/src/aam/mod.rs`) is a serializable snapshot of the full AAM state at a point in time:

```rust
pub struct AamCheckpoint {
    pub beliefs: HashMap<String, Value>,
    pub goals: Vec<Goal>,
    pub capabilities: HashMap<String, CapabilityRecord>,
    pub goal_tree: GoalTree,
    pub timestamp: DateTime<Utc>,
}
```

- **`Aam::checkpoint()`** -- creates a snapshot by cloning the inner state under a read lock
- **`Aam::restore(checkpoint)`** -- replaces the current state with the checkpoint under a write lock, rebuilding the priority queue and goal details map
- **`save_to_file(path)` / `load_from_file(path)`** -- JSON serialization to/from disk

Checkpoints are the unit of persistence: the entire (B, G, C) triple plus the goal tree is captured atomically.

## SessionManager

`SessionManager` (`apxm-runtime/src/aam/session.rs`) manages checkpoint persistence on disk, one JSON file per session ID.

### On-disk layout

```
<checkpoint_dir>/
  <session-id>.json
  <session-id>.json
  ...
```

### API

| Method | Description |
|--------|-------------|
| `new(checkpoint_dir)` | Create manager, mkdir -p the directory |
| `save_checkpoint(session_id, checkpoint)` | Write checkpoint to `<session_id>.json` |
| `load_checkpoint(session_id)` | Load checkpoint, returns `None` if no file exists |
| `list_sessions()` | Return sorted list of all session IDs with saved checkpoints |
| `delete_checkpoint(session_id)` | Remove the checkpoint file (no-op if missing) |
| `checkpoint_path(session_id)` | Return the file path for a session's checkpoint |

## ProcessTable

The `ProcessTable` (`apxm-runtime/src/process_table.rs`) is the central registry of all running agent processes and their threads. It uses `DashMap` for lock-free concurrent access.

### Data structures

- **`AgentProcess`** -- an isolated execution context with `id` (UUID v7), `name`, optional `parent_id`, `ProcessKind` (Local or External), `ProcessState` (Running/Idle/Terminated), and `spawned_at` timestamp
- **`ProcessKind::Local`** -- agent running flows via the executor engine
- **`ProcessKind::External`** -- ACP subprocess (Claude, Codex, etc.) with a type-erased `Arc<Mutex<dyn Any>>` session handle and profile name
- **`AgentThread`** -- a unit of work within a process, tracking `node_id`, `op_type`, and `ThreadState`

### Name index

A secondary `DashMap<String, ProcessId>` provides O(1) lookup by agent name. The name index also serves as the capacity gate: `active_count()` reads its length.

### Spawn safety

- **Capacity limit** -- configurable `max_processes` (default `DEFAULT_MAX_PROCESSES`), enforced before every spawn
- **Spawn depth limit** -- configurable `max_spawn_depth` (default `DEFAULT_MAX_SPAWN_DEPTH`), prevents runaway recursive SPAWN_AGENT chains by walking the parent chain
- **RAII reservation** -- `reserve_spawn_slot()` returns a `SpawnReservation` that holds a placeholder in the name index. If dropped without `commit()`, the placeholder is automatically removed, preventing name leaks on partial failures

### Injected traits

- **`AgentSpawner`** -- trait for spawning external ACP agent processes, injected by the driver
- **`AgentPrompter`** -- trait for sending prompts to external agents, injected by the driver

These traits exist because `apxm-runtime` cannot depend on `apxm-acp`; the driver bridges them.

## ACP Session Lifecycle

External agent processes follow a 3-phase lifecycle:

1. **Spawn** -- `AgentSpawner::spawn_external()` creates an `AcpSession`, injects system preamble (turn 0) with `AamContext` beliefs/goals/capabilities, and registers the process in the ProcessTable
2. **Prompt/Response** -- `AgentPrompter::prompt()` sends messages to the running session and returns the agent's response as a `Value`
3. **Close** -- `ProcessTable::close(name)` removes the process from both the name index and process map, dropping the `Arc` references so the `AcpSession` Drop impl fires and cleans up the subprocess

## Session Hierarchy

Hierarchical scoping allows child agents to inherit, snapshot, or isolate parts of the parent AAM.

### ScopePolicy (4 variants)

Defined in `apxm-core/src/types/aam.rs`:

| Variant | Semantics |
|---------|-----------|
| `Inherit` | Share parent state; child writes are visible to parent |
| `Isolate` | Start with empty state |
| `Snapshot` | Child gets a point-in-time copy; writes do not affect parent |
| `Filter(Vec<String>)` | Inherit only the listed keys (snapshot semantics for selected keys) |

### ScopeSpec

Per-dimension scope specification:

```rust
pub struct ScopeSpec {
    pub beliefs: ScopePolicy,
    pub capabilities: ScopePolicy,
    pub goals: ScopePolicy,
}
```

Convenience constructors: `ScopeSpec::default()` (all Inherit), `ScopeSpec::snapshot_all()` (all Snapshot).

### Aam::child_scope()

Creates a child AAM from a parent according to a `ScopeSpec`. Fast path: if all dimensions are `Inherit`, returns a clone sharing the same `Arc` (zero-copy). Otherwise, creates a new `AamInner` with state derived per the policy.

### ScopeRegistry

`ScopeRegistry` (`apxm-runtime/src/workspace/mod.rs`) tracks active scopes and their AAM instances in a `RwLock<HashMap<String, ScopeEntry>>`.

Each `ScopeEntry` records: `scope_id`, optional `parent_id`, `aam` (the AAM instance), and `spec` (the ScopeSpec used at creation).

Methods: `register()`, `get()`, `children_of(parent_id)`, `remove()`, `scope_ids()`.

### WorkspaceManager

Wraps a `ScopeRegistry` in an `Arc` and provides the coordination surface for multi-agent execution. Currently exposes the scope registry directly; future work will add Materializer, StateProjector, and PolicyEngine integration.

## Session Output Folders

When `--emit-session <dir>` is passed to `dekk apxm execute` or `dekk apxm run`, each execution writes a self-contained folder:

```
<dir>/<execution-id>/
  manifest.json       # execution metadata (graph name, timestamp, parameters)
  input.json           # the input graph as submitted
  results.json         # final output values for all nodes
  metrics.json         # SchedulerMetrics + TokenAccountingSnapshot
  events.jsonl         # all ApxmEvent envelopes, one per line
  node_statuses.json   # per-node status (success/failure, duration, retries)
```

Session folders enable reproducible inspection: the input graph, every node's output, all runtime events, and performance metrics are captured in one directory.

## Related Documentation

- [Agent Abstract Machine (AAM)](../../pxm/aam.md) -- formal `(B, G, C)` definition and transition function
- [Hierarchical AAM](../../design/hierarchical-aam.md) -- scoping diagrams and file-tree-as-AAM design
- [Observability](observability.md) -- event system, MetricsCollector, and CLI output flags
- [Host Integration](../host-integration.md) -- how hosts (Codex, Gemini) interact with sessions and AAM
