#!/usr/bin/env python3
"""apxm_council.py - Multi-model council

Three parallel ASK ops -> synthesis via THINK -> final verdict

Usage: dekk apxm execute examples/python/parallelism/expert_council.py
"""

from apxm import compile, GraphRecorder


@compile()
def council_agent(g: GraphRecorder):
    """Multi-expert council with parallel analysis and synthesis."""
    question = g.ask(
        name="question",
        prompt="What topic should we analyze? Provide a clear question about hardware or AI inference."
    )

    # Three parallel expert opinions
    expert1 = g.ask(name="expert1", prompt="You are a hardware architect. Answer concisely: {question}")

    expert2 = g.ask(name="expert2", prompt="You are a performance engineer. Answer concisely: {question}")

    expert3 = g.ask(name="expert3", prompt="You are a TCO analyst. Answer concisely: {question}")

    # Synthesize all expert opinions
    synthesis = g.think(
        name="synthesis",
        prompt="Three experts reviewed the question.\n\n"
        "Hardware architect: {expert1}\nPerformance engineer: {expert2}\nTCO analyst: {expert3}\n\n"
        "Synthesize a definitive, balanced answer:"
    )

    # Print and return
    output = g.print(message="{synthesis}")

    g.done(output)


if __name__ == "__main__":
    import apxm

    result = apxm.run(council_agent())
    print(result.content)
