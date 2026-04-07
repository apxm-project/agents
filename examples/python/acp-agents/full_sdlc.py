#!/usr/bin/env python3
"""full_sdlc.py - Full SDLC: architect designs, coder implements, reviewer reviews

Usage: python3 -m examples.python.acp-agents.full_sdlc
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def full_sdlc(g: GraphRecorder):
    """Simple SDLC workflow with design, implementation, and review stages."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn architect
    architect = g.spawn("architect", profile="claude", cwd=cwd)

    # Design phase
    architect.ask("Design a caching layer for the API. Provide the implementation plan.")

    # Spawn coder
    coder = g.spawn("coder", profile="claude", cwd=cwd)

    # Implementation phase - coder receives architect's design
    coder_comm = g.communicate("communicate_to_coder", target_agent="coder", message="{0}")
    architect.get_last_node() | coder_comm

    # Review phase - architect reviews coder's implementation
    architect.ask("Review this implementation: {0}")
    coder_comm | architect.get_last_node()

    # Print and return final review
    output = g.print_("output", message="{0}")
    architect.get_last_node() | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(full_sdlc._graph.to_air())
