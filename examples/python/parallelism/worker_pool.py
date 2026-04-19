#!/usr/bin/env python3
"""worker_pool.py - 3 parallel workers process tasks concurrently

Workers process tasks in parallel, then aggregate results.

Usage: python3 -m examples.python.patterns.worker-pool.worker_pool
"""

from apxm import compile, GraphRecorder


@compile()
def worker_pool_parallel(g: GraphRecorder):
    """Parallel worker pool pattern with result aggregation."""
    # Define task list
    task_list = g.ask(
        name="task_list",
        prompt="List 3 distinct technical tasks for analysis. Format as:\n"
        "TASK A: [description]\n"
        "TASK B: [description]\n"
        "TASK C: [description]"
    )

    # Three parallel workers
    worker_a = g.think(
        name="worker_a",
        prompt="You are Worker A. From this task list, perform deep analysis of TASK A:\n{task_list}\n\n"
        "Provide: (1) key findings, (2) confidence score 0-1, (3) recommended next action."
    )

    worker_b = g.think(
        name="worker_b",
        prompt="You are Worker B. From this task list, perform deep analysis of TASK B:\n{task_list}\n\n"
        "Provide: (1) key findings, (2) confidence score 0-1, (3) recommended next action."
    )

    worker_c = g.think(
        name="worker_c",
        prompt="You are Worker C. From this task list, perform deep analysis of TASK C:\n{task_list}\n\n"
        "Provide: (1) key findings, (2) confidence score 0-1, (3) recommended next action."
    )

    # Aggregate results
    aggregate = g.think(
        name="aggregate",
        prompt="Three workers processed parallel tasks. Aggregate their findings:\n\n"
        "WORKER A:\n{worker_a}\n\nWORKER B:\n{worker_b}\n\nWORKER C:\n{worker_c}\n\n"
        "Ranked summary: highest confidence findings and priority actions."
    )

    # Store in memory
    mem = g.update_memory("store_aggregate", data=aggregate, key="worker_pool_aggregate")
    g.add_edge(aggregate, mem)

    # Print and return
    output = g.print(message="=== WORKER POOL RESULTS ===\n{aggregate}")
    g.add_edge(mem, output, dependency="Control")

    g.done(output)
    


if __name__ == "__main__":
    import apxm

    result = apxm.run(worker_pool_parallel())
    print(result.content)
