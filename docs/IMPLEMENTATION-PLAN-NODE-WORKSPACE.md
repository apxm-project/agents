# Implementation Plan: Node Workspace + Agent Context Assembly

**Status**: Ready for implementation
**Assignee**: Lead architect
**Estimated effort**: 3-5 days (16-24 hours)
**Complexity**: Medium-High
**Risk level**: Medium

---

## Overview

Extend APXM's session output system to create per-node workspace folders with agent context assembly. This enables ACP agents to receive tailored context (CLAUDE.md/AGENTS.md) that includes upstream outputs, available skills, and role-specific constraints.

**Key components**:
1. Per-node folder creation in `SessionEventEmitter`
2. Skill resolver for mapping operations → skill directories
3. Context assembler for generating CLAUDE.md/AGENTS.md per node
4. CWD auto-routing for SPAWN_AGENT nodes

**Affected crates** (bottom-up):
- `apxm-core` — constants for node workspace files (**already complete**)
- `apxm-driver` — SkillResolver, ContextAssembler, SessionEventEmitter (**already exists**)
- `apxm-runtime` — wire emit calls into handlers + scheduler (**main work**)
- `apxm-cli` — no changes needed (session_dir already passed)

---

## Current State (2026-04-04)

**Already implemented**:
✅ `crates/apxm-driver/src/skill_resolver.rs` — maps operations to skills
✅ `crates/apxm-driver/src/context_assembler.rs` — generates CLAUDE.md/AGENTS.md
✅ `crates/apxm-driver/src/session_output.rs` — SessionEventEmitter with node workspace support
✅ `crates/apxm-core/src/constants.rs::session::node` — constants for node files
✅ `crates/apxm-core/src/paths.rs::session_node_dir_name()` — node folder naming
✅ `crates/apxm-runtime/src/executor/events.rs` — trait methods `emit_node_output`, `emit_llm_prompt`, `emit_llm_token_for_node`
✅ `crates/apxm-runtime/src/executor/handlers/spawn_agent.rs` — CWD auto-routing (lines 106-118)

**Remaining work**:
🔲 Wire `emit_node_output` into sequential executor (engine.rs)
🔲 Wire `emit_node_output` into parallel executor (dataflow.rs worker loop)
🔲 Wire `emit_llm_prompt` into llm.rs handler
🔲 Wire `session_dir` through ExecutionContext
🔲 Add integration tests for node workspace creation
🔲 Document the feature in CLAUDE.md

---

## File-by-File Implementation Plan

### Phase 1: Wire `emit_node_output` into executors

**Objective**: Ensure `emit_node_output` is called when node results are stored.

#### File 1.1: `crates/apxm-runtime/src/executor/engine.rs`

**Lines affected**: ~133-135 (after line 131 in sequential executor)

**Current code** (lines 124-135):
```rust
match result {
    Ok(value) => {
        // Store result for output tokens
        {
            let mut results = node_results.write().await;
            for output_token_id in &node.output_tokens {
                results.insert(*output_token_id, value.clone());
            }
        }
        if let Some(emitter) = &self.context.event_emitter {
            emitter.emit_node_output(node.id, &value);
        }
```

**Status**: ✅ **ALREADY IMPLEMENTED** (line 133)

**No changes needed** — the call is already wired.

---

#### File 1.2: `crates/apxm-runtime/src/scheduler/dataflow.rs` (worker loop)

**File**: `crates/apxm-runtime/src/scheduler/worker.rs` (worker loop is extracted here)

**Lines affected**: Search for "Store result in token store" or similar

**Current approach**: Need to read `worker.rs` to locate where results are stored after successful execution.

**Action**:
```rust
// After storing result in token store:
if let Some(emitter) = &ctx.event_emitter {
    emitter.emit_node_output(node_id, &result);
}
```

**Risk**: Low — read-only addition, no side effects

---

### Phase 2: Wire `emit_llm_prompt` into llm.rs

**Objective**: Write `prompt.txt` at the start of LLM operations.

#### File 2.1: `crates/apxm-runtime/src/executor/handlers/llm.rs`

**Lines affected**: ~510 (right after `LLMRequest::new(prompt.clone())`)

**Current code** (line 510):
```rust
let mut request = LLMRequest::new(prompt.clone());
```

**New code** (insert after line 510):
```rust
let mut request = LLMRequest::new(prompt.clone());

// Emit prompt for session tracing
if let Some(emitter) = &ctx.event_emitter {
    emitter.emit_llm_prompt(node.id, &prompt);
}
```

**Risk**: Low — defensive emit, no behavioral change

---

### Phase 3: Wire `session_dir` through ExecutionContext

**Objective**: Make session directory path accessible in handlers (for CWD auto-routing).

#### File 3.1: `crates/apxm-runtime/src/executor/context.rs`

**Lines affected**: ExecutionContext struct definition

**Current approach**: Check if `metadata: HashMap<String, String>` already exists in ExecutionContext (likely yes based on spawn_agent.rs line 110).

**Verification** (spawn_agent.rs:110):
```rust
ctx.metadata.get(metadata::SESSION_DIR)
```

**Status**: ✅ **ALREADY IMPLEMENTED** — `session_dir` is passed via `ctx.metadata` with key `constants::runtime::metadata::SESSION_DIR`.

**No changes needed**.

---

#### File 3.2: `crates/apxm-driver/src/runtime.rs` (RuntimeExecutor)

**Lines affected**: Where ExecutionContext is constructed before execution

**Current approach**: Check if `session_dir` is already passed into metadata when `--emit-session` is used.

**Action**: Read `runtime.rs` to verify metadata injection.

**Expected code** (in `execute_artifact_with_args`):
```rust
if let Some(session_dir) = session_output.as_ref().map(|s| s.session_dir().display().to_string()) {
    ctx.metadata.insert(
        constants::runtime::metadata::SESSION_DIR.to_string(),
        session_dir,
    );
}
```

**Risk**: Low — likely already implemented

---

### Phase 4: Integration Testing

**Objective**: Validate end-to-end node workspace creation.

#### File 4.1: New test in `crates/apxm-driver/tests/session_output_integration.rs`

**Test cases**:
1. **test_node_workspace_creation**: Run a 3-node graph (CONST_STR → SPAWN_AGENT → PRINT) with `--emit-session`, assert:
   - `nodes/01_const_str/` exists
   - `nodes/02_spawn_agent/` contains `CLAUDE.md`, `skills/extend/SKILL.md`
   - `nodes/03_print/` has `output.json`

2. **test_llm_prompt_and_response**: Run an ASK node, assert:
   - `nodes/01_ask/prompt.txt` contains the prompt
   - `nodes/01_ask/response.txt` contains LLM output
   - `nodes/01_ask/output.json` exists

3. **test_skill_resolution**: Run SPAWN_AGENT with profile="claude", assert:
   - Skill directory is copied (not symlinked)
   - Multiple skills present if op_type + profile both resolve

**Location**: Create new file or extend existing `crates/apxm-driver/tests/` suite.

**Dependency**: Requires `.agents/skills/claude/SKILL.md` fixture in test data.

---

### Phase 5: Documentation

#### File 5.1: Update `CLAUDE.md`

**Section**: Add new "Session Output" subsection under "## CLI Commands"

**Content**:
```markdown
### Session Output: Per-Node Workspaces

When `--emit-session` is passed, APXM creates a per-node workspace:

```
~/.apxm/sessions/<exec-id>/
└── nodes/
    ├── 01_spawn_architect/
    │   ├── CLAUDE.md          ← agent context (role, upstream, skills)
    │   ├── node.json           ← metadata (id, op, attributes)
    │   ├── live.json           ← runtime status
    │   ├── output.json         ← result value
    │   ├── status.json         ← final status (duration, retries, error)
    │   ├── trace.ndjson        ← per-node event stream
    │   └── skills/
    │       └── extend/
    │           └── SKILL.md
    ├── 06_architect_plans/    (ASK node)
    │   ├── prompt.txt         ← LLM prompt
    │   ├── response.txt       ← streamed LLM output
    │   ├── output.json
    │   └── ...
    └── 09_coder_implements/
        └── ...
```

**SPAWN_AGENT CWD auto-routing**: If no explicit `cwd` attribute is set, the node workspace becomes the agent's working directory automatically. Agents read `CLAUDE.md`/`AGENTS.md` from their cwd.
```

---

## Detailed Work Breakdown

### Bottom-up implementation order (by crate dependency)

#### Tier 1: apxm-core (foundation) ✅ COMPLETE
- Constants already defined in `constants.rs::session::node`
- Helper `session_node_dir_name()` already exists in `paths.rs`

#### Tier 2: apxm-driver (orchestration) ✅ COMPLETE
- SkillResolver already implemented
- ContextAssembler already implemented
- SessionEventEmitter already integrated into session_output.rs

#### Tier 3: apxm-runtime (execution) 🔲 IN PROGRESS
**3.1 Wire emit calls**:
- ✅ `engine.rs` sequential executor — already calls `emit_node_output` (line 133)
- 🔲 `scheduler/worker.rs` parallel executor — need to add `emit_node_output` after result store
- 🔲 `handlers/llm.rs` — add `emit_llm_prompt` after line 510

**3.2 Verify session_dir plumbing**:
- ✅ `spawn_agent.rs` already reads `ctx.metadata.get(metadata::SESSION_DIR)` (line 110)
- 🔲 Verify `driver/runtime.rs` injects `SESSION_DIR` into metadata when `--emit-session` is used

**Estimated effort**: 1-2 hours

---

#### Tier 4: Integration tests 🔲 TODO
**Location**: `crates/apxm-driver/tests/session_output_integration.rs` (new or extend existing)

**Test fixtures needed**:
- Minimal `.agents/skills/claude/SKILL.md`
- Simple 3-node graph JSON

**Test cases** (see Phase 4 above)

**Estimated effort**: 2-3 hours

---

#### Tier 5: Documentation 🔲 TODO
- Update `CLAUDE.md` with session output section
- Add example to `docs/guides/debugging.md` showing how to inspect node workspaces

**Estimated effort**: 1 hour

---

## Risk Assessment

### High-risk areas
None — feature is largely isolated to session output subsystem.

### Medium-risk areas
1. **Worker loop emit**: Adding `emit_node_output` in parallel scheduler worker loop
   - **Mitigation**: Read `worker.rs` carefully to find exact insertion point
   - **Fallback**: If worker loop is complex, emit only in sequential path initially

2. **LLM prompt emission timing**: Inserting emit before LLM request
   - **Mitigation**: Defensive — emit doesn't affect request execution
   - **Risk**: If prompt is mutated after emit (e.g., for schema retries), prompt.txt may differ from final request
   - **Acceptable**: prompt.txt shows initial prompt, which is more useful for debugging

### Low-risk areas
- SkillResolver, ContextAssembler already tested
- SessionEventEmitter already tested
- CWD auto-routing already implemented in spawn_agent.rs
- Constants already defined

---

## Testing Strategy

### Unit tests (already exist)
✅ `skill_resolver.rs::tests::resolves_spawn_agent_profile_skill`
✅ `context_assembler.rs::tests::assembles_agents_doc_with_upstream_output`

### Integration tests (to be added)
🔲 `session_output_integration.rs::test_node_workspace_creation`
🔲 `session_output_integration.rs::test_llm_prompt_and_response`
🔲 `session_output_integration.rs::test_skill_resolution`

### Manual validation
Run a real multi-agent graph with `dekk apxm execute --emit-session` and inspect:
```bash
dekk apxm execute examples/flagship/architect-implement-review.apxm --emit-session
ls -R ~/.apxm/sessions/$(ls -t ~/.apxm/sessions | head -1)/nodes/
cat ~/.apxm/sessions/$(ls -t ~/.apxm/sessions | head -1)/nodes/01_*/CLAUDE.md
```

---

## Implementation Checklist

### Must-have (P0)
- [x] SkillResolver implementation
- [x] ContextAssembler implementation
- [x] SessionEventEmitter node workspace creation
- [x] emit_node_output wired in sequential executor
- [ ] emit_node_output wired in parallel executor (worker.rs)
- [ ] emit_llm_prompt wired in llm.rs
- [ ] session_dir metadata injection verified
- [ ] Integration test: node workspace creation

### Nice-to-have (P1)
- [ ] Integration test: LLM prompt/response files
- [ ] Integration test: skill resolution
- [ ] Documentation: CLAUDE.md session output section
- [ ] Example: debugging.md with node workspace inspection

### Future (P2)
- [ ] Per-node logs aggregation (stdout/stderr capture from ACP subprocesses)
- [ ] Skill template variables (e.g., `{execution_id}` in SKILL.md)
- [ ] Hierarchical skill inheritance (parent node skills available to children)

---

## Open Questions

### Q1: Should all nodes get workspace folders, or only SPAWN_AGENT/COMMUNICATE?
**Decision**: All nodes get folders (current implementation).
**Rationale**: Uniform structure simplifies replay/debugging. Overhead is minimal (mkdir + 3 JSON files).

### Q2: What if a skill directory doesn't exist?
**Decision**: SkillResolver silently skips missing skills (returns empty Vec).
**Rationale**: Graceful degradation — node workspace is created, but no skills/ subdir.

### Q3: Should prompt.txt show the initial prompt or final prompt (after retries)?
**Decision**: Initial prompt (emit right after `LLMRequest::new()`).
**Rationale**: More useful for debugging — shows what user provided. Final prompt (with retry scaffolding) is less readable.

### Q4: What happens if node_name has special characters?
**Decision**: `session_node_dir_name()` sanitizes to alphanumeric + `_` (already implemented in paths.rs).
**Example**: "Spawn Architect!" → "01_spawn_architect"

---

## Performance Considerations

### Disk I/O impact
- **Per-node overhead**: ~10 file operations (mkdir, 5× JSON write, 1× SKILL.md copy)
- **For 100-node graph**: ~1000 file ops over 30-60 seconds → negligible
- **Mitigation**: Already implemented — writes are buffered, no syncs

### Memory overhead
- **Node metadata map**: `HashMap<u64, WorkspaceNodeMetadata>` — ~1KB per node
- **For 100-node graph**: ~100KB — negligible
- **LLM token buffers**: `HashMap<u64, Vec<String>>` — unbounded for streaming LLM responses
- **Mitigation**: Flush to `response.txt` on node completion, then drop Vec

---

## Dependencies

### Build dependencies (no changes)
All required crates already in Cargo.toml:
- `serde_json` (context serialization)
- `tokio::fs` (async file I/O — already used in session_output.rs)

### Runtime dependencies (no changes)
- Project must have `.agents/skills/` directory with SKILL.md files (optional — graceful if missing)
- Session directory must be writable (already enforced by `SessionOutputWriter::new`)

---

## Migration Path

**Backward compatibility**: ✅ Fully backward compatible
- Old artifacts work unchanged
- Existing session output files (manifest.json, results.json) unchanged
- New `nodes/` directory is additive

**Feature flag**: None needed — controlled by `--emit-session` CLI flag (already exists)

**Rollout strategy**:
1. Deploy to staging with integration tests
2. Run manual validation on flagship example
3. Merge to main

---

## Success Criteria

### Definition of Done
- [ ] All P0 checklist items completed
- [ ] Integration tests pass on CI
- [ ] Manual validation: 3-node graph creates correct node workspace structure
- [ ] Documentation updated in CLAUDE.md
- [ ] No performance regression (session creation <5% slower)

### Acceptance criteria
1. **Functional**: Run `dekk apxm execute graph.apxm --emit-session`, verify:
   - Every node has a `nodes/{id}_{name}/` folder
   - SPAWN_AGENT nodes have CLAUDE.md or AGENTS.md
   - ASK/THINK/REASON nodes have prompt.txt and response.txt
   - Skills are copied (not symlinked)

2. **Performance**: 100-node graph execution with `--emit-session` completes in <120% of baseline time

3. **Correctness**: Upstream outputs in CLAUDE.md match actual node outputs from previous nodes

---

## Next Steps (Immediate Actions)

### Step 1: Read worker.rs to locate result store
```bash
rg "store.*result" crates/apxm-runtime/src/scheduler/worker.rs
```
Identify exact line where result is stored after successful execution, add emit call.

### Step 2: Add emit_llm_prompt in llm.rs
```bash
# Open llm.rs at line 510
# Insert emit call after LLMRequest::new()
```

### Step 3: Verify session_dir metadata injection
```bash
rg "SESSION_DIR" crates/apxm-driver/src/runtime.rs
```
Confirm that `ctx.metadata.insert(SESSION_DIR, ...)` exists in RuntimeExecutor.

### Step 4: Write integration test
```bash
# Create crates/apxm-driver/tests/session_output_integration.rs
# Use tempdir fixture for session output
# Assert node workspace structure
```

### Step 5: Manual validation
```bash
dekk apxm execute examples/flagship/architect-implement-review.apxm --emit-session
tree ~/.apxm/sessions/$(ls -t ~/.apxm/sessions | head -1)
```

---

## Appendix A: File Dependency Graph

```
apxm-core/constants.rs (session::node)
    ↓
apxm-core/paths.rs (session_node_dir_name)
    ↓
apxm-driver/skill_resolver.rs
apxm-driver/context_assembler.rs
    ↓
apxm-driver/session_output.rs (SessionEventEmitter)
    ↓
apxm-runtime/executor/events.rs (ExecutionEventEmitter trait)
    ↓
apxm-runtime/executor/engine.rs (sequential executor)
apxm-runtime/scheduler/worker.rs (parallel executor)
apxm-runtime/executor/handlers/llm.rs (LLM handler)
    ↓
apxm-driver/runtime.rs (RuntimeExecutor)
    ↓
apxm-cli (CLI entry point)
```

---

## Appendix B: Example Session Directory Layout

After running `dekk apxm execute architect-implement-review.apxm --emit-session`:

```
~/.apxm/sessions/2026-04-04T15-30-45-abc123/
├── manifest.json           (execution metadata)
├── input.apxm              (copy of source graph)
├── results.json            (all node outputs)
├── metrics.json            (execution metrics)
├── node_statuses.json      (per-node status)
├── trace.ndjson            (session-level event stream)
├── live.json               (current progress)
└── nodes/
    ├── 01_spawn_architect/
    │   ├── CLAUDE.md
    │   ├── node.json
    │   ├── live.json
    │   ├── output.json
    │   ├── status.json
    │   ├── trace.ndjson
    │   └── skills/
    │       └── extend/
    │           ├── SKILL.md
    │           └── extend.apxm
    ├── 02_architect_task/
    │   ├── node.json
    │   ├── live.json
    │   ├── output.json
    │   └── status.json
    ├── 03_architect_plans/
    │   ├── CLAUDE.md
    │   ├── node.json
    │   ├── live.json
    │   ├── prompt.txt         ← ASK node LLM prompt
    │   ├── response.txt       ← ASK node LLM output
    │   ├── output.json
    │   ├── status.json
    │   └── trace.ndjson
    └── ... (more nodes)
```

---

**End of Implementation Plan**
