#!/usr/bin/env python3
"""audit.py - APXM Project Audit Workflow

Self-hosted workflow that audits the APXM project for issues.

Graph structure:
- spawn architect (claude)
- exc: run build check (dekk apxm build 2>&1)
- exc: run test check (dekk apxm test-all --quiet 2>&1)
- exc: run autofix check (python3 scripts/apxm-autofix.py --report-only)
- exc: run policy check (python3 scripts/apxm-policy-check.py)
- exc: count TODOs/FIXMEs/stubs
- exc: check for missing ops in artifact emitter
- think: synthesize audit report
- think: generate actionable recommendations

Usage:
    dekk apxm execute examples/python/self-hosted/audit.py "full"
"""

from apxm import DependencyType, GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude


@compile()
def audit(g: GraphRecorder):
    """Audit the APXM project for issues.

    Parameters:
        scope (str): Audit scope - "full" (default), "build", "test", "examples", "policy"
    """
    g.param("scope", "str")

    cwd = agent_cwd()

    # Spawn architect agent
    architect = g.spawn("architect", profile=claude, cwd=cwd)

    # Step 1: Run build check
    build_result = architect.ask(prompt="""Run APXM build and capture warnings/errors:

Execute:
  dekk apxm build 2>&1

Parse the output and report:
- Build status (success/failure)
- Number of warnings by type (dead_code, unused_imports, etc.)
- Number of errors (if any)
- Critical issues (if any)

Output JSON:
{{
  "status": "clean" | "warnings" | "errors",
  "warnings": {{
    "dead_code": N,
    "unused_imports": N,
    "other": N
  }},
  "errors": [],
  "total_warnings": N,
  "total_errors": N
}}
""")

    print1 = g.print(message="=== BUILD STATUS ===\n{build_result}")

    # Step 2: Run test check
    test_result = architect.ask(prompt="""Run APXM test suite and report results:

Execute:
  dekk apxm test-all --quiet 2>&1

Parse the output and report:
- Test status (all pass/some failures)
- Number of tests run
- Number of tests passed
- Number of tests failed
- Failed test names (if any)

Output JSON:
{{
  "status": "pass" | "fail",
  "total": N,
  "passed": N,
  "failed": N,
  "failed_tests": ["test1", "test2", ...]
}}
""")
    g.add_edge(print1, test_result, dependency=DependencyType.CONTROL)

    print2 = g.print(message="=== TEST STATUS ===\n{test_result}")

    # Step 3: Run autofix check
    autofix_result = architect.ask(prompt="""Run APXM autofix validation and report:

Execute:
  python3 scripts/apxm-autofix.py --report-only

Parse the output and report:
- Validation status (all pass/some failures)
- Number of examples validated
- Number of examples passed
- Number of examples failed
- Failure types (import_error, mlir_parse_error, compile_error, etc.)

Output JSON:
{{
  "status": "pass" | "fail",
  "total": N,
  "passed": N,
  "failed": N,
  "failures_by_type": {{
    "import_error": N,
    "mlir_parse_error": N,
    "compile_error": N
  }}
}}
""")
    g.add_edge(print2, autofix_result, dependency=DependencyType.CONTROL)

    print3 = g.print(message="=== AUTOFIX STATUS ===\n{autofix_result}")

    # Step 4: Run policy check
    policy_result = architect.ask(prompt="""Run APXM policy checks and report violations:

Execute:
  python3 scripts/apxm-policy-check.py

Parse the output and report:
- Policy status (clean/violations)
- Number of violations by category
- Critical violations (if any)

Output JSON:
{{
  "status": "clean" | "violations",
  "violations": {{
    "category1": N,
    "category2": N
  }},
  "total_violations": N,
  "critical": []
}}
""")
    g.add_edge(print3, policy_result, dependency=DependencyType.CONTROL)

    print4 = g.print(message="=== POLICY STATUS ===\n{policy_result}")

    # Step 5: Count TODOs/FIXMEs/stubs
    todo_result = architect.ask(prompt="""Count TODOs, FIXMEs, and stub handlers in runtime and compiler:

Execute:
  grep -r "TODO\\|FIXME" crates/apxm-runtime crates/apxm-compiler --include="*.rs" | wc -l
  grep -r "stub\\|unimplemented" crates/runtime/apxm-runtime/src/executor/handlers --include="*.rs" | wc -l

Report:
- Total TODOs/FIXMEs
- Total stub handlers
- High-priority items (if marked as such)

Output JSON:
{{
  "todos_fixmes": N,
  "stub_handlers": N,
  "high_priority": []
}}
""")
    g.add_edge(print4, todo_result, dependency=DependencyType.CONTROL)

    print5 = g.print(message="=== TODO/STUB STATUS ===\n{todo_result}")

    # Step 6: Check for missing ops in artifact emitter
    missing_ops_result = architect.ask(prompt="""Check for missing AIS operations in artifact emitter:

Compare:
1. AIS operations defined in crates/core/apxm-ais/src/operations/definitions.rs (AISOperationType enum)
2. Operations mapped in crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp (mapOperation function)

Report any operations defined in Rust but missing from the C++ emitter.

Execute:
  grep "^\\s*[A-Z][a-zA-Z]*," crates/core/apxm-ais/src/operations/definitions.rs | wc -l
  grep "Case<.*Op>" crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp | wc -l

Output JSON:
{{
  "total_ops": N,
  "mapped_ops": N,
  "missing_ops": ["Op1", "Op2", ...],
  "status": "complete" | "incomplete"
}}
""")
    g.add_edge(print5, missing_ops_result, dependency=DependencyType.CONTROL)

    print6 = g.print(message="=== MISSING OPS STATUS ===\n{missing_ops_result}")

    # Step 7: Synthesize audit report
    synthesize = g.think(
        name="synthesize_report",
        prompt="""Synthesize comprehensive audit report from all checks:

Build status: {build_result}
Test status: {test_result}
Autofix status: {autofix_result}
Policy status: {policy_result}
TODO/Stub status: {todo_result}
Missing ops status: {missing_ops_result}

Generate structured audit report:

# APXM Project Audit Report

## Summary
- Overall status: <CLEAN/WARNINGS/ISSUES/CRITICAL>
- Scope: <scope from param>
- Timestamp: <current date>

## Build Health
- Status: <clean/warnings/errors>
- Total warnings: N
- Total errors: N
- Details: <summary>

## Test Coverage
- Status: <pass/fail>
- Tests run: N
- Tests passed: N
- Tests failed: N
- Failed tests: <list if any>

## Example Validation
- Status: <pass/fail>
- Examples validated: N
- Examples passed: N
- Examples failed: N
- Failure types: <breakdown>

## Policy Compliance
- Status: <clean/violations>
- Total violations: N
- Violations by category: <breakdown>
- Critical violations: <list if any>

## Code Quality
- TODOs/FIXMEs: N
- Stub handlers: N
- High-priority items: <list if any>

## Completeness
- Total AIS ops: N
- Mapped ops: N
- Missing ops: <list if any>

## Overall Assessment
<2-3 sentence summary of project health>
"""
    )
    g.add_edge(print6, synthesize, dependency=DependencyType.CONTROL)

    print7 = g.print(message="=== AUDIT REPORT ===\n{synthesize}")

    # Step 8: Generate actionable recommendations
    recommend = g.think(
        name="generate_recommendations",
        prompt="""Generate actionable recommendations ranked by impact and effort:

Audit report: {synthesize}

For each issue category (build warnings, test failures, policy violations, etc.):
1. Rank by impact (CRITICAL/HIGH/MEDIUM/LOW)
2. Estimate effort (TRIVIAL/SMALL/MEDIUM/LARGE)
3. Suggest concrete next steps

Output recommendations sorted by (impact, effort):

# Actionable Recommendations

## Critical (do now)
- [ ] <item> - <why critical> - <effort estimate> - <concrete steps>

## High Priority (this week)
- [ ] <item> - <why important> - <effort estimate> - <concrete steps>

## Medium Priority (this month)
- [ ] <item> - <why needed> - <effort estimate> - <concrete steps>

## Low Priority (backlog)
- [ ] <item> - <why nice-to-have> - <effort estimate> - <concrete steps>

## Suggested Focus Areas
1. <area 1>: <rationale>
2. <area 2>: <rationale>
3. <area 3>: <rationale>
"""
    )
    g.add_edge(print7, recommend, dependency=DependencyType.CONTROL)

    print8 = g.print(message="=== RECOMMENDATIONS ===\n{recommend}")

    g.done(print8)


if __name__ == "__main__":
    import apxm

    result = apxm.run(audit("full"))
    print(result.content)
