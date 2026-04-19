#!/usr/bin/env python3
"""priority_scheduling.py - Benchmark for Priority-based Scheduling

Tests: Priority wiring + scheduler optimization
Measures: Critical path completion time under resource contention

Graph structure: Critical path (ask→think→reason→final) vs 5 parallel background tasks
- O0: All 9 nodes get equal scheduling priority (random order)
- O2 with priority: Critical path nodes scheduled first (faster user-visible response)

Metrics:
- Critical path completion time (time until 'final' node completes)
- Background task completion time (time until all 5 background nodes complete)
- Total execution time
- Speedup of critical path (should improve under contention)

Usage:
  dekk apxm execute priority_scheduling.air -O0  # No priority (equal scheduling)
  dekk apxm execute priority_scheduling.air -O2  # With priority (critical path first)
"""

from apxm import compile, GraphRecorder


@compile()
def priority_scheduling(g: GraphRecorder):
    """Critical path vs background work to test priority scheduling."""

    # ============================================================
    # CRITICAL PATH: User-visible workflow (high priority)
    # ============================================================
    # This is the main user-facing flow that should complete ASAP

    critical_ask = g.ask(
        name="critical_ask",
        prompt="Design a REST API for a simple TODO list application. "
        "What are the core endpoints? Provide 3-4 sentences."
    )

    critical_think = g.think(
        name="critical_think",
        prompt="{critical_ask}\n\n"
        "Based on this API design, what are the key data models needed? "
        "Describe the schema in 3-4 sentences."
    )

    critical_reason = g.reason(
        name="critical_reason",
        prompt="{critical_think}\n\n"
        "Given these data models, what are the main scalability challenges? "
        "Propose a solution in 3-4 sentences."
    )

    critical_final = g.think(
        name="critical_final",
        prompt="{critical_reason}\n\n"
        "Summarize the complete design (API + data model + scalability) "
        "in 4-5 sentences."
    )

    # ============================================================
    # BACKGROUND WORK: Speculative/analytics (low priority)
    # ============================================================
    # These are nice-to-have tasks that can wait (logging, analytics, etc.)

    # Background task 1: Analyze API naming conventions
    bg_task_1 = g.think(
        name="bg_naming_analysis",
        prompt="Analyze common REST API naming conventions for TODO applications. "
        "What are the best practices? Provide 4-5 examples."
    )

    # Background task 2: Research authentication patterns
    bg_task_2 = g.think(
        name="bg_auth_research",
        prompt="Research authentication patterns for TODO applications. "
        "Compare JWT vs session-based auth. Which is better and why? "
        "Provide 4-5 sentences."
    )

    # Background task 3: Analyze database choices
    bg_task_3 = g.think(
        name="bg_database_analysis",
        prompt="Compare PostgreSQL vs MongoDB for a TODO list application. "
        "What are the tradeoffs? Which would you choose and why? "
        "Provide 4-5 sentences."
    )

    # Background task 4: Research caching strategies
    bg_task_4 = g.think(
        name="bg_caching_research",
        prompt="Research caching strategies for TODO list APIs. "
        "Where should caching be applied? What are the tradeoffs? "
        "Provide 4-5 sentences."
    )

    # Background task 5: Analyze monitoring approaches
    bg_task_5 = g.think(
        name="bg_monitoring_analysis",
        prompt="Analyze monitoring approaches for TODO list applications. "
        "What metrics should be tracked? What tools are commonly used? "
        "Provide 4-5 sentences."
    )

    # Merge background tasks (optional, not on critical path)
    bg_report = g.merge(
        "bg_report",
        bg_task_1,
        bg_task_2,
        bg_task_3,
        bg_task_4,
        bg_task_5
    )

    # ============================================================
    # FINAL OUTPUT
    # ============================================================

    # The critical path result is what the user sees immediately.
    user_output = g.print(
        message="=== USER-VISIBLE OUTPUT (Critical Path) ===\n\n"
        "{critical_final}\n\n"
        "This is the CRITICAL PATH result that the user sees.\n"
        "With priority scheduling, this should complete FIRST,\n"
        "even if background tasks are still running."
    )

    # Background report (less urgent, can complete later). Each background
    # task NodeRef is auto-wired via its variable name placeholder.
    bg_output = g.print(
        message="=== BACKGROUND ANALYTICS (Low Priority) ===\n\n"
        "Naming conventions: {bg_task_1}\n\n"
        "Authentication: {bg_task_2}\n\n"
        "Database choice: {bg_task_3}\n\n"
        "Caching strategy: {bg_task_4}\n\n"
        "Monitoring: {bg_task_5}\n\n"
        "This background work completed after the critical path."
    )
    # Control edge keeps the merge as a synchronization barrier without
    # adding a Data input that would clash with input_names.
    g.add_edge(bg_report, bg_output, dependency="Control")

    # Wait for both paths to complete
    all_done = g.wait_all("all_done", user_output, bg_output)

    final_output = g.print(
        message="=== PRIORITY SCHEDULING STRESS TEST ===\n\n"
        "Critical path (ask\u2192think\u2192reason\u2192final): COMPLETED\n"
        "Background tasks (5 parallel speculative nodes): COMPLETED\n\n"
        "O0: All 9 nodes scheduled with equal priority (random order)\n"
        "O2: Critical path prioritized \u2192 faster user-visible response\n\n"
        "Metric: Critical path completion time should be LOWER in O2 under contention."
    )
    g.add_edge(all_done, final_output, dependency="Control")

    g.done(final_output)


if __name__ == "__main__":
    # Output the graph as JSON
    print(priority_scheduling._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(priority_scheduling())
    # print(result.content)
