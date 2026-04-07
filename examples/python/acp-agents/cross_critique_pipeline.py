#!/usr/bin/env python3
"""cross_critique_pipeline.py - Diamond dataflow with cross-agent context passing

Claude and Codex propose next ACP features in parallel, then each critiques the other.
True diamond dataflow with cross-agent context passing.

Usage: python3 -m examples.python.acp-agents.cross_critique_pipeline
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def cross_critique_pipeline(g: GraphRecorder):
    """Diamond pattern: parallel proposals, then cross-critiques."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents
    agent_a = g.spawn("agent_a", profile="claude", cwd=cwd)
    agent_b = g.spawn("agent_b", profile="codex", cwd=cwd)

    # Parallel proposals
    question = (
        "In the APXM project (this codebase), what is the single most impactful next feature "
        "to implement for the ACP multi-agent system? Propose your top idea with a one-paragraph "
        "justification. Be specific and concrete. Under 150 words."
    )
    agent_a.ask(question)
    agent_b.ask(question)

    # Print parallel proposals
    print1 = g.print_(
        "print_proposals",
        message="=== PROPOSALS (parallel) ===\nClaude:\n{0}\n\nCodex:\n{1}"
    )
    agent_a.get_last_node() | print1
    agent_b.get_last_node() | print1

    # Cross-critiques: A critiques B, B critiques A
    codex_critique_prompt = g.ask(
        "build_codex_critique",
        template="Critique the following feature proposal from Claude for the APXM project. "
        "Is it feasible? Is it truly the most impactful? What's missing? Under 100 words.\n\n"
        "Claude's proposal:\n{0}"
    )
    agent_a.get_last_node() | codex_critique_prompt
    print1 >> codex_critique_prompt

    claude_critique_prompt = g.ask(
        "build_claude_critique",
        template="Critique the following feature proposal from Codex for the APXM project. "
        "Is it feasible? Is it truly the most impactful? What's missing? Under 100 words.\n\n"
        "Codex's proposal:\n{0}"
    )
    agent_b.get_last_node() | claude_critique_prompt
    print1 >> claude_critique_prompt

    # Send cross-critiques
    codex_critiques = g.communicate(
        "codex_critiques_claude",
        target_agent="agent_b",
        message="{0}"
    )
    codex_critique_prompt | codex_critiques

    claude_critiques = g.communicate(
        "claude_critiques_codex",
        target_agent="agent_a",
        message="{0}"
    )
    claude_critique_prompt | claude_critiques

    # Print critiques
    print2 = g.print_(
        "print_critiques",
        message="=== CRITIQUES ===\nCodex critiques Claude:\n{0}\n\nClaude critiques Codex:\n{1}"
    )
    codex_critiques | print2
    claude_critiques | print2

    g.return_("result", source=print2)
    


if __name__ == "__main__":
    import json
    print(cross_critique_pipeline._graph.to_air())
