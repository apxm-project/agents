#!/usr/bin/env python3
"""shared_prefix.py -- Demonstrate shared-prefix graph hints

A shared system prompt is prepended to 3 parallel specialized queries.
The compiler can emit shared-prefix hints for graph-aware backends. Backend
cache behavior must be verified from emitted metrics.

Usage: dekk agents execute examples/python/optimization/shared_prefix.py
"""

from apxm import DependencyType, compile, GraphRecorder


@compile()
def shared_prefix_demo(g: GraphRecorder):
    """Shared prefix -> 3 parallel queries with backend-agnostic hints."""

    prefix = (
        "You are reviewing an authentication module. It uses bcrypt for "
        "passwords, JWT for sessions, and Redis for token storage.\n\n"
    )

    # 3 parallel queries sharing the same prefix
    security = g.ask(name="security", prompt=prefix + "What are the security risks?")
    performance = g.ask(name="performance", prompt=prefix + "What are the performance bottlenecks?")
    reliability = g.ask(name="reliability", prompt=prefix + "What are the reliability concerns?")

    # Merge all reviews
    report = g.merge("report", security, performance, reliability)

    output = g.print(
        message="Security: {security}\nPerformance: {performance}\nReliability: {reliability}"
    )
    g.add_edge(output, report, dependency=DependencyType.CONTROL)
    g.done(report)


if __name__ == "__main__":
    import apxm

    result = apxm.run(shared_prefix_demo())
    print(result.content)
