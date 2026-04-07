#!/usr/bin/env python3
"""resilient_acp_pipeline.py - Resilient ACP pattern

Spawn agent, send task, handle result.

Usage: python3 -m examples.python.patterns.resilient-acp.resilient_acp_pipeline
"""

from apxm import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def resilient_acp(g: GraphRecorder):
    """Resilient ACP pattern with task formatting and result handling."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn worker agent
    worker = g.spawn("worker", profile=claude, cwd=cwd)

    # Define and format task
    task_description = g.ask(
        "task_description",
        "Describe a complex coding task that requires careful implementation."
    )

    formatted_task = g.think(
        "formatted_task",
        "Format this as a precise coding instruction for an agent:\n{task_description}"
    )

    # Send to worker
    worker_comm = g.communicate(
        "worker_result",
        target_agent="worker",
        message="{formatted_task}"
    )

    # Review and summarize result
    summary = g.think(
        "summary",
        "Review the worker's result and summarize what was accomplished:\n{worker_comm}"
    )

    # Print and return
    output = g.print("=== RESULT ===\n{summary}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(resilient_acp._graph.to_air())
