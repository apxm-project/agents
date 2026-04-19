#!/usr/bin/env python3
"""fusion_stress.py - Benchmark for ASK operation fusion

Tests: FuseAskOps optimization pass
Measures: LLM call count reduction via sequential pair fusion

Graph structure: 10 sequential pairs of ask("question {i}") → think("elaborate on {prev}")
- O0: 20 LLM calls (10 ask + 10 think, all separate)
- O2 with FuseAskOps: should reduce to ~10 LLM calls (pairs fused)

Metrics:
- Total LLM calls (should halve from O0 to O2)
- Total execution time (should improve with fewer round-trips)
- Average latency per operation

Usage:
  dekk apxm execute fusion_stress.air -O0  # No fusion (20 LLM calls)
  dekk apxm execute fusion_stress.air -O2  # With fusion (~10 LLM calls)
"""

from apxm import compile, GraphRecorder


@compile()
def fusion_stress(g: GraphRecorder):
    """Sequential chain of 10 ask→think pairs to stress test fusion optimization."""

    # Start with initial question
    current = g.ask(
        name="initial_ask",
        prompt="What is the capital of France? Answer in one word."
    )

    # Chain of 10 ask→think pairs
    # Each pair should be fusible into a single LLM call.
    # Auto-wire reads the caller's local variable scope, so referencing
    # `{prev}` in the template wires the edge from whatever NodeRef the
    # local `prev` currently holds.
    for i in range(10):
        prev = current  # bind local for template auto-wire
        ask_node = g.ask(
            name=f"ask_{i}",
            prompt=f"{{prev}} Based on this, what is a famous landmark in that city? "
            f"Answer in 3-4 words (iteration {i})."
        )

        # THINK: Elaborate on the previous ASK answer.
        prev = ask_node
        think_node = g.think(
            name=f"think_{i}",
            prompt=f"{{prev}} Elaborate on why this landmark is historically significant. "
            f"Provide 2-3 sentences (iteration {i})."
        )

        current = think_node

    # Final output (current is the last think node).
    final = current  # bind local for template auto-wire
    output = g.print(
        message="=== FUSION STRESS TEST RESULT ===\n\n"
        "Final elaboration:\n{final}\n\n"
        "This workflow executed 10 ask\u2192think pairs.\n"
        "O0: 20 separate LLM calls\n"
        "O2 with FuseAskOps: ~10 fused LLM calls"
    )

    g.done(output)


if __name__ == "__main__":
    # Output the graph as JSON
    print(fusion_stress._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(fusion_stress())
    # print(result.content)
