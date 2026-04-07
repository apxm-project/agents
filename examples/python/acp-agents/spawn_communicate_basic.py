#!/usr/bin/env python3
"""spawn_communicate_basic.py - Basic: spawn a Claude agent and send one message

Usage: python3 -m examples.python.acp-agents.spawn_communicate_basic
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def spawn_communicate_basic(g: GraphRecorder):
    """Spawn Claude agent and send a review request."""
    # Spawn Claude agent with typed profile
    reviewer = g.spawn(
        "reviewer",
        profile=claude,
        cwd=os.environ.get("APXM_HOME", os.getcwd())
    )

    # Send message using AgentHandle sugar
    reviewer.ask("Review the current directory structure and suggest improvements.")

    # Get reviewer response
    review = reviewer.get_last_node()

    # Print and return the response (auto-named)
    output = g.print("{review}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(spawn_communicate_basic._graph.to_air())
