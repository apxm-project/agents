#!/usr/bin/env python3
"""tool_use.py - Agent that invokes a tool

Usage: python3 -m examples.python.basics.tool_use
"""

from apxm.graph import compile, GraphRecorder


@compile()
def tool_agent(g: GraphRecorder):
    """Agent that uses external tool for search."""
    # Register the search capability
    register = g.register_capability(
        "register_search",
        capability_name="search",
        description="Search capability for web queries",
        parameters_schema={"type": "object", "properties": {"query": {"type": "string"}}}
    )

    topic = g.ask("ask_topic", template="What topic should we research?")
    register >> topic

    # Invoke the search tool
    results = g.invoke("search_results", capability="search", params={"query": "{topic}"})

    # Summarize findings
    summary = g.ask("summarize", template="Summarize these findings: {results}")

    g.done(summary)
    


if __name__ == "__main__":
    import json
    print(tool_agent._graph.to_air())
