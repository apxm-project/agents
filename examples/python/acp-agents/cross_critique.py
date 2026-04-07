#!/usr/bin/env python3
"""cross_critique.py - Cross-critique: two agents propose, then critique each other

Usage: python3 -m examples.python.acp-agents.cross_critique
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude, codex
import os


@compile()
def cross_critique(g: GraphRecorder):
    """Two agents propose designs and critique each other."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn two agents
    agent_a = g.spawn("agent_a", profile=claude, cwd=cwd)
    agent_b = g.spawn("agent_b", profile=codex, cwd=cwd)

    # Both agents propose designs (parallel)
    design_prompt = "Propose a design for a REST API for a todo application."
    agent_a.ask(design_prompt)
    agent_b.ask(design_prompt)

    # Cross-critique: A reviews B's proposal, B reviews A's proposal
    agent_a.ask("Review this alternative design: {0}")
    agent_b.ask("Review this alternative design: {0}")

    # Get critique nodes
    agent_b_last = agent_b.get_last_node()
    agent_a_last = agent_a.get_last_node()

    # Get proposal nodes (second-to-last before critique)
    # We need to manually wire this since we're doing cross-critique
    # Let's use communicate nodes for clarity
    a_proposal_to_b = g.communicate(
        "a_proposal_to_b",
        target_agent="agent_b",
        message="{0}"
    )
    agent_a.get_spawn_node() | a_proposal_to_b

    b_proposal_to_a = g.communicate(
        "b_proposal_to_a",
        target_agent="agent_a",
        message="{0}"
    )
    agent_b.get_spawn_node() | b_proposal_to_a

    # Merge critiques
    merge_critiques = g.ask(
        "merge_critiques",
        template="Synthesize these two critiques:\n\nA's critique of B:\n{agent_a_last}\n\nB's critique of A:\n{agent_b_last}"
    )

    # Print and return
    output = g.print("{merge_critiques}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(cross_critique._graph.to_air())
