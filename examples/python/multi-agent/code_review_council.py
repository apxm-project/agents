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
        "You are a security reviewer. Review this code for vulnerabilities:\n\n{code}\n\n"
        "Identify all security issues with severity ratings."
    )

    perf_review = g.ask(
        "perf_review",
        "You are a performance reviewer. Review this code for efficiency:\n\n{code}\n\n"
        "Identify performance bottlenecks and optimization opportunities."
    )

    maint_review = g.ask(
        "maint_review",
        "You are a maintainability reviewer. Review this code for clarity:\n\n{code}\n\n"
        "Assess readability, structure, and long-term maintainability."
    )

    # Synthesize verdict
    verdict = g.think(
        "verdict",
        "Three reviewers assessed this code:\n\n"
        "Security:\n{0}\n\nPerformance:\n{1}\n\nMaintainability:\n{2}\n\n"
        "Synthesize a final verdict with priority issues and concrete fixes:"
    )
    security_review | verdict
    perf_review | verdict
    maint_review | verdict

    # Print and return
    output = g.print_("output", message="{0}")
    verdict | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(json.dumps(code_review_council._graph.to_dict(), indent=2))
