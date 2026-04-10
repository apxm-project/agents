---
name: add-op
description: Add a new AIS operation to APXM with compiler and runtime implementation
user-invocable: true
---

# Add Operation

Orchestrates the full procedure to add a new AIS operation to APXM. Spawns parallel agents for compiler-side and runtime-side implementation, with architect planning and reviewer verification.

## What it does

This workflow automates adding a new operation to the AIS (Agent Instruction Set):

1. **Architect** (Claude) reads the codebase and creates a structured implementation plan
2. **Compiler Dev** (Claude) implements compiler-side changes (enum, MLIR lowering, TableGen)
3. **Runtime Dev** (Codex) implements runtime-side changes (handler, dispatcher, tests)
4. **Reviewer** (Claude) verifies both implementations and runs tests

All agents work in parallel where possible, with proper synchronization via WAIT_ALL nodes.

## Agents spawned

- `architect` (claude) — plans implementation, wire index, attributes, mnemonic
- `compiler_dev` (claude) — modifies definitions.rs, AISOps.td, ArtifactEmitter.cpp
- `runtime_dev` (codex) — creates handler, updates dispatcher, adds tests
- `reviewer` (claude) — reviews, runs `dekk apxm build` and `cargo test`

## Parameters

1. `op_name` (str): Name of the operation (e.g., "CHECKPOINT", "GUARD")
2. `op_description` (str): What the operation does (e.g., "Save execution state for later resume")

## Usage

### Quick execution (compile + run):
```bash
dekk apxm execute .agents/skills/add-op/add_op.air --emit-session "CHECKPOINT" "Save execution state for later resume"
```

### Run pre-compiled artifact:
```bash
dekk apxm run .agents/skills/add-op/add_op.apxmobj --emit-session "CHECKPOINT" "Save execution state for later resume"
```

### Monitor progress:
```bash
# Find latest session
ls -lt ~/.apxm/sessions/ | head -2

# Watch live progress
watch -n 1 cat ~/.apxm/sessions/<id>/live.json

# View per-node status
cat ~/.apxm/sessions/<id>/node_statuses.json

# Replay timeline
dekk apxm replay ~/.apxm/sessions/<id>
```

## Output structure

The session folder will contain:
- `nodes/01_architect/` — implementation plan
- `nodes/02_compiler_dev/` — compiler changes
- `nodes/03_runtime_dev/` — runtime changes
- `nodes/04_reviewer/` — test results and verification
- `results.json` — all outputs including final synthesis

## Post-Implementation: Regenerate Codegen

After the operation is implemented and tests pass, regenerate all downstream projections:

```bash
dekk apxm codegen frontend      # Updates operations.py (OpSpec + FieldSpec) and emission.py
dekk apxm codegen typescript     # Updates generated.ts (ALL_OPERATIONS, OpSpec)
```

Verify the new operation appears in generated output:
```bash
python -c "from apxm._generated.operations import ALL_OPERATIONS; print([o.op for o in ALL_OPERATIONS if o.op == 'MY_OP'])"
```

## Notes

- The workflow uses `APXM_HOME` environment variable for agent cwd (defaults to current directory)
- Wire index is automatically determined by reading `definitions.rs`
- If tests fail, the reviewer suggests fixes
- All changes follow APXM conventions (apxm-core types, proper error handling)
- New attributes for the operation MUST be defined in `crates/core/apxm-ais/src/attrs.rs` and added to `ALL_ATTR_NAMES` (see `add-attr` skill)
