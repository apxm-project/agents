#!/usr/bin/env python3
"""multi_turn_communicate.py - Multi-turn: spawn agent, analyze, fix, verify

Usage: python3 -m examples.python.acp-agents.multi_turn_communicate
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def multi_turn_communicate(g: GraphRecorder):
    """Multi-turn conversation with a coder agent."""
    # Spawn coder agent
    coder = g.spawn(
        "coder",
        profile="claude",
        cwd=os.environ.get("APXM_HOME", os.getcwd())
    )

    # Multi-turn conversation using method chaining
    coder.ask("Analyze the codebase for potential bugs.")
    coder.ask("Fix the most critical bug you found.")
    coder.ask("Verify the fix by running the test suite.")

    # Print final result
    output = g.print_("output", message="{0}")
    coder.get_last_node() | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(multi_turn_communicate._graph.to_air())
