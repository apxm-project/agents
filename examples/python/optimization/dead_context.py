#!/usr/bin/env python3
"""dead_context.py -- Demonstrate dead context elimination

Five context values are wired to a downstream node, but its template
only references {0} and {1}. The DeadContextElimination pass prunes
the unused inputs, saving tokens.

O0: 5 context values sent to LLM. O2: 2 (3 pruned).

Usage: dekk apxm execute examples/python/optimization/dead_context.py
"""

from apxm import compile, GraphRecorder


@compile()
def dead_context_demo(g: GraphRecorder):
    """5 context inputs, only 2 used. O0: 5 sent. O2: 2 (3 pruned)."""

    # Five context chunks
    schema = g.text("schema", value="Users table: id, name, email, created_at")
    api = g.text("api", value="GET /users, POST /users, DELETE /users/:id")
    env = g.text("env", value="DATABASE_URL=postgres://localhost/app")
    metrics = g.text("metrics", value="Avg response: 120ms, p99: 450ms")
    audit = g.text("audit", value="Last scan: all clear, no CVEs found")

    # Only uses {0} (schema) and {1} (api) -- other 3 are dead context
    analysis = g.think(
        "analysis",
        "{0}\n{1}\n\nList the main entities and their API endpoints."
    )
    schema | analysis
    api | analysis
    env | analysis
    metrics | analysis
    audit | analysis

    output = g.print("Analysis:\n{analysis}")
    g.done(output)


if __name__ == "__main__":
    print(dead_context_demo._graph.to_air())
