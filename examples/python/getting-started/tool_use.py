#!/usr/bin/env python3
"""tool_use.py - Workflow that invokes a Python-backed tool.

Usage: dekk apxm execute examples/python/getting-started/tool_use.py
"""

from apxm import GraphRecorder, compile, tool


@tool
def search_docs(query: str) -> str:
    """Return deterministic search results for a query."""
    return (
        f"Search results for {query}: "
        "APXM records tool nodes, validates capability bindings, and emits session metrics."
    )


@compile()
def tool_agent(g: GraphRecorder):
    """Invoke a Python-backed search tool."""
    results = g.invoke_tool(search_docs, query="APXM Python tool integration")

    summary = g.print(message="TOOL RESULT:\n{results}")

    g.done(summary)


if __name__ == "__main__":
    import apxm

    result = apxm.run(tool_agent(), mock=True)
    print(result.content)
