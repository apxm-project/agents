#!/usr/bin/env python3
"""full_sdlc.py - Full SDLC: architect designs, coder implements, reviewer reviews

Usage: python3 -m examples.python.acp-agents.full_sdlc
"""

from apxm import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def full_sdlc(g: GraphRecorder):
    """Simple SDLC workflow with design, implementation, and review stages."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn architect
    architect = g.spawn("architect", profile=claude, cwd=cwd)

    # Design phase
    architect.ask("Design a caching layer for the API. Provide the implementation plan.")

    # Spawn coder
    coder = g.spawn("coder", profile=claude, cwd=cwd)

    # Get architect's design
    architect_design = architect.get_last_node()

    # Implementation phase - coder receives architect's design
    coder_comm = g.communicate("communicate_to_coder", target_agent="coder", message="{architect_design}")

    # Review phase - architect reviews coder's implementation
    architect.ask("Review this implementation: {coder_comm}")

    # Get final review
    final_review = architect.get_last_node()

    # Print and return final review
    output = g.print("{final_review}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(full_sdlc._graph.to_air())
