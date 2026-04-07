#!/usr/bin/env python3
"""apxm_council.py - Multi-model council

Three parallel ASK ops -> synthesis via THINK -> final verdict

Usage: python3 -m examples.python.multi-agent.apxm_council
"""

from apxm import compile, GraphRecorder


@compile()
def council_agent(g: GraphRecorder):
    """Multi-expert council with parallel analysis and synthesis."""
    question = g.ask(
        "question",
        "What topic should we analyze? Provide a clear question about hardware or AI inference."
    )

    # Three parallel expert opinions
    expert1 = g.ask("expert1", "You are a hardware architect. Answer concisely: {question}")

    expert2 = g.ask("expert2", "You are a performance engineer. Answer concisely: {question}")

    expert3 = g.ask("expert3", "You are a TCO analyst. Answer concisely: {question}")

    # Synthesize all expert opinions
    synthesis = g.think(
        "synthesis",
        "Three experts reviewed the question.\n\n"
        "Hardware architect: {expert1}\nPerformance engineer: {expert2}\nTCO analyst: {expert3}\n\n"
        "Synthesize a definitive, balanced answer:"
    )

    # Print and return
    output = g.print("{synthesis}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(council_agent._graph.to_air())
