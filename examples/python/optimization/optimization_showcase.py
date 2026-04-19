#!/usr/bin/env python3
"""optimization_showcase.py -- All compiler optimizations in one workflow

Demonstrates every optimization the APXM compiler can apply:
1. Parallel scheduling -- independent nodes run concurrently
2. Fusion -- adjacent THINK nodes merge into one LLM call
3. Shared prefix reuse -- parallel branches share prompt prefix for KV-cache
4. Dead context elimination -- unused inputs are pruned

Usage:
  dekk apxm execute examples/python/optimization/optimization_showcase.py -O0
  dekk apxm execute examples/python/optimization/optimization_showcase.py -O2
"""

from apxm import compile, GraphRecorder


@compile()
def optimization_showcase(g: GraphRecorder):
    """Showcase: parallel scheduling, fusion, shared prefix, and DCE."""

    topic = g.ask(
        name="topic",
        prompt="What technical topic should we analyze? "
        "(e.g., 'Rust async runtimes', 'distributed consensus')"
    )

    # Fast triage (could use cheap model in production)
    triage = g.ask(
        name="triage",
        prompt="Quick triage of: {topic}\n"
        "- Complexity: [LOW/MEDIUM/HIGH]\n"
        "- Security: [MINIMAL/MODERATE/CRITICAL]\n"
        "- Performance: [LOW/MEDIUM/HIGH]"
    )

    # PARALLEL + SHARED PREFIX: three analyses fan out from triage
    # All share "Based on triage: {triage}" prefix -- KV-cache reuse
    arch = g.think(
        name="architecture",
        prompt="Based on triage: {triage}\n\n"
        "Deep architecture analysis of: {topic}\n"
        "Cover: components, patterns, trade-offs. 300 words."
    )

    security = g.think(
        name="security",
        prompt="Based on triage: {triage}\n\n"
        "Deep security analysis of: {topic}\n"
        "Cover: attack vectors, mitigations, best practices. 300 words."
    )

    perf = g.think(
        name="performance",
        prompt="Based on triage: {triage}\n\n"
        "Deep performance analysis of: {topic}\n"
        "Cover: bottlenecks, optimization strategies. 300 words."
    )

    # Synthesis
    synthesis = g.think(
        name="synthesis",
        prompt="Synthesize into executive summary:\n\n"
        "ARCHITECTURE:\n{arch}\n\n"
        "SECURITY:\n{security}\n\n"
        "PERFORMANCE:\n{perf}\n\n"
        "500-word summary with key insights and recommendations."
    )

    # FUSION CANDIDATES: adjacent THINK nodes (review + validation)
    review = g.think(
        name="review",
        prompt="Review this synthesis:\n{synthesis}\n\n"
        "What's strong? What needs improvement?"
    )

    validation = g.think(
        name="validation",
        prompt="Validate completeness:\n{synthesis}\n\n"
        "Check: coverage, accuracy, actionability."
    )

    report = g.merge("report", synthesis, review, validation)

    output = g.print(
        message="TOPIC: {topic}\n\nSYNTHESIS:\n{synthesis}\n\n"
        "REVIEW:\n{review}\n\nVALIDATION:\n{validation}"
    )
    g.add_edge(output, report, dependency="Control")
    g.done(report)


if __name__ == "__main__":
    import apxm

    result = apxm.run(optimization_showcase())
    print(result.content)
