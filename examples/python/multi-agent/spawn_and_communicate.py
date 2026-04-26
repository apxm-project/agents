#!/usr/bin/env python3
"""spawn_and_communicate.py -- Spawn agents and communicate via ACP

Part 1: Spawn a single agent and send one message.
Part 2: Two-agent pipeline — analyst produces analysis, summarizer condenses it.

Usage: dekk apxm execute examples/python/multi-agent/spawn_and_communicate.py
"""

from apxm import GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude


@compile()
def basic_spawn(g: GraphRecorder):
    """Spawn one Claude agent and send a single review request."""
    cwd = agent_cwd()

    reviewer = g.spawn("reviewer", profile=claude, cwd=cwd)
    review = reviewer.ask("Review the current directory structure and suggest improvements.")
    output = g.print(message="{review}")
    g.done(output)


@compile()
def two_agent_pipeline(g: GraphRecorder):
    """Two-agent pipeline: analyst writes analysis, summarizer condenses it."""
    cwd = agent_cwd()

    # Spawn two agents
    analyst = g.spawn("analyst", profile=claude, cwd=cwd)
    summarizer = g.spawn("summarizer", profile=claude, cwd=cwd)

    # Analyst conducts analysis
    analysis = analyst.ask(
        "Conduct a thorough technical analysis of APXM as an AI execution framework. "
        "Cover: architecture, capabilities, use cases, limitations. 400 words."
    )

    # Summarizer receives analysis via the spawned-agent handle.
    summary = summarizer.ask("Produce a 3-bullet executive summary:\n\n{analysis}")

    output = g.print(message="=== ANALYSIS ===\n{analysis}\n\n=== SUMMARY ===\n{summary}")
    g.done(output)


if __name__ == "__main__":
    import apxm

    result = apxm.run(two_agent_pipeline())
    print(result.content)
