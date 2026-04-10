---
name: remove-op
description: Safely remove an AIS operation from compiler, runtime, and tests
user-invocable: true
---

# Remove Operation

Safely removes an operation from the full APXM stack. Performs impact analysis, parallel removal across compiler and runtime, and verification to ensure nothing broke.

## What it does

This workflow automates removing an AIS operation:

1. **Impact Analysis** (THINK) — analyzes where the operation is used and what would break
2. **Compiler Dev** (Claude) — removes from enum, TableGen, MLIR lowering
3. **Runtime Dev** (Codex) — removes handler, dispatcher entry, tests
4. **Verifier** (Claude) — runs build + tests + autofix to ensure clean removal

## Agents spawned

- `compiler_dev` (claude) — removes from definitions.rs, AISOps.td, ArtifactEmitter.cpp
- `runtime_dev` (codex) — deletes handler file, updates mod.rs dispatcher
- `verifier` (claude) — runs `dekk apxm build`, `cargo test`, `apxm-autofix.py`

## Parameters

1. `op_name` (str): Name of the operation to remove (e.g., "OBSOLETE_OP")

## Usage

### Quick execution:
```bash
dekk apxm execute .agents/skills/remove-op/remove_op.air --emit-session "OBSOLETE_OP"
```

### Run pre-compiled artifact:
```bash
dekk apxm run .agents/skills/remove-op/remove_op.apxmobj --emit-session "OBSOLETE_OP"
```

### Monitor progress:
```bash
ls -lt ~/.apxm/sessions/ | head -2
watch -n 1 cat ~/.apxm/sessions/<id>/live.json
dekk apxm replay ~/.apxm/sessions/<id>
```

## Output structure

The session folder will contain:
- `nodes/01_impact_analysis/` — analysis of what's affected
- `nodes/02_compiler_dev/` — compiler-side removal
- `nodes/03_runtime_dev/` — runtime-side removal
- `nodes/04_verifier/` — build/test results, lingering references check
- `results.json` — final removal summary

## Wire index handling

The workflow determines whether to:
- Fully remove the wire index
- Mark it as reserved/deprecated for protocol stability

This decision is part of the impact analysis.

## Notes

- Checks examples/ and tests/ for usage before removal
- Searches for lingering references via `grep -r`
- Can handle deprecation instead of removal if needed for backward compatibility
- Uses `dekk apxm build` and `cargo test` for verification
