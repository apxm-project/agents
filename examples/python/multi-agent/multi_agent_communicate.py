#!/usr/bin/env python3
"""multi_agent_communicate.py - Cross-agent COMMUNICATE demo

Demonstrates: spawn_agent + communicate(acp) for two-agent pipeline

Usage: python3 -m examples.python.multi-agent.multi_agent_communicate
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def pipeline(g: GraphRecorder):
    """Two-agent pipeline with ACP communication."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents
    analyst = g.spawn("analyst", profile="claude", cwd=cwd)
    summarizer = g.spawn("summarizer", profile="claude", cwd=cwd)

    # Analyst conducts analysis
    analyst.ask(
        "Conduct a thorough technical analysis of APXM as an AI execution framework. "
        "Cover: architecture, capabilities, use cases, limitations, competitive positioning. "
        "400 words."
    )

    # Summarizer produces executive summary
    summary_comm = g.communicate(
        "summary_request",
        target_agent="summarizer",
        message="Produce a 3-bullet executive summary of this technical analysis:\n\n{0}"
    )
    analyst.get_last_node() | summary_comm

    # Print both outputs
    print_node = g.print_(
        "output",
        message="=== ANALYSIS ===\n\n{0}\n\n=== SUMMARY ===\n\n{1}"
    )
    analyst.get_last_node() | print_node
    summary_comm | print_node

    g.return_("result", source=print_node)
    


if __name__ == "__main__":
    import json
    print(pipeline._graph.to_air())
