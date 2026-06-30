#!/usr/bin/env python3
"""Simple APXM workflow example demonstrating basic API usage."""

from apxm import compile, GraphRecorder


@compile()
def simple_pipeline(g: GraphRecorder, question: str):
    """Build a simple ask->think->reason workflow."""
    ask = g.ask(name="initial_response", prompt="Answer this question: {question}")
    think = g.think(name="deeper_analysis", prompt="Think deeply about: {initial_response}")
    reason = g.reason(name="final_answer", prompt="Reason through the logic: {deeper_analysis}")
    g.done(source=reason)


if __name__ == "__main__":
    import apxm

    result = apxm.run(simple_pipeline("What is consciousness?"))
    print(result.content)
