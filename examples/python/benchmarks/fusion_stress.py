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
  dekk apxm execute fusion_stress.apxm -O0  # No fusion (20 LLM calls)
  dekk apxm execute fusion_stress.apxm -O2  # With fusion (~10 LLM calls)
"""

from apxm import compile, GraphRecorder


@compile()
def fusion_stress(g: GraphRecorder):
    """Sequential chain of 10 ask→think pairs to stress test fusion optimization."""

    # Start with initial question
    current = g.ask(
        "initial_ask",
        "What is the capital of France? Answer in one word."
    )

    # Chain of 10 ask→think pairs
    # Each pair should be fusible into a single LLM call
    for i in range(10):
        # ASK: Simple factual question
        ask_node = g.ask(
            f"ask_{i}",
            f"{{0}} Based on this, what is a famous landmark in that city? "
            f"Answer in 3-4 words (iteration {i})."
        )
        current | ask_node

        # THINK: Elaborate on the previous answer
        think_node = g.think(
            f"think_{i}",
            f"{{0}} Elaborate on why this landmark is historically significant. "
            f"Provide 2-3 sentences (iteration {i})."
        )
        ask_node | think_node

        current = think_node

    # Final output
    output = g.print(
        "=== FUSION STRESS TEST RESULT ===\n\n"
        "Final elaboration:\n{0}\n\n"
        "This workflow executed 10 ask→think pairs.\n"
        "O0: 20 separate LLM calls\n"
        "O2 with FuseAskOps: ~10 fused LLM calls"
    )
    current | output

    g.done(output)


if __name__ == "__main__":
    # Output the graph as JSON
    print(fusion_stress._graph.to_json())
