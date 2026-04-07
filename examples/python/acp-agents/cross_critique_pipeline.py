#!/usr/bin/env python3
"""cross_critique_pipeline.py - Diamond dataflow with cross-agent context passing

Claude and Codex propose next ACP features in parallel, then each critiques the other.
True diamond dataflow with cross-agent context passing.

Usage: python3 -m examples.python.acp-agents.cross_critique_pipeline
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude, codex
import os


@compile()
def cross_critique_pipeline(g: GraphRecorder):
    """Diamond pattern: parallel proposals, then cross-critiques."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents
    agent_a = g.spawn("agent_a", profile=claude, cwd=cwd)
    agent_b = g.spawn("agent_b", profile=codex, cwd=cwd)

    # Parallel proposals
    question = (
        "In the APXM project (this codebase), what is the single most impactful next feature "
        "to implement for the ACP multi-agent system? Propose your top idea with a one-paragraph "
        "justification. Be specific and concrete. Under 150 words."
    )
    agent_a.ask(question)
    agent_b.ask(question)

    # Get proposals for later use
    claude_proposal = agent_a.get_last_node()
    codex_proposal = agent_b.get_last_node()

    # Print parallel proposals
    print1 = g.print("=== PROPOSALS (parallel) ===\nClaude:\n{claude_proposal}\n\nCodex:\n{codex_proposal}")

    # Cross-critiques: A critiques B, B critiques A
    codex_critique_prompt = g.ask(
        "build_codex_critique",
        template="Critique the following feature proposal from Claude for the APXM project. "
        "Is it feasible? Is it truly the most impactful? What's missing? Under 100 words.\n\n"
        "Claude's proposal:\n{claude_proposal}"
    )
    print1 >> codex_critique_prompt

    claude_critique_prompt = g.ask(
        "build_claude_critique",
        template="Critique the following feature proposal from Codex for the APXM project. "
        "Is it feasible? Is it truly the most impactful? What's missing? Under 100 words.\n\n"
        "Codex's proposal:\n{codex_proposal}"
    )
    print1 >> claude_critique_prompt

    # Send cross-critiques
    codex_critiques = g.communicate(
        "codex_critiques_claude",
        target_agent="agent_b",
        message="{codex_critique_prompt}"
    )

    claude_critiques = g.communicate(
        "claude_critiques_codex",
        target_agent="agent_a",
        message="{claude_critique_prompt}"
    )

    # Print critiques
    print2 = g.print("=== CRITIQUES ===\nCodex critiques Claude:\n{codex_critiques}\n\nClaude critiques Codex:\n{claude_critiques}")

    g.done(print2)
    


if __name__ == "__main__":
    import json
    print(cross_critique_pipeline._graph.to_air())
