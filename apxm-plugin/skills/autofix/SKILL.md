---
name: autofix
description: Automated validate/classify/fix/verify loop for APXM codebase
user-invocable: true
---

# Autofix

Automated system for finding and fixing compilation/validation issues across Python examples, Rust tests, and MLIR dialect alignment.

## When to Use

Run autofix after:
- Changes to the Python frontend (apxm/graph/)
- Compiler modifications (lower_mlir.rs, dialect/)
- AIS operation updates (AISOps.td, definitions.rs)
- Adding new Python examples
- Major refactoring

## How It Works

The autofix system has three phases:

### 1. VALIDATE

Runs all Python examples through the full pipeline:
1. Import the Python module
2. Extract the `@compile`'d graph
3. Emit `.air` via `to_air()`
4. Compile via `dekk apxm compile file.air`
5. Validate via `dekk apxm validate file.air`
6. Track pass/fail for each example

### 2. CLASSIFY

Groups failures into bug clusters:
- **attr_mismatch**: Validator reports missing/wrong attribute (e.g., "missing required attribute 'recipient'")
- **mlir_parse_error**: E900 module parsing errors (Rust emits MLIR text that parser rejects)
- **import_error**: Python import failures (missing modules, PYTHONPATH issues)
- **validation_error**: Graph validation errors (missing edges, unreachable nodes)
- **type_mismatch**: Type annotation errors (expected '!', mismatched types)
- **build_error**: Cargo/dekk build failures
- **unknown**: Anything else

### 3. FIX

Generates task prompts for each cluster:
- Lists affected files
- Shows error messages
- Provides cluster-specific fix instructions
- Includes verification commands

## Usage

### Basic audit:
```bash
python3 scripts/apxm-autofix.py
```

### Scope to specific directory:
```bash
python3 scripts/apxm-autofix.py --scope examples/python/acp-agents
```

### Report only (no task generation):
```bash
python3 scripts/apxm-autofix.py --report-only
```

### Verify after fixes:
```bash
python3 scripts/apxm-autofix.py --verify-only
```

### Auto-fix via APXM runtime (Recommended):
```bash
# Through the Python script (validates, executes workflow, re-validates)
python3 scripts/apxm-autofix.py --auto-fix

# Or directly execute the workflow
dekk apxm execute .agents/skills/autofix/autofix_workflow.air --emit-session -- "examples/python"

# With custom scope
dekk apxm execute .agents/skills/autofix/autofix_workflow.air --emit-session -- "examples/python/acp-agents"
```

The `--auto-fix` mode runs the autofix workflow through the APXM runtime, spawning agents via ACP. The workflow:
1. Runs validation to identify failures
2. Classifies failures into error clusters
3. Spawns parallel fixer agents for each cluster type
4. Runs verification to check fixes
5. Generates a summary report

## Policy Checking

The `apxm-policy-check.py` script enforces coding standards:

```bash
# Run all policy checks:
python3 scripts/apxm-policy-check.py

# Auto-generate fix suggestions:
python3 scripts/apxm-policy-check.py --fix
```

### Policy Rules

1. **No hardcoded "ais.*" strings**: Use `AISOperationType::mnemonic()` instead
2. **Python uses _generated/ constants**: Operation names come from generated modules
3. **AISOps.td matches enum**: Dialect ops align with `AISOperationType` variants
4. **Typed memory attributes**: Use `AISMemorySpaceAttr`, not `StrAttr`

## Adding New Classifiers

To add a new failure pattern:

1. Edit `scripts/apxm-autofix.py`
2. Add pattern to `FAILURE_PATTERNS` dict:
   ```python
   FAILURE_PATTERNS = {
       # ... existing patterns ...
       "new_error_type": [
           r"pattern1",
           r"pattern2",
       ],
   }
   ```
3. Add fix instructions to `get_fix_instructions()`:
   ```python
   instructions = {
       # ... existing instructions ...
       "new_error_type": """
       1. Step-by-step fix instructions
       2. Where to look
       3. What to change
       """,
   }
   ```

## The Full Loop

Typical workflow:
```bash
# 1. Make changes to compiler/frontend/dialect
vim crates/compiler/apxm-compiler/src/lower_mlir.rs

# 2. Run autofix to find issues
python3 scripts/apxm-autofix.py

# 3. Review generated tasks
cat /tmp/autofix-tasks/cluster-mlir_parse_error.txt

# 4. Fix issues using auto-fix mode
python3 scripts/apxm-autofix.py --auto-fix

# 5. Verify fixes
python3 scripts/apxm-autofix.py --verify-only

# 6. Run policy checks
python3 scripts/apxm-policy-check.py

# 7. Build and test
dekk apxm build
cargo test --workspace

# 8. Commit
git add -A
git commit -m "fix(mlir_parse): align dialect with Python emission"
```

## Dogfooding: autofix_loop.py

The `examples/python/real-world/autofix_loop.py` workflow is the "ultrathink" version — an APXM graph that:
- Spawns an architect agent to analyze failures
- Spawns parallel implementer agents per bug cluster
- Has a reviewer agent that validates fixes
- Uses COMMUNICATE to send task prompts to agents

This workflow compiles through APXM's own compiler, demonstrating self-hosting.

## Files

- `scripts/apxm-autofix.py` - Main driver (validate/classify/fix)
- `scripts/apxm-policy-check.py` - Policy enforcement
- `.agents/skills/autofix/SKILL.md` - This documentation
- `examples/python/real-world/autofix_loop.py` - APXM workflow graph

## Notes

- The script uses `PYTHONPATH=crates/compiler/apxm-frontend/python` for imports
- Examples with hyphens in paths are imported via `importlib.util`
- Temp `.air` files are written to `/tmp/` and cleaned up
- Task files are regenerated on each run (not persistent)
- Currently validates Python examples only (Rust test integration is Phase 2)
