#!/usr/bin/env python3
"""mixed_priority.py - Benchmark for priority-based scheduling

Tests: Priority mapping from critical path analysis to vLLM scheduling
Measures: Critical path latency with/without priority scheduling

Usage:
  dekk apxm execute mixed_priority.apxm -O0 --target cost      # No priority
  dekk apxm execute mixed_priority.apxm -O2 --target latency   # With priority scheduling
"""

from apxm import compile, GraphRecorder


@compile()
def mixed_priority(g: GraphRecorder):
    """Critical path + background speculative work."""

    # Critical path: user-facing query
    user_query = g.text(
        "user_query",
        value="What are the top 3 programming languages for web development in 2026?"
    )

    # CRITICAL PATH - should get highest priority
    quick_answer = g.ask(
        "quick_answer",
        "Provide a concise answer to this question:\n{user_query}\n\n"
        "Answer in 2-3 sentences, prioritize speed."
    )

    # CRITICAL PATH - refine the answer
    final_answer = g.think(
        "final_answer",
        "Given this quick answer:\n{quick_answer}\n\n"
        "Refine it to be more precise and add brief justification.\n"
        "Keep it under 5 sentences."
    )

    # Background task 1 - speculative deep analysis (lower priority)
    deep_analysis = g.reason(
        "deep_analysis",
        "Original query: {user_query}\n\n"
        "Provide a comprehensive analysis covering:\n"
        "- Language ecosystem maturity\n"
        "- Job market trends\n"
        "- Performance characteristics\n"
        "- Developer experience\n"
        "This is background analysis, not time-critical."
    )

    # Background task 2 - speculative comparison (lower priority)
    comparison = g.think(
        "comparison",
        "Original query: {user_query}\n\n"
        "Create a detailed comparison table of the top languages.\n"
        "Include: performance, learning curve, community size, ecosystem.\n"
        "This is background analysis, not time-critical."
    )

    # Background task 3 - speculative future trends (lower priority)
    future_trends = g.think(
        "future_trends",
        "Original query: {user_query}\n\n"
        "Predict language trends for the next 5 years.\n"
        "What new languages might emerge? What might decline?\n"
        "This is background analysis, not time-critical."
    )

    # Merge all results - critical answer comes first
    complete_response = g.merge(
        "complete_response",
        final_answer,
        deep_analysis,
        comparison,
        future_trends
    )

    output = g.print(
        "=== QUICK ANSWER (Critical Path) ===\n{final_answer}\n\n"
        "=== DEEP ANALYSIS (Background) ===\n{deep_analysis}\n\n"
        "=== COMPARISON (Background) ===\n{comparison}\n\n"
        "=== FUTURE TRENDS (Background) ===\n{future_trends}"
    )

    output >> complete_response
    g.done(complete_response)


if __name__ == "__main__":
    # Output the graph as JSON
    print(mixed_priority._graph.to_air())
