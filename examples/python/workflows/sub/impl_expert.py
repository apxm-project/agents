#!/usr/bin/env python3
"""impl_expert.py - Implementation expert workflow

Usage: python3 -m examples.python.workflows.sub.impl_expert
"""

from apxm.graph import compile, GraphRecorder


@compile()
def impl_expert(g: GraphRecorder, task: str):
    """Provide implementation guidance for a task.

    Parameters
    ----------
    task : str
        The task to provide implementation guidance for.
    """
    impl_think = g.think(
        "impl_think",
        template="You are an implementation expert. For this task: {task}\n\n"
        "Provide:\n"
        "1. Implementation approach\n"
        "2. Key algorithms or patterns\n"
        "3. Testing strategy\n"
        "4. Deployment considerations\n\n"
        "Focus on practical, actionable guidance."
    )

    g.return_("result", source=impl_think)
    


if __name__ == "__main__":
    import json
    print(impl_expert._graph.to_air())
