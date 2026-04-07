#!/usr/bin/env python3
"""Autofix Loop - APXM workflow for automated issue fixing.

This is the "ultrathink" version of autofix — a workflow graph that orchestrates
multiple agents to analyze, fix, and verify issues in the APXM codebase.

The workflow:
1. Analyzes validation failures and creates bug clusters
2. Spawns parallel implementer agents per cluster
3. Reviews and verifies all fixes
4. Iterates if needed

Usage:
    PYTHONPATH=crates/apxm-frontend/python python3 examples/python/workflows/autofix_loop.py
    dekk apxm compile examples/python/workflows/autofix_loop.air
    dekk apxm execute examples/python/workflows/autofix_loop.air --emit-session
"""

from apxm.graph import compile, GraphRecorder, AgentConfig
from apxm._generated.agents import claude, codex


@compile()
def autofix_loop(g: GraphRecorder):
    """Multi-agent workflow for automated issue fixing.

    Simplified version using ASK and COMMUNICATE operations.
    Full version with bash/read capabilities would require capability registration.
    """
    import os
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents
    architect = g.spawn("architect", profile=claude, cwd=cwd)
    implementer = g.spawn("implementer", profile=codex, cwd=cwd)
    reviewer = g.spawn("reviewer", profile=claude, cwd=cwd)

    # Step 1: Architect analyzes the autofix report
    analyze_task = g.text(
        value="""Analyze APXM validation failures and create a fix strategy.

Run the autofix validation:
  python3 scripts/apxm-autofix.py

Review the generated task files in /tmp/autofix-tasks/.

Determine:
1. Which clusters are related and can be fixed together
2. Priority order (e.g., fix import_error before mlir_parse_error)
3. Estimated complexity for each cluster

Output a JSON strategy with priority_order and cluster_groups.
""")

    architect.ask("{analyze_task}")
    strategy = architect.get_last_node()

    print1 = g.print("=== STRATEGY ===\n{strategy}")

    # Step 2: Implementer works on highest priority cluster
    implement_task = g.ask(
        "build_implement_task",
        """Based on this strategy, work on the highest priority cluster.

Read the task file from /tmp/autofix-tasks/ for that cluster.
Follow the fix instructions carefully.
Make the necessary changes to the codebase.
Run verification: python3 scripts/apxm-autofix.py --verify-only

Report what you changed and verification results.

Strategy:
{strategy}
"""
    )
    print1 >> implement_task

    implementer.ask("{implement_task}")
    implementation = implementer.get_last_node()

    print2 = g.print("=== IMPLEMENTATION ===\n{implementation}")

    # Step 3: Reviewer validates the fixes
    review_task = g.ask(
        "build_review_task",
        """Review the implementation and verify quality.

Check:
1. Did all validations pass?
2. Are the fixes consistent with APXM coding standards?
3. Were any issues missed?

Run policy checks:
  python3 scripts/apxm-policy-check.py

If any issues remain, output: {{"status": "iterate", "issues": [...]}}
If everything passes, output: {{"status": "success"}}

Implementation:
{implementation}
"""
    )
    print2 >> review_task

    reviewer.ask("{review_task}")
    review = reviewer.get_last_node()

    print3 = g.print("=== REVIEW ===\n{review}")

    # Final report
    final = g.merge("final_report", strategy, implementation, review)
    print3 >> final

    g.done(final)


if __name__ == "__main__":
    import sys
    from pathlib import Path

    # Emit the .air file
    air_content = autofix_loop._graph.to_air()

    # Write to file
    output_path = Path(__file__).with_suffix(".air")
    output_path.write_text(air_content)

    print(f"Generated: {output_path}")
    print()
    print("To compile and execute:")
    print(f"  dekk apxm compile {output_path}")
    print(f"  dekk apxm execute {output_path} --emit-session")
