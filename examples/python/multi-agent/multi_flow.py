#!/usr/bin/env python3
"""multi_flow.py - Agent with multiple flows and cross-agent calls

Note: This example demonstrates flow composition concepts but uses direct
graph composition since the Python frontend doesn't yet support multi-agent
cross-flow calls like "Researcher.research(topic)".

Usage: python3 -m examples.python.multi-agent.multi_flow
"""

from apxm.graph import compile, GraphRecorder


@compile()
def researcher_research(g: GraphRecorder, topic: str):
    """Research flow."""
    findings = g.think("findings", "Research this topic thoroughly: {topic}")
    g.return_("result", source=findings)
    


@compile()
def researcher_critique(g: GraphRecorder, text: str):
    """Critique flow."""
    evaluation = g.reason(
        "evaluation",
        "Critically evaluate this text for accuracy: {text}"
    )
    g.return_("result", source=evaluation)
    


@compile()
def coordinator_main(g: GraphRecorder):
    """Coordinator that orchestrates research and critique."""
    topic = g.ask("topic", "What topic should we investigate?")

    # Research findings
    findings = g.think("findings", "Research this topic thoroughly: {0}")
    topic | findings

    # Critique the findings
    evaluation = g.reason("evaluation", "Critically evaluate this text for accuracy: {0}")
    findings | evaluation

    # Final summary
    summary = g.ask("summary", "Provide a final summary of the investigation based on: {0}")
    evaluation | summary

    g.return_("result", source=summary)
    


if __name__ == "__main__":
    import json
    # Print the coordinator workflow
    print(coordinator_main._graph.to_air())
