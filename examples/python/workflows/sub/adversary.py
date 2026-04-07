#!/usr/bin/env python3
"""adversary.py - Adversarial review workflow

Finds what will go wrong and what is over-engineered.

Usage: python3 -m examples.python.workflows.sub.adversary
"""

from apxm.graph import compile, GraphRecorder


@compile()
def adversary(g: GraphRecorder, task: str):
    """Adversarial review to identify risks and over-engineering.

    Parameters
    ----------
    task : str
        The task to review adversarially.
    """
    adversary_reason = g.reason(
        "adversary_reason",
        template="ultrathink. You are an adversarial reviewer. Your job is to find what will go wrong "
        "and what is over-engineered. For this task:\n"
        "- What already exists that should NOT be re-implemented?\n"
        "- What is the minimal viable change vs over-engineering?\n"
        "- Top 3 failure modes?\n"
        "- What should NOT be built in v1?\n"
        "- The ONE thing that breaks everything if done wrong?\n\n"
        "Be brutal. Adversary wins on scope disputes. Task: {task}"
    )

    g.return_("result", source=adversary_reason)
    


if __name__ == "__main__":
    import json
    print(adversary._graph.to_air())
