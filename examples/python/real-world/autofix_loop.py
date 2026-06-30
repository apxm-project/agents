#!/usr/bin/env python3
"""Autofix Loop - APXM workflow for automated issue fixing.

This is the "ultrathink" version of autofix — a workflow that coordinates
multiple agents to analyze, fix, and verify issues in the APXM codebase.

The workflow:
1. Analyzes validation failures and creates bug clusters
2. Spawns parallel implementer agents per cluster
3. Reviews and verifies all fixes
4. Iterates if needed

Usage:
    dekk agents execute examples/python/real-world/autofix_loop.py
    dekk agents execute examples/python/real-world/autofix_loop.py --emit-session
"""

from apxm import DependencyType, GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude, codex


@compile()
def autofix_loop(g: GraphRecorder):
    """Multi-agent workflow for automated issue fixing.

    Simplified version using ASK and COMMUNICATE operations.
    Full version with bash/read capabilities would require capability registration.
    """
    cwd = agent_cwd()

    # Spawn agents
    architect = g.spawn("architect", profile=claude, cwd=cwd)
    implementer = g.spawn("implementer", profile=codex, cwd=cwd)
    reviewer = g.spawn("reviewer", profile=claude, cwd=cwd)

    # Step 1: Architect analyzes the autofix report
    strategy = architect.ask(
        "Analyze APXM validation failures and create a fix strategy.\n\n"
        "Run the autofix validation:\n"
        "  python3 scripts/apxm-autofix.py\n\n"
        "Review the generated task files in /tmp/autofix-tasks/.\n\n"
        "Determine:\n"
        "1. Which clusters are related and can be fixed together\n"
        "2. Priority order (e.g., fix import_error before mlir_parse_error)\n"
        "3. Estimated complexity for each cluster\n\n"
        "Output a JSON strategy with priority_order and cluster_groups."
    )

    print1 = g.print(message="=== STRATEGY ===\n{strategy}")

    # Step 2: Implementer works on highest priority cluster
    implement_task = g.ask(
        name="build_implement_task",
        prompt="Based on this strategy, work on the highest priority cluster.\n\n"
        "Read the task file from /tmp/autofix-tasks/ for that cluster.\n"
        "Follow the fix instructions carefully.\n"
        "Make the necessary changes to the codebase.\n"
        "Run verification: python3 scripts/apxm-autofix.py --verify-only\n\n"
        "Report what you changed and verification results.\n\n"
        "Strategy:\n{strategy}"
    )
    g.add_edge(print1, implement_task, dependency=DependencyType.CONTROL)

    implementation = implementer.ask("{implement_task}")

    print2 = g.print(message="=== IMPLEMENTATION ===\n{implementation}")

    # Step 3: Reviewer validates the fixes
    review_task = g.ask(
        name="build_review_task",
        prompt="Review the implementation and verify quality.\n\n"
        "Check:\n"
        "1. Did all validations pass?\n"
        "2. Are the fixes consistent with APXM coding standards?\n"
        "3. Were any issues missed?\n\n"
        "Run policy checks:\n"
        "  python3 scripts/apxm-policy-check.py\n\n"
        'If any issues remain, output: {{"status": "iterate", "issues": [...]}}\n'
        'If everything passes, output: {{"status": "success"}}\n\n'
        "Implementation:\n{implementation}"
    )
    g.add_edge(print2, review_task, dependency=DependencyType.CONTROL)

    review = reviewer.ask("{review_task}")

    print3 = g.print(message="=== REVIEW ===\n{review}")

    # Final report
    final = g.merge("final_report", strategy, implementation, review)
    g.add_edge(print3, final, dependency=DependencyType.CONTROL)

    g.done(final)


if __name__ == "__main__":
    import apxm

    result = apxm.run(autofix_loop())
    print(result.content)
