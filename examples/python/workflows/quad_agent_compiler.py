#!/usr/bin/env python3
"""quad_agent_compiler.py - Spawn 4 parallel Claude agents for compiler enhancements

Usage: python3 -m examples.python.workflows.quad_agent_compiler
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def compiler_orchestrator(g: GraphRecorder):
    """Spawn 4 parallel Claude agents for compiler enhancements."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn all 4 agents using Team sugar
    team = g.team("compiler_team")
    agent1 = team.add("agent1", profile="claude", cwd=cwd)
    agent2 = team.add("agent2", profile="claude", cwd=cwd)
    agent3 = team.add("agent3", profile="claude", cwd=cwd)
    agent4 = team.add("agent4", profile="claude", cwd=cwd)

    # Dispatch tasks in parallel
    agent1.ask(
        "Read the file /tmp/agent1-task.txt using bash and implement exactly what it says. "
        "Start: bash cat /tmp/agent1-task.txt. Then read all referenced source files. "
        "Implement everything. Use dekk apxm build to verify. Commit when done."
    )

    agent2.ask(
        "Read the file /tmp/agent2-task.txt using bash and implement exactly what it says. "
        "Start: bash cat /tmp/agent2-task.txt. Then read all referenced source files. "
        "Implement everything. Use dekk apxm build to verify. Commit when done."
    )

    agent3.ask(
        "Read the file /tmp/agent3-task.txt using bash and implement exactly what it says. "
        "Start: bash cat /tmp/agent3-task.txt. Then read all referenced source files. "
        "Implement everything. Use dekk apxm build to verify. Commit when done."
    )

    agent4.ask(
        "Read the file /tmp/agent4-task.txt using bash and implement exactly what it says. "
        "Start: bash cat /tmp/agent4-task.txt. Then read all referenced source files. "
        "Implement everything. Use dekk apxm build to verify. Commit when done."
    )

    # Wait for all to complete
    sync = team.wait_all("sync_all")

    # Synthesize results
    summary = g.think(
        "summary",
        template="Four compiler enhancement agents completed. Summarize results. "
        "AGENT1 (ModelProfile): {0} AGENT2 (Diagnostics): {1} "
        "AGENT3 (Flow Param Fix): {2} AGENT4 (Passes+CLI): {3}"
    )
    agent1.get_last_node() | summary
    agent2.get_last_node() | summary
    agent3.get_last_node() | summary
    agent4.get_last_node() | summary
    sync >> summary

    # Print and return
    output = g.print_("output", message="=== COMPILER ENHANCEMENT COMPLETE ===\n{0}")
    summary | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(compiler_orchestrator._graph.to_air())
