#!/usr/bin/env python3
"""code_review_council.py - Parallel code review council

Usage: dekk agents execute examples/python/real-world/code_review_council.py
"""

from apxm import compile, GraphRecorder


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
        prompt="You are a security reviewer. Review this code for vulnerabilities:\n\n{code}\n\n"
        "Identify all security issues with severity ratings."
    )

    perf_review = g.ask(
        prompt="You are a performance reviewer. Review this code for efficiency:\n\n{code}\n\n"
        "Identify performance bottlenecks and optimization opportunities."
    )

    maint_review = g.ask(
        prompt="You are a maintainability reviewer. Review this code for clarity:\n\n{code}\n\n"
        "Assess readability, structure, and long-term maintainability."
    )

    # Synthesize verdict
    verdict = g.think(
        prompt="Three reviewers assessed this code:\n\n"
        "Security:\n{security_review}\n\nPerformance:\n{perf_review}\n\nMaintainability:\n{maint_review}\n\n"
        "Synthesize a final verdict with priority issues and concrete fixes:"
    )

    # Print and return
    output = g.print(message="{verdict}")

    g.done(output)
    


if __name__ == "__main__":
    import apxm

    sample_code = 'def add(a, b): return a + b'
    result = apxm.run(code_review_council(sample_code))
    print(result.content)
