#!/usr/bin/env python3
"""architect.py - Software architecture design workflow

Usage: python3 -m examples.python.workflows.sub.architect
"""

from apxm.graph import compile, GraphRecorder


@compile()
def architect(g: GraphRecorder, task: str):
    """Design high-level architecture for a task.

    Parameters
    ----------
    task : str
        The task to design architecture for.
    """
    architect_think = g.think(
        "architect_think",
        template="You are a software architect. Design a high-level architecture for: {task}\n\n"
        "Provide:\n"
        "1. Key components\n"
        "2. Data flow\n"
        "3. Technology recommendations\n\n"
        "Be concise but thorough."
    )

    g.return_("result", source=architect_think)
    


if __name__ == "__main__":
    import json
    print(architect._graph.to_air())
