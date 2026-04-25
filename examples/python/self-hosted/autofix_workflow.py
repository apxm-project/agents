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
    dekk apxm execute examples/python/self-hosted/autofix_workflow.py "examples/python"
"""

from apxm import GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude, codex


@compile()
def autofix_workflow(g: GraphRecorder):
    """Autofix loop as an APXM workflow.

    Parameters:
        scope (str): Scope to validate (e.g., "examples/python")
    """
    g.param("scope", "str")

    cwd = agent_cwd()

    # Spawn agents
    validator = g.spawn("validator", profile=claude, cwd=cwd)

    # Step 1: Run validation
    validate_result = validator.ask(prompt="""Run the APXM autofix validation for scope: examples/python

Execute:
  python3 scripts/apxm-autofix.py --scope examples/python --report-only

Report the full output (it will show validation results, task files created in /tmp/autofix-tasks/, etc.)
""")

    print1 = g.print(message="=== VALIDATION OUTPUT ===\n{validate_result}")

    # Step 2: Classify failures into clusters
    classify = g.think(
        name="classify_failures",
        prompt="""Analyze the autofix validation output and classify failures:

Validation output from validator:
{validate_result}

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
    g.add_edge(print1, classify, dependency="Control")

    print2 = g.print(message="=== FAILURE CLASSIFICATION ===\n{classify}")

    # Step 3: Branch on whether there are failures
    # Note: In a real implementation, we'd use BRANCH_ON_VALUE to conditionally execute.
    # For this simplified version, we'll just spawn fixers regardless.

    # Step 4: Spawn parallel fixers (simplified to 3 fixers for common error types)
    fixer1 = g.spawn("fixer_import", profile=claude, cwd=cwd)
    fixer2 = g.spawn("fixer_mlir", profile=claude, cwd=cwd)
    fixer3 = g.spawn("fixer_compile", profile=claude, cwd=cwd)

    # Build fix prompts for each cluster type
    fix_import_task = g.ask(
        name="build_fix_import_task",
        prompt="""Fix import errors from the classification:

Classification: {classify}

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
    g.add_edge(print2, fix_import_task, dependency="Control")

    fix_mlir_task = g.ask(
        name="build_fix_mlir_task",
        prompt="""Fix MLIR parse errors from the classification:

Classification: {classify}

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
    g.add_edge(print2, fix_mlir_task, dependency="Control")

    fix_compile_task = g.ask(
        name="build_fix_compile_task",
        prompt="""Fix compile errors from the classification:

Classification: {classify}

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
    g.add_edge(print2, fix_compile_task, dependency="Control")

    # All fixers work in parallel
    import_fixes = fixer1.ask("{fix_import_task}")

    mlir_fixes = fixer2.ask("{fix_mlir_task}")

    compile_fixes = fixer3.ask("{fix_compile_task}")

    print3 = g.print(message="=== IMPORT FIXES ===\n{import_fixes}")
    print4 = g.print(message="=== MLIR FIXES ===\n{mlir_fixes}")
    print5 = g.print(message="=== COMPILE FIXES ===\n{compile_fixes}")

    # Step 5: Wait for all fixers to complete
    wait = g.wait_all(
        "wait_all_fixers",
        import_fixes,
        mlir_fixes,
        compile_fixes
    )
    g.add_edge(print3, wait, dependency="Control")
    g.add_edge(print4, wait, dependency="Control")
    g.add_edge(print5, wait, dependency="Control")

    # Step 6: Run final verification
    verifier = g.spawn("verifier", profile=claude, cwd=cwd)

    verify_result = verifier.ask(prompt="""Run final verification of all fixes:

Execute:
  python3 scripts/apxm-autofix.py --scope examples/python --verify-only

Report the results (pass/fail counts, any remaining issues).
""")
    g.add_edge(wait, verify_result, dependency="Control")

    print6 = g.print(message="=== VERIFICATION OUTPUT ===\n{verify_result}")

    # Step 7: Generate final report
    report = g.think(
        name="generate_report",
        prompt="""Generate autofix report:

Initial validation: {validate_result}
Classification: {classify}
Import fixes: {import_fixes}
MLIR fixes: {mlir_fixes}
Compile fixes: {compile_fixes}
Final verification: {verify_result}

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
    g.add_edge(print6, report, dependency="Control")

    print7 = g.print(message="=== AUTOFIX REPORT ===\n{report}")

    g.done(print7)


if __name__ == "__main__":
    import apxm

    result = apxm.run(autofix_workflow("crates/runtime"))
    print(result.content)
