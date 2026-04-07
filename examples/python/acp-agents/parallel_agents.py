#!/usr/bin/env python3
"""parallel_agents.py - Parallel: spawn Claude and Codex, compare their analyses

Usage: python3 -m examples.python.acp-agents.parallel_agents
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def parallel_agents(g: GraphRecorder):
    """Spawn Claude and Codex agents to compare analyses."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn two agents in parallel
    claude_analyst = g.spawn(
        "claude_analyst",
        profile="claude",
        cwd=cwd
    )

    codex_analyst = g.spawn(
        "codex_analyst",
        profile="codex",
        cwd=cwd
    )

    # Send same prompt to both agents (parallel execution)
    prompt = "Analyze the architecture of this project and suggest improvements."
    claude_analyst.ask(prompt)
    codex_analyst.ask(prompt)

    # Merge the analyses
    merge_analyses = g.ask(
        "merge_analyses",
        template="Compare and synthesize these two analyses:\n\nClaude:\n{0}\n\nCodex:\n{1}"
    )
    claude_analyst.get_last_node() | merge_analyses
    codex_analyst.get_last_node() | merge_analyses

    # Print and return
    output = g.print_("output", message="{0}")
    merge_analyses | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(parallel_agents._graph.to_air())
