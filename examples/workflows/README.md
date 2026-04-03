# Add APXM Feature (Meta-Workflow)

**Pattern:** `SPAWN × 13 → parallel COMMUNICATE × 10 → WAIT_ALL × 2 → synthesize → implement → simplify → VERIFY → commit`

## What it demonstrates

This is a **meta-workflow** — a graph that builds new APXM features by orchestrating 13 agents:

| Agent | Profile | Role |
|-------|---------|------|
| `claude_planner_1-5` | claude | Ultrathink planning (architectural perspective) |
| `codex_planner_1-5` | codex | Ultrathink planning (implementation perspective) |
| `claude_synthesizer` | claude | Merge 10 plans into one unified plan |
| `codex_implementer` | codex | Execute the plan, run tests, commit |
| `claude_simplifier` | claude | Review, simplify, polish |

## Workflow Phases

1. **Spawn** — 13 agents start in parallel
2. **Plan (Claude × 5)** — 5 Claude agents independently analyze the feature request with ultrathink
3. **Plan (Codex × 5)** — 5 Codex agents do the same, more implementation-focused
4. **Sync + Checkpoint** — `WAIT_ALL` collects all 10 plans, `CHECKPOINT` saves them
5. **Synthesize** — Claude synthesizer merges the best ideas from all 10 plans
6. **Implement** — Codex implementer executes the unified plan, runs tests
7. **Simplify** — Claude simplifier reviews and removes unnecessary complexity
8. **Verify** — `VERIFY` checks implementation quality
9. **Commit** — Codex commits the changes with a conventional commit message
10. **Persist** — `UMEM` stores the result in LTM

## Usage

```bash
# Validate
apxm validate examples/workflows/add-apxm-feature.apxm

# Run with a feature request
apxm execute examples/workflows/add-apxm-feature.apxm -- \
  "Add a ROLLBACK operation that reverts execution state to a previous CHECKPOINT"

# Another example
apxm execute examples/workflows/add-apxm-feature.apxm -- \
  "Add support for TIMEOUT attribute on COMMUNICATE that cancels if agent doesn't respond within N ms"
```

## Checkpoints

The workflow creates 5 durable checkpoints for recovery:

| Checkpoint ID | After Phase |
|---------------|-------------|
| `add-feature-claude-plans` | 5 Claude plans collected |
| `add-feature-codex-plans` | 5 Codex plans collected |
| `add-feature-synthesis` | Unified plan ready |
| `add-feature-implementation` | Code changes made |
| `add-feature-final` | Simplified + verified |

## Ops Used

`SPAWN_AGENT`, `CONST_STR`, `MERGE`, `COMMUNICATE`, `WAIT_ALL`, `CHECKPOINT`, `VERIFY`, `UMEM`, `PRINT`
