#!/usr/bin/env python3
"""worker_pool.py - 3 parallel workers process tasks concurrently

Workers process tasks in parallel, then aggregate results.

Usage: python3 -m examples.python.patterns.worker-pool.worker_pool
"""

from apxm.graph import compile, GraphRecorder


@compile()
def worker_pool_parallel(g: GraphRecorder):
    """Parallel worker pool pattern with result aggregation."""
    # Define task list
    task_list = g.ask(
        "task_list",
        "List 3 distinct technical tasks for analysis. Format as:\n"
        "TASK A: [description]\n"
        "TASK B: [description]\n"
        "TASK C: [description]"
    )

    # Three parallel workers
    worker_a = g.think(
        "worker_a",
        "You are Worker A. From this task list, perform deep analysis of TASK A:\n{0}\n\n"
        "Provide: (1) key findings, (2) confidence score 0-1, (3) recommended next action."
    )
    task_list | worker_a

    worker_b = g.think(
        "worker_b",
        "You are Worker B. From this task list, perform deep analysis of TASK B:\n{0}\n\n"
        "Provide: (1) key findings, (2) confidence score 0-1, (3) recommended next action."
    )
    task_list | worker_b

    worker_c = g.think(
        "worker_c",
        "You are Worker C. From this task list, perform deep analysis of TASK C:\n{0}\n\n"
        "Provide: (1) key findings, (2) confidence score 0-1, (3) recommended next action."
    )
    task_list | worker_c

    # Aggregate results
    aggregate = g.think(
        "aggregate",
        "Three workers processed parallel tasks. Aggregate their findings:\n\n"
        "WORKER A:\n{0}\n\nWORKER B:\n{1}\n\nWORKER C:\n{2}\n\n"
        "Ranked summary: highest confidence findings and priority actions."
    )
    worker_a | aggregate
    worker_b | aggregate
    worker_c | aggregate

    # Store in memory
    mem = g.update_memory("store_aggregate", data="{0}", key="worker_pool_aggregate")
    aggregate | mem

    # Print and return
    output = g.print_("output", message="=== WORKER POOL RESULTS ===\n{0}")
    aggregate | output
    mem >> output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(json.dumps(worker_pool_parallel._graph.to_dict(), indent=2))
