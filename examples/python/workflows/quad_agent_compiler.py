#!/usr/bin/env python3
"""quad_agent_compiler.py - Spawn 4 parallel Claude agents for compiler enhancements

Usage: python3 -m examples.python.workflows.quad_agent_compiler
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude, codex
import os


@compile()
def compiler_orchestrator(g: GraphRecorder):
    """Spawn 4 parallel Claude agents for compiler enhancements.

    Demonstrates Team sugar and auto-wiring with AgentHandle references.
    """
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn all 4 agents using Team sugar
    team = g.team("compiler_team")
    a1 = team.add("agent1", profile=claude, cwd=cwd)
    a2 = team.add("agent2", profile=claude, cwd=cwd)
    a3 = team.add("agent3", profile=claude, cwd=cwd)
    a4 = team.add("agent4", profile=claude, cwd=cwd)

    # Dispatch tasks in parallel
    a1.ask(
        "Read the file /tmp/agent1-task.txt using bash and implement exactly what it says. "
        "Start: bash cat /tmp/agent1-task.txt. Then read all referenced source files. "
        "Implement everything. Use dekk apxm build to verify. Commit when done."
    )

    a2.ask(
        "Read the file /tmp/agent2-task.txt using bash and implement exactly what it says. "
        "Start: bash cat /tmp/agent2-task.txt. Then read all referenced source files. "
        "Implement everything. Use dekk apxm build to verify. Commit when done."
    )

    a3.ask(
        "Read the file /tmp/agent3-task.txt using bash and implement exactly what it says. "
        "Start: bash cat /tmp/agent3-task.txt. Then read all referenced source files. "
        "Implement everything. Use dekk apxm build to verify. Commit when done."
    )

    a4.ask(
        "Read the file /tmp/agent4-task.txt using bash and implement exactly what it says. "
        "Start: bash cat /tmp/agent4-task.txt. Then read all referenced source files. "
        "Implement everything. Use dekk apxm build to verify. Commit when done."
    )

    # Wait for all to complete
    sync = team.wait_all("sync_all")

    # Synthesize results (auto-wired from {a1}, {a2}, {a3}, {a4})
    summary = g.think(
        "Four compiler enhancement agents completed. Summarize results. "
        "AGENT1 (ModelProfile): {a1} AGENT2 (Diagnostics): {a2} "
        "AGENT3 (Flow Param Fix): {a3} AGENT4 (Passes+CLI): {a4}"
    )
    sync >> summary

    # Print and return (auto-wired from {summary})
    g.print("=== COMPILER ENHANCEMENT COMPLETE ===\n{summary}")
    g.done(summary)


if __name__ == "__main__":
    import json
    print(compiler_orchestrator._graph.to_air())
