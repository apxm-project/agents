#!/usr/bin/env python3
"""apxm_council.py - Multi-model council

Three parallel ASK ops -> synthesis via THINK -> final verdict

Usage: python3 -m examples.python.multi-agent.apxm_council
"""

from apxm.graph import compile, GraphRecorder


@compile()
def council_agent(g: GraphRecorder):
    """Multi-expert council with parallel analysis and synthesis."""
    question = g.ask(
        "question",
        template="What topic should we analyze? Provide a clear question about hardware or AI inference."
    )

    # Three parallel expert opinions
    expert1 = g.ask("expert1", template="You are a hardware architect. Answer concisely: {0}")
    question | expert1

    expert2 = g.ask("expert2", template="You are a performance engineer. Answer concisely: {0}")
    question | expert2

    expert3 = g.ask("expert3", template="You are a TCO analyst. Answer concisely: {0}")
    question | expert3

    # Synthesize all expert opinions
    synthesis = g.think(
        "synthesis",
        template="Three experts reviewed the question.\n\n"
        "Hardware architect: {0}\nPerformance engineer: {1}\nTCO analyst: {2}\n\n"
        "Synthesize a definitive, balanced answer:"
    )
    expert1 | synthesis
    expert2 | synthesis
    expert3 | synthesis

    # Print and return
    output = g.print_("output", message="{0}")
    synthesis | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(council_agent._graph.to_air())
