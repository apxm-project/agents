#!/usr/bin/env python3
"""cross_critique_pipeline.py - Diamond dataflow with cross-agent context passing

Claude and Codex propose next ACP features in parallel, then each critiques the other.
True diamond dataflow with cross-agent context passing.

Usage: dekk agents execute examples/python/patterns/cross_critique.py
"""

from apxm import DependencyType, GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude, codex


@compile()
def cross_critique_pipeline(g: GraphRecorder):
    """Diamond pattern: parallel proposals, then cross-critiques."""
    cwd = agent_cwd()

    # Spawn agents
    agent_a = g.spawn("agent_a", profile=claude, cwd=cwd)
    agent_b = g.spawn("agent_b", profile=codex, cwd=cwd)

    # Parallel proposals
    question = (
        "In the APXM project (this codebase), what is the single most impactful next feature "
        "to implement for the ACP multi-agent system? Propose your top idea with a one-paragraph "
        "justification. Be specific and concrete. Under 150 words."
    )
    claude_proposal = agent_a.ask(question)
    codex_proposal = agent_b.ask(question)

    # Print parallel proposals
    print1 = g.print(message="=== PROPOSALS (parallel) ===\nClaude:\n{claude_proposal}\n\nCodex:\n{codex_proposal}")

    # Cross-critiques: A critiques B, B critiques A
    codex_critique_prompt = g.ask(
        name="build_codex_critique",
        prompt="Critique the following feature proposal from Claude for the APXM project. "
        "Is it feasible? Is it truly the most impactful? What's missing? Under 100 words.\n\n"
        "Claude's proposal:\n{claude_proposal}"
    )
    g.add_edge(print1, codex_critique_prompt, dependency=DependencyType.CONTROL)

    claude_critique_prompt = g.ask(
        name="build_claude_critique",
        prompt="Critique the following feature proposal from Codex for the APXM project. "
        "Is it feasible? Is it truly the most impactful? What's missing? Under 100 words.\n\n"
        "Codex's proposal:\n{codex_proposal}"
    )
    g.add_edge(print1, claude_critique_prompt, dependency=DependencyType.CONTROL)

    # Send cross-critiques
    codex_critiques = agent_b.ask("{codex_critique_prompt}")

    claude_critiques = agent_a.ask("{claude_critique_prompt}")

    # Print critiques
    print2 = g.print(message="=== CRITIQUES ===\nCodex critiques Claude:\n{codex_critiques}\n\nClaude critiques Codex:\n{claude_critiques}")

    g.done(print2)
    


if __name__ == "__main__":
    import apxm

    result = apxm.run(cross_critique_pipeline())
    print(result.content)
