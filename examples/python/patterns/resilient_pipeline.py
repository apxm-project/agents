#!/usr/bin/env python3
"""resilient_pipeline.py -- Resilient agent pipeline with retry semantics

Demonstrates spawning a worker agent, formatting a task, and handling
the result. APXM provides automatic resilience:

- Exponential backoff retry (500ms to 60s) on transient failures
- Provider health tracking (unhealthy backends are deprioritized)
- Each COMMUNICATE node is atomic -- the scheduler handles failures

Usage: dekk apxm execute examples/python/patterns/resilient_pipeline.py
"""

from apxm import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def resilient_pipeline(g: GraphRecorder):
    """Resilient ACP pipeline: format task, send to worker, review result."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn worker agent
    # APXM runtime tracks provider health and retries transient failures
    worker = g.spawn("worker", profile=claude, cwd=cwd)

    # Define and format task
    task = g.ask(
        "task",
        "Describe a complex coding task that requires careful implementation."
    )

    formatted = g.think(
        "formatted",
        "Format this as a precise coding instruction for an agent:\n{task}"
    )

    # Send to worker
    # Each COMMUNICATE is atomic -- exponential backoff on failure (500ms-60s)
    result = g.communicate(
        "worker_result",
        target_agent="worker",
        message="{formatted}"
    )

    # Review result
    summary = g.think(
        "summary",
        "Review the worker's result and summarize what was accomplished:\n{result}"
    )

    output = g.print("=== RESULT ===\n{summary}")
    g.done(output)


if __name__ == "__main__":
    print(resilient_pipeline._graph.to_air())
