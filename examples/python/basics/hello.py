#!/usr/bin/env python3
"""hello.py - Minimal agent that asks a question

Usage: python3 -m examples.python.basics.hello
"""

from apxm.graph import compile, GraphRecorder


@compile()
def hello_world(g: GraphRecorder):
    """Simple greeting workflow."""
    greeting = g.ask(
        "greeting",
        "Generate a friendly greeting for someone learning about AI agents"
    )
    g.return_("output", source=greeting)
    


if __name__ == "__main__":
    import json
    print(hello_world._graph.to_air())
