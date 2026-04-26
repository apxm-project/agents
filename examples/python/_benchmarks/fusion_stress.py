#!/usr/bin/env python3
"""fusion_stress.py - Legacy explicit benchmark source for ASK-chain mutation

Tests: legacy FuseAskOps experiment
Measures: compile-time ASK-chain mutation when the pass is explicitly requested

Graph structure: sequential ASK chain
- Default O-levels: no fusion
- Explicit pass-list with FuseAskOps: adjacent ASK nodes may be rewritten

Metrics:
- Compiled node count
- FuseAskOps diagnostics

Usage:
  dekk apxm compile fusion_stress.py \
    --pass-list normalize,build-prompt,fuse-ask-ops,canonicalizer \
    -o /tmp/fusion_stress.apxmobj
"""

from apxm import compile, GraphRecorder


@compile()
def fusion_stress(g: GraphRecorder):
    """Sequential ASK chain to stress test the legacy explicit pass."""

    # Start with initial question
    current = g.ask(
        name="initial_ask",
        prompt="What is the capital of France? Answer in one word."
    )

    # Chain of ASK nodes. Each producer has one consumer, which is the shape
    # FuseAskOps understands when the pass is explicitly requested.
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

        prev = ask_node
        elaborate_node = g.ask(
            name=f"elaborate_{i}",
            prompt=f"{{prev}} Elaborate on why this landmark is historically significant. "
            f"Provide 2-3 sentences (iteration {i})."
        )

        current = elaborate_node

    # Final output (current is the last ASK node).
    final = current  # bind local for template auto-wire
    output = g.print(
        message="=== ASK-CHAIN STRESS TEST RESULT ===\n\n"
        "Final elaboration:\n{final}\n\n"
        "This workflow is a legacy explicit FuseAskOps stress source."
    )

    g.done(output)


if __name__ == "__main__":
    # Output the graph as JSON
    print(fusion_stress._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(fusion_stress())
    # print(result.content)
