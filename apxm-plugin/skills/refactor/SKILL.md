---
name: refactor
description: Refactor a module or crate with analysis, implementation, and verification
user-invocable: true
---

# Refactor

Refactors a module or crate in APXM. Spawns agents to analyze, implement, and verify the refactoring. Ensures tests pass and code compiles throughout.

## What it does

This workflow orchestrates a safe refactoring:

1. **Analyzer** (Claude) — reads the code, identifies refactoring opportunities, creates step-by-step plan
2. **Implementer** (Codex) — executes the refactoring, keeping code compilable at each step
3. **Test Runner** (Claude) — runs tests, autofix, clippy; fixes any breakage

## Agents spawned

- `analyzer` (claude) — analyzes current structure, proposes new structure, identifies impact
- `implementer` (codex) — does the refactoring incrementally, runs `cargo check` at each step
- `test_runner` (claude) — runs `dekk apxm build`, `cargo test`, `apxm-autofix.py`, `cargo clippy`

## Parameters

1. `target` (str): Module/crate name or file path (e.g., "apxm-runtime" or "crates/runtime/apxm-runtime/src/executor/mod.rs")
2. `goal` (str): Refactoring goal (e.g., "Extract scheduler into its own module")

## Usage

### Quick execution:
```bash
dekk apxm execute .agents/skills/refactor/refactor.air --emit-session "apxm-runtime" "Extract scheduler into its own module"
```

### Run pre-compiled artifact:
```bash
dekk apxm run .agents/skills/refactor/refactor.apxmobj --emit-session "apxm-runtime" "Extract scheduler into its own module"
```

### Monitor progress:
```bash
ls -lt ~/.apxm/sessions/ | head -2
cat ~/.apxm/sessions/<id>/node_statuses.json
dekk apxm replay ~/.apxm/sessions/<id>
```

## Output structure

The session folder will contain:
- `nodes/01_analyzer/` — refactoring analysis (current structure, proposed structure, impact, plan)
- `nodes/02_implementer/` — refactoring changes (files created/modified/deleted)
- `nodes/03_test_runner/` — test results (build status, test results, clippy warnings, fixes made)
- `nodes/04_summary/` — refactoring summary (what changed, test status, breaking changes)
- `results.json` — final summary

## Analysis structure

The analyzer produces:
1. **Current structure** — key types, functions, modules, organization
2. **Proposed structure** — new module boundaries, what moves where, new abstractions
3. **Impact analysis** — what breaks, public API changes, what needs updating
4. **Step-by-step plan** — order of changes to maintain compilability

## Implementation guarantees

The implementer:
- Preserves functionality (refactor, not rewrite)
- Keeps code compilable at each step (`cargo check` after each change)
- Updates imports wherever needed
- Runs `cargo fmt` and `cargo clippy` at the end

## Test verification

The test runner:
- Runs `dekk apxm build`
- Runs `cargo test`
- Runs `python3 scripts/apxm-autofix.py`
- Runs `cargo clippy -- -D warnings`
- If tests fail, fixes issues and re-runs

## When to use

Use this workflow:
- To extract modules for better separation of concerns
- To rename types/functions across the codebase
- To reorganize crate structure
- To eliminate code duplication
- To improve module boundaries

## Example refactorings

- "Extract scheduler into its own module"
- "Rename ExecutionContext to RuntimeContext"
- "Move all handler functions into handlers/ subdirectory"
- "Split apxm-runtime into apxm-runtime and apxm-scheduler"
- "Consolidate error types into apxm-core"

## Notes

- The refactoring is incremental (compile after each step)
- Tests must pass before the workflow completes
- Breaking changes are explicitly flagged
- The workflow uses `APXM_HOME` for agent cwd (defaults to current directory)
