#!/usr/bin/env python3
"""parallel_agents.py - Parallel: spawn Claude and Codex, compare their analyses

Usage: python3 -m examples.python.acp-agents.parallel_agents
"""

from apxm import compile, GraphRecorder
from apxm._generated.agents import claude, codex
import os


@compile()
def parallel_agents(g: GraphRecorder):
    """Spawn Claude and Codex agents to compare analyses."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn two agents in parallel
    claude_analyst = g.spawn(
        "claude_analyst",
        profile=claude,
        cwd=cwd
    )

    codex_analyst = g.spawn(
        "codex_analyst",
        profile=codex,
        cwd=cwd
    )

    # Send same prompt to both agents (parallel execution)
    prompt = "Analyze the architecture of this project and suggest improvements."
    claude_analyst.ask(prompt)
    codex_analyst.ask(prompt)

    # Get analyses
    claude_analysis = claude_analyst.get_last_node()
    codex_analysis = codex_analyst.get_last_node()

    # Merge the analyses
    merge_analyses = g.ask(
        name="merge_analyses",
        prompt="Compare and synthesize these two analyses:\n\nClaude:\n{claude_analysis}\n\nCodex:\n{codex_analysis}"
    )

    # Print and return
    output = g.print(message="{merge_analyses}")

    g.done(output)
    


if __name__ == "__main__":
    import apxm

    result = apxm.run(parallel_agents())
    print(result.content)
