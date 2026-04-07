#!/usr/bin/env python3
"""code_review_council.py - Parallel code review council

Usage: python3 -m examples.python.multi-agent.code_review_council
"""

from apxm.graph import compile, GraphRecorder


@compile()
def code_review_council(g: GraphRecorder, code: str):
    """Multi-reviewer council for comprehensive code review.

    Parameters
    ----------
    code : str
        Code to review.
    """
    # Parallel reviews from three perspectives
    security_review = g.ask(
        "security_review",
        template="You are a security reviewer. Review this code for vulnerabilities:\n\n{code}\n\n"
        "Identify all security issues with severity ratings."
    )

    perf_review = g.ask(
        "perf_review",
        template="You are a performance reviewer. Review this code for efficiency:\n\n{code}\n\n"
        "Identify performance bottlenecks and optimization opportunities."
    )

    maint_review = g.ask(
        "maint_review",
        template="You are a maintainability reviewer. Review this code for clarity:\n\n{code}\n\n"
        "Assess readability, structure, and long-term maintainability."
    )

    # Synthesize verdict
    verdict = g.think(
        "verdict",
        template="Three reviewers assessed this code:\n\n"
        "Security:\n{security_review}\n\nPerformance:\n{perf_review}\n\nMaintainability:\n{maint_review}\n\n"
        "Synthesize a final verdict with priority issues and concrete fixes:"
    )

    # Print and return
    output = g.print("{verdict}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(code_review_council._graph.to_air())
