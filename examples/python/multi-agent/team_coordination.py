#!/usr/bin/env python3
"""team_coordination.py -- Team sugar API for multi-agent coordination

Demonstrates g.team(), team.add(), handle.ask(), team.wait_all(),
and team.merge() for coordinating multiple agents.

Usage: dekk apxm execute examples/python/multi-agent/team_coordination.py
"""

from apxm import compile, GraphRecorder
from apxm._generated.agents import claude, codex
import os


@compile()
def team_coordination(g: GraphRecorder):
    """3-agent team: architect designs, coder implements, reviewer verifies."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Create a team
    team = g.team("dev_team")

    # Add team members with different profiles
    architect = team.add("architect", profile=claude, cwd=cwd)
    coder = team.add("coder", profile=codex, cwd=cwd)
    reviewer = team.add("reviewer", profile=claude, cwd=cwd)

    # Each member gets a task
    architect.ask("Design a REST API for a task manager with CRUD operations.")
    coder.ask("Implement a basic HTTP server in Python with 3 endpoints.")
    reviewer.ask("Write a code review checklist for REST API implementations.")

    # Wait for all to complete, then merge results
    sync = team.wait_all("sync")
    results = team.merge("results")
    sync >> results

    output = g.print("output", message="Team results merged: {results}")
    g.done(output)


if __name__ == "__main__":
    print(team_coordination._graph.to_air())
