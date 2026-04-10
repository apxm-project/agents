---
name: audit
description: Comprehensive health check for APXM project (build, tests, examples, policy)
user-invocable: true
---

# Audit

Automated system for auditing the APXM project's overall health across multiple dimensions: build status, test coverage, example validation, policy compliance, code quality, and completeness.

## When to Use

Run audit:
- Before releases to verify project health
- After major refactoring to catch regressions
- Periodically (weekly/monthly) for health monitoring
- When investigating performance or stability issues
- Before starting new feature work to establish baseline

## What It Checks

The audit workflow examines six key areas:

### 1. BUILD HEALTH

Runs `dekk apxm build` and reports:
- Build status (clean/warnings/errors)
- Warning counts by type (dead_code, unused_imports, etc.)
- Error counts (if any)
- Critical issues requiring immediate attention

### 2. TEST COVERAGE

Runs `cargo test --workspace --quiet` and reports:
- Test pass/fail status
- Number of tests run, passed, failed
- Failed test names for debugging

### 3. EXAMPLE VALIDATION

Runs `apxm-autofix.py --report-only` and reports:
- Example validation status
- Pass/fail counts
- Failure types (import_error, mlir_parse_error, compile_error)

### 4. POLICY COMPLIANCE

Runs `apxm-policy-check.py` and reports:
- Policy violation status
- Violation counts by category
- Critical violations requiring fixes

### 5. CODE QUALITY

Counts TODOs, FIXMEs, and stub handlers:
- Total TODO/FIXME comments in runtime and compiler
- Stub handler count
- High-priority items flagged for attention

### 6. COMPLETENESS

Checks AIS operation coverage:
- Total AIS operations defined
- Operations mapped in artifact emitter
- Missing operations needing implementation

## Usage

### Run full audit:
```bash
# Generate the workflow
PYTHONPATH=crates/compiler/apxm-frontend/python python3 examples/python/self-hosted/audit.py > /tmp/audit.air

# Compile it
dekk apxm compile /tmp/audit.air -o /tmp/audit.apxmobj

# Execute with full scope
dekk apxm execute /tmp/audit.air --emit-session -- "full"
```

### Quick audit (pre-built):
```bash
dekk apxm execute .agents/skills/audit/audit.air --emit-session -- "full"
```

### Scoped audit:
```bash
# Build only
dekk apxm execute .agents/skills/audit/audit.air -- "build"

# Tests only
dekk apxm execute .agents/skills/audit/audit.air -- "test"

# Examples only
dekk apxm execute .agents/skills/audit/audit.air -- "examples"

# Policy only
dekk apxm execute .agents/skills/audit/audit.air -- "policy"
```

## Output

The audit workflow produces:

### 1. Structured Audit Report

Markdown-formatted report with:
- Overall project status (CLEAN/WARNINGS/ISSUES/CRITICAL)
- Per-category breakdowns
- Detailed findings for each area
- Overall assessment summary

### 2. Actionable Recommendations

Prioritized task list ranked by impact and effort:
- **Critical** (do now): blockers and critical bugs
- **High Priority** (this week): important improvements
- **Medium Priority** (this month): nice-to-haves
- **Low Priority** (backlog): future work

Each recommendation includes:
- Why it matters (impact)
- Effort estimate (trivial/small/medium/large)
- Concrete next steps

### 3. Session Output

When run with `--emit-session`, outputs are saved to:
```
~/.apxm/sessions/<execution-id>/
├── manifest.json        # Execution metadata
├── trace.ndjson        # Event stream
├── results.json        # All node outputs
├── audit_report.txt    # Structured report
└── recommendations.txt # Actionable tasks
```

## Integration with Other Tools

The audit workflow complements existing tools:
- **autofix**: Fixes specific example validation failures
- **policy-check**: Enforces coding standards
- **build**: Compiles the project
- **test**: Runs the test suite

Typical flow:
1. `dekk apxm audit` - Identify all issues
2. `python3 scripts/apxm-autofix.py --auto-fix` - Fix example issues
3. `python3 scripts/apxm-policy-check.py --fix` - Fix policy violations
4. `dekk apxm build` - Verify build is clean
5. `cargo test --workspace` - Verify tests pass
6. `dekk apxm audit` - Confirm all issues resolved

## Customization

To add new checks, edit `examples/python/self-hosted/audit.py`:

1. Add a new task step:
   ```python
   new_check_task = g.text(value="""Your check instructions...""")
   architect.ask("{new_check_task}")
   new_check_result = architect.get_last_node()
   ```

2. Update the synthesis step to include new check:
   ```python
   synthesize = g.think("synthesize_report", """
   ...
   New check status: {new_check_result}
   ...
   """)
   ```

3. Regenerate `.air` and `.apxmobj`:
   ```bash
   PYTHONPATH=crates/compiler/apxm-frontend/python python3 examples/python/self-hosted/audit.py > .agents/skills/audit/audit.air
   dekk apxm compile .agents/skills/audit/audit.air -o .agents/skills/audit/audit.apxmobj
   ```

## Graph Structure

The audit workflow is a linear pipeline:

```
spawn(architect) →
  build_check →
  test_check →
  autofix_check →
  policy_check →
  todo_check →
  missing_ops_check →
  synthesize_report →
  generate_recommendations →
  done
```

All checks run sequentially through a single `architect` agent, with `PRINT` nodes providing progress feedback.

## Files

- `examples/python/self-hosted/audit.py` - Python graph definition
- `.agents/skills/audit/SKILL.md` - This documentation
- `.agents/skills/audit/audit.air` - Compiled MLIR
- `.agents/skills/audit/audit.apxmobj` - Binary artifact

## Notes

- The audit workflow uses a single agent to maintain context across checks
- All checks produce JSON for structured parsing
- The `scope` parameter is currently informational (full audit always runs)
- Future: Add conditional branching based on scope parameter
- Session replay: `dekk apxm replay ~/.apxm/sessions/<id>` to review past audits
