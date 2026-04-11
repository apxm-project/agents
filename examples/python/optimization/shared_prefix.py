#!/usr/bin/env python3
"""shared_prefix.py -- Demonstrate shared prefix / KV-cache reuse

A shared system prompt is prepended to 3 parallel specialized queries.
The PromptCanonicalization pass reorders so the shared prefix comes
first, enabling KV-cache reuse across the parallel branches.

Usage: dekk apxm execute examples/python/optimization/shared_prefix.py
"""

from apxm import compile, GraphRecorder


@compile()
def shared_prefix_demo(g: GraphRecorder):
    """Shared prefix -> 3 parallel queries. Compiler enables KV-cache reuse."""

    prefix = (
        "You are reviewing an authentication module. It uses bcrypt for "
        "passwords, JWT for sessions, and Redis for token storage.\n\n"
    )

    # 3 parallel queries sharing the same prefix
    security = g.ask("security", prefix + "What are the security risks?")
    performance = g.ask("performance", prefix + "What are the performance bottlenecks?")
    reliability = g.ask("reliability", prefix + "What are the reliability concerns?")

    # Merge all reviews
    report = g.merge("report", security, performance, reliability)

    output = g.print(
        "Security: {security}\nPerformance: {performance}\nReliability: {reliability}"
    )
    output >> report
    g.done(report)


if __name__ == "__main__":
    print(shared_prefix_demo._graph.to_air())
