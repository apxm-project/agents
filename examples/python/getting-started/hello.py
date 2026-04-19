#!/usr/bin/env python3
"""hello.py - Minimal agent that asks a question

Usage: python3 -m examples.python.basics.hello
"""

from apxm import compile, GraphRecorder, Anthropic


@compile()
def hello_world(g: GraphRecorder):
    """Simple greeting workflow."""
    greeting = g.ask(
        name="greeting",
        prompt="Generate a friendly greeting for someone learning about AI agents",
        # model=Anthropic.CLAUDE_SONNET_4_6,  # optional: override default
    )
    g.done(greeting)



if __name__ == "__main__":
    import apxm

    result = apxm.run(hello_world())
    print(result.content)
