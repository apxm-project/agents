# Phase 3: APXM Changes -- Hierarchical AAM

**Source:** [Plan 1: APXM Changes](../plan-1-apxm-changes.md), Phase A6
**Draft:** v6 -- Revised with investigation findings
**Timeline:** ~3-4 weeks (revised down from ~4-5; see investigation notes)
**Dependencies:** Phase 2 (A5: Capability system extensions) complete

---

## Current State

The AAM currently uses flat in-memory structures (see `apxm-runtime/src/aam/`):

- **Beliefs**: `HashMap<String, Value>` -- single global key-value store
- **Goals**: `PriorityQueue<Goal>` with `GoalTree` for parent-child relationships (from `apxm-core/src/types/aam.rs`)
- **Capabilities**: `HashMap<String, CapabilityRecord>` -- flat registry

The `apxm-core/src/types/aam.rs` module already defines scope-related types:

- **`ScopePolicy`** enum: `Inherit`, `Isolate`, `Snapshot`, `Filter(Vec<String>)`
- **`ScopeSpec`**: per-dimension (`beliefs`, `capabilities`, `goals`) scope specification
- **`GoalTree`**: parent-child goal relationships with `CompletionPolicy` (`AllChildren`, `AnyChild`, `Manual`)

These types are wired into runtime execution: `Aam::child_scope(&self, spec: &ScopeSpec) -> Aam` is fully implemented at `aam/mod.rs:295-347`, covering all four scope policies (Inherit, Isolate, Snapshot, Filter) with 6 unit tests (`aam/mod.rs:762-956`). `ExecutionContext` carries `scope_id` and `scope_registry` (`executor/context.rs:49-51`). `ScopeRegistry` and `WorkspaceManager` exist at `workspace/mod.rs` (288 lines, 8 tests) as in-memory registries.

**What is missing:** File-tree backing (materializing in-memory AAM to `~/.apxm/workspaces/` directories), `StateProjector::promote()` direction (child -> parent state propagation), and CLI `state` commands. The file-tree-backed hierarchical AAM described in [hierarchical-aam.md](../../implementation/runtime/hierarchical-aam.md) is the target.

> **Note:** The `MemorySystem` already provides scoped operations -- `read_scoped()`, `write_scoped()`, `search_scoped()`, `delete_scoped()` -- using key-prefix namespacing (`__scope__/<scope_id>/<key>`). The per-workspace file approach described below is additive to this existing scoping, not a replacement.

---

## A6.1 WorkspaceManager

**Already exists (in-memory):** `WorkspaceManager` and `ScopeRegistry` at `apxm-runtime/src/workspace/mod.rs` (288 lines, 8 tests). `ScopeRegistry` maintains a `HashMap`-based registry of active scopes with `ScopeEntry` (scope_id, parent_id, AAM, ScopeSpec). `WorkspaceManager` wraps `ScopeRegistry` for register/get/children/remove.

**New work:** Extend `WorkspaceManager` with file-system-backed directory management:

```rust
impl WorkspaceManager {
    /// Create a new workspace directory under ~/.apxm/workspaces/ with scoped AAM structure
    pub fn create_workspace(&self, id: &str, scope: ScopeSpec) -> Result<WorkspacePath>;

    /// Open an existing workspace, loading its AAM state from files
    pub fn open_workspace(&self, id: &str) -> Result<Workspace>;

    /// Archive a completed workspace (move to ~/.apxm/workspaces/.archive/)
    pub fn archive_workspace(&self, id: &str) -> Result<()>;
}
```

This adds the `~/.apxm/workspaces/` layout with `scope.toml`, `data/`, `goals/`, and `tools/` materialization on top of the existing in-memory registry.

Each workspace gets the directory structure:

```
~/.apxm/workspaces/<workspace-id>/
+-- scope.toml           # Scope metadata (id, parent, policy, revision)
+-- data/                # B (Beliefs) -- TOML/JSON data files
+-- goals/               # G (Goals) -- goal tree as TOML
|   +-- current.toml
+-- tools/               # C (Capabilities) -- tool definitions
    +-- <registered-tools>.toml
```

---

## A6.2 Materializer

Writes AAM state to the file tree and reads it back:

```rust
pub struct Materializer;

impl Materializer {
    /// Project in-memory AAM state to the file tree
    pub fn materialize(workspace: &WorkspacePath, aam: &AamState) -> Result<()>;

    /// Load AAM state from the file tree
    pub fn hydrate(workspace: &WorkspacePath) -> Result<AamState>;
}
```

---

## A6.3 StateProjector

Enforces scoping rules (Inherit, Isolate, Snapshot, Filter) between parent and child workspaces:

```rust
pub struct StateProjector;

impl StateProjector {
    /// Create a child AAM scope from a parent, applying the scope policy
    pub fn project(parent: &AamState, policy: &ScopeSpec) -> AamState;

    /// Promote child state changes back to parent (explicit, policy-gated)
    pub fn promote(child: &AamState, parent: &mut AamState, keys: &[String]) -> Result<()>;
}
```

---

## A6.4 Three-tier memory backing

Map the memory hierarchy to the file tree:

| Tier | Current | Proposed |
|------|---------|----------|
| **STM** | `InMemoryBackend` (from `apxm-backends`) | `InMemoryBackend` (unchanged -- volatile by design) |
| **LTM** | `~/.apxm/memory/ltm.sqlite` (global) | Per-workspace `data/*.toml` files (persistent, scoped) |
| **Episodic** | `~/.apxm/memory/episodes.jsonl` (global) | Per-workspace `episodes.jsonl` (scoped trace) |

> **Note:** The `MemorySystem` already has scoped operations (`read_scoped()`, `write_scoped()`, `search_scoped()`, `delete_scoped()`) with key-prefix namespacing (`__scope__/<scope_id>/<key>`). The per-workspace file approach above is additive -- it provides physical file separation on top of the existing logical namespacing. Both mechanisms should coexist: key-prefix scoping for STM (which remains in-memory), file-path scoping for LTM and Episodic (which move to per-workspace directories).

---

## A6.5 CLI commands

```bash
apxm state show <session-id>          # Display AAM state for a workspace
apxm state show <session-id> --json   # Machine-readable AAM dump
apxm state list                       # List all workspaces
apxm state diff <session-a> <session-b>  # Diff two workspace states
```

---

## A6.6 Deliverables

| Deliverable | Location | Status |
|-------------|----------|--------|
| WorkspaceManager (file-system extension) | `apxm-runtime/src/workspace/manager.rs` | Extend existing in-memory `WorkspaceManager` at `workspace/mod.rs` |
| Materializer | `apxm-runtime/src/workspace/materializer.rs` | NEW -- full implementation needed |
| StateProjector (`promote()`) | `apxm-runtime/src/workspace/projector.rs` | NEW -- `child_scope()` exists but `promote()` does not |
| Scoped execution context | `apxm-runtime/src/executor/context.rs` (modify) | PARTIAL -- `scope_id` and `scope_registry` already wired |
| CLI `state` commands | `apxm-cli/src/commands/state.rs` | NEW -- full implementation needed |

> **Effort note:** The original ~25-day estimate assumed none of the scoping infrastructure existed. Since `child_scope()` (6 tests), `ScopeRegistry` (8 tests), `WorkspaceManager` (in-memory), and scoped memory operations are all implemented, the actual new work is: Materializer (~5-6 days), file-system WorkspaceManager extension (~3-4 days), `promote()` (~2-3 days), per-workspace memory paths (~2-3 days), CLI `state` commands (~3-4 days). Revised estimate: **~15-18 days**.

---

## A6.7 Acceptance criteria

- [ ] `apxm state show <session>` displays B, G, C from the file tree
- [ ] AAM state backed by `~/.apxm/workspaces/<id>/` directories
- [ ] Scoping rules (Inherit, Isolate, Filter) enforced between parent/child scopes
- [ ] Three-tier memory operational (STM in-memory, LTM file-backed, Episodic append-only)
- [ ] State survives process restarts (persistent file backing)

---

## Session Isolation: Preparing for Phase 4

Phase 3 workspace management lays the groundwork for per-session AAM isolation in Phase 4. The current `Runtime::build_context()` clones the shared `Aam` (`Arc<RwLock<AamState>>`), meaning all sessions share state. Phase 3 should add:

```rust
impl Runtime {
    pub fn get_or_create_session_aam(&self, session_id: &str) -> Aam {
        if let Some(scope) = self.scope_registry.get(session_id) {
            return scope.aam.clone();
        }
        let session_aam = self.aam.child_scope(&ScopeSpec::isolate());
        self.scope_registry.register(session_id.to_string(), None, session_aam.clone(), ScopeSpec::isolate());
        session_aam
    }
}
```

This does NOT change Phase 3 scope (workspace is file-backed, not server-backed). It prepares the Runtime API so Phase 4's `apxm-server` can call `get_or_create_session_aam()` in `build_context()`.

See [Session → AAM Mapping Design](../SESSION-AAM-MAPPING.md) and [Sessions & Server Investigation](../SESSIONS-AND-SERVER-INVESTIGATION.md) for full details.

---

## Cross-references

- **AAM formal model:** [pxm/aam.md](../../pxm/aam.md)
- **Hierarchical AAM design:** [implementation/runtime/hierarchical-aam.md](../../implementation/runtime/hierarchical-aam.md)
- **Session → AAM mapping:** [SESSION-AAM-MAPPING.md](../SESSION-AAM-MAPPING.md)
- **Sessions & server investigation:** [SESSIONS-AND-SERVER-INVESTIGATION.md](../SESSIONS-AND-SERVER-INVESTIGATION.md)
- **Codex consumer (C8-C9):** [codex.md](codex.md)
- **Gemini-CLI consumer (G9-G10):** [gemini-cli.md](gemini-cli.md)
- **Phase overview:** [README.md](README.md)
