#!/usr/bin/env python3
"""resilient_pipeline.py -- Resilient agent pipeline with retry semantics

Demonstrates spawning a worker agent, formatting a task, and handling
the result. APXM provides automatic resilience:

- Exponential backoff retry (500ms to 60s) on transient failures
- Provider health tracking (unhealthy backends are deprioritized)
- Each COMMUNICATE node is atomic -- the scheduler handles failures

Usage: dekk apxm execute examples/python/patterns/resilient_pipeline.py
"""

from apxm import GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude


@compile()
def resilient_pipeline(g: GraphRecorder):
    """Resilient ACP pipeline: format task, send to worker, review result."""
    cwd = agent_cwd()

    # Spawn worker agent
    # APXM runtime tracks provider health and retries transient failures
    worker = g.spawn("worker", profile=claude, cwd=cwd)

    # Define and format task
    task = g.ask(
        name="task",
        prompt="Describe a complex coding task that requires careful implementation."
    )

    formatted = g.think(
        name="formatted",
        prompt="Format this as a precise coding instruction for an agent:\n{task}"
    )

    # Send to worker
    # Each COMMUNICATE is atomic -- exponential backoff on failure (500ms-60s)
    result = g.communicate(
        name="worker_result",
        target_agent="worker",
        message="{formatted}"
    )

    # Review result
    summary = g.think(
        name="summary",
        prompt="Review the worker's result and summarize what was accomplished:\n{result}"
    )

    output = g.print(message="=== RESULT ===\n{summary}")
    g.done(output)


if __name__ == "__main__":
    import apxm

    result = apxm.run(resilient_pipeline())
    print(result.content)
