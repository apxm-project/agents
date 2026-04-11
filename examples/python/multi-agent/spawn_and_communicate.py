#!/usr/bin/env python3
"""spawn_and_communicate.py -- Spawn agents and communicate via ACP

Part 1: Spawn a single agent and send one message.
Part 2: Two-agent pipeline — analyst produces analysis, summarizer condenses it.

Usage: dekk apxm execute examples/python/multi-agent/spawn_and_communicate.py
"""

from apxm import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def basic_spawn(g: GraphRecorder):
    """Spawn one Claude agent and send a single review request."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    reviewer = g.spawn("reviewer", profile=claude, cwd=cwd)
    reviewer.ask("Review the current directory structure and suggest improvements.")

    review = reviewer.get_last_node()
    output = g.print("{review}")
    g.done(output)


@compile()
def two_agent_pipeline(g: GraphRecorder):
    """Two-agent pipeline: analyst writes analysis, summarizer condenses it."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn two agents
    analyst = g.spawn("analyst", profile=claude, cwd=cwd)
    summarizer = g.spawn("summarizer", profile=claude, cwd=cwd)

    # Analyst conducts analysis
    analyst.ask(
        "Conduct a thorough technical analysis of APXM as an AI execution framework. "
        "Cover: architecture, capabilities, use cases, limitations. 400 words."
    )
    analysis = analyst.get_last_node()

    # Summarizer receives analysis via COMMUNICATE
    summary = g.communicate(
        "summary_request",
        target_agent="summarizer",
        message="Produce a 3-bullet executive summary:\n\n{analysis}"
    )

    output = g.print("=== ANALYSIS ===\n{analysis}\n\n=== SUMMARY ===\n{summary}")
    g.done(output)


if __name__ == "__main__":
    print(two_agent_pipeline._graph.to_air())
