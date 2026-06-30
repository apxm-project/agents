#!/usr/bin/env python3
"""parallel_agents.py - Parallel: spawn Claude and Codex, compare their analyses

Usage: dekk agents execute examples/python/multi-agent/parallel_agents.py
"""

from apxm import GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude, codex


@compile()
def parallel_agents(g: GraphRecorder):
    """Spawn Claude and Codex agents to compare analyses."""
    cwd = agent_cwd()

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
    claude_analysis = claude_analyst.ask(prompt)
    codex_analysis = codex_analyst.ask(prompt)

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
