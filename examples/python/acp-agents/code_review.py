#!/usr/bin/env python3
"""code_review.py - Codex analyzes code, Claude reviews the analysis

Sequential pipeline: Codex goes first, Claude gets the full context.

Usage: python3 -m examples.python.acp-agents.code_review
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def code_review(g: GraphRecorder):
    """Sequential code review workflow with Codex and Claude."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn both agents
    coder = g.spawn("coder", profile="codex", cwd=cwd)
    reviewer = g.spawn("reviewer", profile="claude", cwd=cwd)

    # Codex analyzes the code
    coding_task = (
        "Look at the file crates/apxm-runtime/src/executor/handlers/spawn_agent.rs "
        "in this codebase. Write a brief summary of what it does and identify one "
        "potential improvement or edge case that could be handled better. Keep your "
        "response under 200 words."
    )
    coder.ask(coding_task)

    # Print Codex's analysis
    print1 = g.print_("print_codex", message="=== CODEX ANALYSIS ===\n{0}")
    coder.get_last_node() | print1

    # Claude reviews Codex's analysis
    review_prompt = g.ask(
        "build_review_prompt",
        "You are doing a code review. Here is Codex's analysis of the spawn_agent handler. "
        "Review it critically: is it accurate? Did it miss anything? Do you agree with the "
        "improvement? Be concise, under 150 words.\n\nCodex's analysis:\n{0}"
    )
    coder.get_last_node() | review_prompt

    # Send to Claude
    reviewer_critiques = g.communicate(
        "reviewer_critiques",
        target_agent="reviewer",
        message="{0}"
    )
    review_prompt | reviewer_critiques
    print1 >> reviewer_critiques

    # Print Claude's review
    print2 = g.print_("print_claude", message="=== CLAUDE REVIEW ===\n{0}")
    reviewer_critiques | print2

    g.return_("result", source=print2)
    


if __name__ == "__main__":
    import json
    print(json.dumps(code_review._graph.to_dict(), indent=2))
