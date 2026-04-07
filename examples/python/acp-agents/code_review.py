#!/usr/bin/env python3
"""code_review.py - Codex analyzes code, Claude reviews the analysis

Sequential pipeline: Codex goes first, Claude gets the full context.

Usage: python3 -m examples.python.acp-agents.code_review
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude, codex
import os


@compile()
def code_review(g: GraphRecorder):
    """Sequential code review workflow with Codex and Claude."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn both agents
    coder = g.spawn("coder", profile=codex, cwd=cwd)
    reviewer = g.spawn("reviewer", profile=claude, cwd=cwd)

    # Codex analyzes the code
    coding_task = (
        "Look at the file crates/apxm-runtime/src/executor/handlers/spawn_agent.rs "
        "in this codebase. Write a brief summary of what it does and identify one "
        "potential improvement or edge case that could be handled better. Keep your "
        "response under 200 words."
    )
    coder.ask(coding_task)

    # Get coder's last node for reference
    codex_analysis = coder.get_last_node()

    # Print Codex's analysis
    print1 = g.print("=== CODEX ANALYSIS ===\n{codex_analysis}")

    # Claude reviews Codex's analysis
    review_prompt = g.ask(
        "build_review_prompt",
        template="You are doing a code review. Here is Codex's analysis of the spawn_agent handler. "
        "Review it critically: is it accurate? Did it miss anything? Do you agree with the "
        "improvement? Be concise, under 150 words.\n\nCodex's analysis:\n{codex_analysis}"
    )

    # Send to Claude
    reviewer_critiques = g.communicate(
        "reviewer_critiques",
        target_agent="reviewer",
        message="{review_prompt}"
    )
    print1 >> reviewer_critiques

    # Print Claude's review
    print2 = g.print("=== CLAUDE REVIEW ===\n{reviewer_critiques}")

    g.done(print2)
    


if __name__ == "__main__":
    import json
    print(code_review._graph.to_air())
