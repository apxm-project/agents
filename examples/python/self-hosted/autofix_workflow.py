#!/usr/bin/env python3
"""autofix_workflow.py - Autofix as APXM Graph

The autofix loop we built, but as a native APXM workflow graph.

Graph structure:
- exc: run validation (apxm-autofix.py --report-only)
- think: classify — parse output, identify failure clusters
- branch: if all pass → done, else → fix
- spawn fixer_1..N (claude) — one per failure cluster, run in parallel
- exc: run verification (apxm-autofix.py --verify-only)
- think: report — summarize what was fixed

Usage:
    PYTHONPATH=crates/apxm-frontend/python python3 examples/python/self-hosted/autofix_workflow.py > /tmp/autofix.air
    dekk apxm compile /tmp/autofix.air -o /tmp/autofix.apxmobj
    dekk apxm execute /tmp/autofix.air "examples/python"
"""

import os
from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude, codex


@compile()
def autofix_workflow(g: GraphRecorder):
    """Autofix loop as an APXM workflow.

    Parameters:
        scope (str): Scope to validate (e.g., "examples/python", "examples/python/acp-agents")
    """
    g.param("scope", "str")

    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents
    validator = g.spawn("validator", profile=claude, cwd=cwd)

    # Step 1: Run validation
    validate_task = g.text(value="""Run the APXM autofix validation for scope: examples/python

Execute:
  python3 scripts/apxm-autofix.py --scope examples/python --report-only

Report the full output (it will show validation results, task files created in /tmp/autofix-tasks/, etc.)
"""
    )

    validator.ask("{0}")
    validate_task | validator.get_last_node()

    print1 = g.print("=== VALIDATION OUTPUT ===\n{0}")
    validator.get_last_node() | print1

    # Step 2: Classify failures into clusters
    classify = g.think(
        "classify_failures",
        template="""Analyze the autofix validation output and classify failures:

Validation output from validator:
{0}

Parse the output and create failure clusters:
1. Group by error type (import_error, mlir_parse_error, compile_error, etc.)
2. For each cluster, identify:
   - How many files affected
   - Common root cause (if any)
   - Suggested fix strategy

Output JSON:
{{
  "clusters": [
    {{
      "type": "import_error",
      "count": 3,
      "files": ["path1", "path2", "path3"],
      "root_cause": "...",
      "fix_strategy": "..."
    }},
    ...
  ],
  "total_failures": N,
  "status": "pass" | "fail"
}}

If status is "pass", output {{"status": "pass", "clusters": []}}.
"""
    )
    validator.get_last_node() | classify
    print1 >> classify

    print2 = g.print("=== FAILURE CLASSIFICATION ===\n{0}")
    classify | print2

    # Step 3: Branch on whether there are failures
    # Note: In a real implementation, we'd use BRANCH_ON_VALUE to conditionally execute.
    # For this simplified version, we'll just spawn fixers regardless.

    # Step 4: Spawn parallel fixers (simplified to 3 fixers for common error types)
    fixer1 = g.spawn("fixer_import", profile=claude, cwd=cwd)
    fixer2 = g.spawn("fixer_mlir", profile=claude, cwd=cwd)
    fixer3 = g.spawn("fixer_compile", profile=claude, cwd=cwd)

    # Build fix prompts for each cluster type
    fix_import_task = g.ask(
        "build_fix_import_task",
        template="""Fix import errors from the classification:

Classification: {0}

If there are import_error failures:
1. Read the task file from /tmp/autofix-tasks/import_error.md
2. For each affected file, fix the import statements:
   - Update module paths (e.g., apxm.graph → apxm.graph.module)
   - Add missing imports
   - Remove obsolete imports
3. Run validation to verify: python3 scripts/apxm-autofix.py --scope examples/python --verify-only

If there are no import_error failures, output "No import errors to fix".

Report what you fixed and verification results.
"""
    )
    classify | fix_import_task
    print2 >> fix_import_task

    fix_mlir_task = g.ask(
        "build_fix_mlir_task",
        template="""Fix MLIR parse errors from the classification:

Classification: {0}

If there are mlir_parse_error failures:
1. Read the task file from /tmp/autofix-tasks/mlir_parse_error.md
2. For each affected file, fix the AIR output:
   - Fix malformed attribute syntax
   - Fix missing required attributes
   - Fix type mismatches
3. Run validation to verify: python3 scripts/apxm-autofix.py --scope examples/python --verify-only

If there are no mlir_parse_error failures, output "No MLIR errors to fix".

Report what you fixed and verification results.
"""
    )
    classify | fix_mlir_task
    print2 >> fix_mlir_task

    fix_compile_task = g.ask(
        "build_fix_compile_task",
        template="""Fix compile errors from the classification:

Classification: {0}

If there are compile_error failures:
1. Read the task file from /tmp/autofix-tasks/compile_error.md
2. For each affected file, fix the code:
   - Add missing parameters
   - Fix attribute names
   - Fix method call signatures
3. Run validation to verify: python3 scripts/apxm-autofix.py --scope examples/python --verify-only

If there are no compile_error failures, output "No compile errors to fix".

Report what you fixed and verification results.
"""
    )
    classify | fix_compile_task
    print2 >> fix_compile_task

    # All fixers work in parallel
    fixer1.ask("{0}")
    fix_import_task | fixer1.get_last_node()

    fixer2.ask("{0}")
    fix_mlir_task | fixer2.get_last_node()

    fixer3.ask("{0}")
    fix_compile_task | fixer3.get_last_node()

    print3 = g.print("=== IMPORT FIXES ===\n{0}")
    fixer1.get_last_node() | print3

    print4 = g.print("=== MLIR FIXES ===\n{0}")
    fixer2.get_last_node() | print4

    print5 = g.print("=== COMPILE FIXES ===\n{0}")
    fixer3.get_last_node() | print5

    # Step 5: Wait for all fixers to complete
    wait = g.wait_all(
        "wait_all_fixers",
        fixer1.get_last_node(),
        fixer2.get_last_node(),
        fixer3.get_last_node()
    )
    print3 >> wait
    print4 >> wait
    print5 >> wait

    # Step 6: Run final verification
    verifier = g.spawn("verifier", profile=claude, cwd=cwd)

    verify_task = g.text(value="""Run final verification of all fixes:

Execute:
  python3 scripts/apxm-autofix.py --scope examples/python --verify-only

Report the results (pass/fail counts, any remaining issues).
"""
    )

    verifier.ask("{0}")
    verify_task | verifier.get_last_node()
    wait >> verifier.get_last_node()

    print6 = g.print("=== VERIFICATION OUTPUT ===\n{0}")
    verifier.get_last_node() | print6

    # Step 7: Generate final report
    report = g.think(
        "generate_report",
        template="""Generate autofix report:

Initial validation: {0}
Classification: {1}
Import fixes: {2}
MLIR fixes: {3}
Compile fixes: {4}
Final verification: {5}

Report:
- Scope: examples/python
- Initial failures: <count by type>
- Fixes applied:
  * Import errors: <count fixed>
  * MLIR errors: <count fixed>
  * Compile errors: <count fixed>
- Final verification: <pass/fail>
- Remaining issues: <count> (if any)
- Status: <success/partial/failed>

If status is partial or failed, list remaining issues and suggested next steps.
"""
    )
    validator.get_last_node() | report
    classify | report
    fixer1.get_last_node() | report
    fixer2.get_last_node() | report
    fixer3.get_last_node() | report
    verifier.get_last_node() | report
    print6 >> report

    print7 = g.print("=== AUTOFIX REPORT ===\n{0}")
    report | print7

    g.done(print7)


if __name__ == "__main__":
    print(autofix_workflow._graph.to_air())
