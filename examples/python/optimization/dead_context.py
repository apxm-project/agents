#!/usr/bin/env python3
"""dead_context.py -- Demonstrate dead context elimination

Five context values are wired to a downstream node, but its template
only references {schema} and {api}. The DeadContextElimination pass
prunes the unused inputs, saving tokens.

This example uses the explicit `name=` form so that the strings used
in `{...}` placeholders are explicit and do not depend on the local
Python variable names. Most examples use the implicit form (variable
name == placeholder name); see `shared_prefix.py` for the implicit
style.

O0: 5 context values sent to LLM. O2: 2 (3 pruned).

Usage: dekk agents execute examples/python/optimization/dead_context.py
"""

from apxm import compile, GraphRecorder
from apxm.constants import INPUT_NAMES


@compile()
def dead_context_demo(g: GraphRecorder):
    """5 context inputs, only 2 used. O0: 5 sent. O2: 2 (3 pruned)."""

    # Five context chunks
    schema_text = "Users table: id, name, email, created_at"
    api_text = "GET /users, POST /users, DELETE /users/:id"
    env_text = "DATABASE_URL=postgres://localhost/app"
    metrics_text = "Avg response: 120ms, p99: 450ms"
    audit_text = "Last scan: all clear, no CVEs found"

    # Each context node is named explicitly; the placeholder strings in
    # `analysis` reference these names directly.
    ctx_schema = g.ask(name="schema", prompt=schema_text)
    ctx_api = g.ask(name="api", prompt=api_text)
    ctx_env = g.ask(name="env", prompt=env_text)
    ctx_metrics = g.ask(name="metrics", prompt=metrics_text)
    ctx_audit = g.ask(name="audit", prompt=audit_text)

    # The template references {schema} and {api} only. We explicitly
    # declare every context as an input so that, at O0, all five are
    # sent to the LLM. DeadContextElimination prunes the three unused
    # contexts at O2.
    analysis = g.think(
        name="analysis",
        prompt="{schema}\n{api}\n\nList the main entities and their API endpoints.",
        **{INPUT_NAMES: ["schema", "api", "env", "metrics", "audit"]},
    )
    g.add_edge(ctx_schema, analysis)
    g.add_edge(ctx_api, analysis)
    g.add_edge(ctx_env, analysis)
    g.add_edge(ctx_metrics, analysis)
    g.add_edge(ctx_audit, analysis)

    output = g.print(name="report", message="Analysis:\n{analysis}")
    g.done(output)


if __name__ == "__main__":
    import apxm

    result = apxm.run(dead_context_demo())
    print(result.content)
