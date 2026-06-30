#!/usr/bin/env python3
"""hello.py - Minimal agent that asks a question

Usage: dekk agents execute examples/python/getting-started/hello.py
"""

from apxm import compile, GraphRecorder


@compile()
def hello_world(g: GraphRecorder):
    """Simple greeting workflow."""
    greeting = g.ask(
        name="greeting",
        prompt="Generate a friendly greeting for someone learning about AI agents",
    )
    g.done(greeting)



if __name__ == "__main__":
    import apxm

    result = apxm.run(hello_world())
    print(result.content)
