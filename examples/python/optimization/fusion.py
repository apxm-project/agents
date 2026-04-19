#!/usr/bin/env python3
"""fusion.py -- Demonstrate ASK/THINK fusion optimization

At O0, each ASK and THINK is a separate LLM call (6 total).
At O2, the FuseAskOps pass merges adjacent ASK->THINK pairs into
single calls, reducing to 3 LLM round-trips.

Usage: dekk apxm execute examples/python/optimization/fusion.py
"""

from apxm import compile, GraphRecorder


@compile()
def fusion_demo(g: GraphRecorder):
    """3 sequential ASK->THINK pairs. O0: 6 LLM calls. O2: 3 (fused)."""

    # Pair 1: question + elaboration
    q1 = g.ask(name="q1", prompt="What is the capital of France?")
    e1 = g.think(name="e1", prompt="Elaborate on this answer: {q1}")

    # Pair 2: follow-up + elaboration
    q2 = g.ask(name="q2", prompt="What is a famous landmark there? Context: {e1}")
    e2 = g.think(name="e2", prompt="Explain why this landmark matters: {q2}")

    # Pair 3: synthesis + elaboration
    q3 = g.ask(name="q3", prompt="Summarize the cultural significance: {e2}")
    e3 = g.think(name="e3", prompt="Final reflection on this topic: {q3}")

    output = g.print(message="Result:\n{e3}")
    g.done(output)


if __name__ == "__main__":
    import apxm

    result = apxm.run(fusion_demo())
    print(result.content)
