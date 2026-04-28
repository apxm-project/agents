#!/usr/bin/env python3
"""cse_stress.py - Explicit-pass CSE stress source

Tests: generic CSE only when requested through an explicit pass list.
Measures: whether duplicate deterministic prompts can be folded in a controlled
ablation. Generic CSE is not part of default O-level pipelines.

Graph structure: Same prompt "Analyze {topic}" fed to 3 different downstream paths
- O0: 3 identical LLM calls (no deduplication)
- explicit CSE pass: may reduce to 1 LLM call + result reuse when semantics allow

Metrics:
- Total unique LLM calls
- Total execution time
- Metrics proving whether result reuse occurred

Usage:
  dekk apxm execute examples/python/benchmarks/stress/cse_stress.py -O0
  dekk apxm compile examples/python/benchmarks/stress/cse_stress.py \
    --pass-list normalize,build-prompt,cse,canonicalizer
"""

from apxm import compile, GraphRecorder


@compile()
def cse_stress(g: GraphRecorder):
    """Three branches with identical ASK prompts for explicit CSE testing."""

    # Define a topic constant
    topic = "microservices architecture patterns"

    # The SAME prompt executed three times in parallel

    analysis_1 = g.ask(
        name="analysis_security",
        prompt=f"Analyze {topic} from a SECURITY perspective. "
        "What are the key security considerations? "
        "Provide 3-4 sentences."
    )

    analysis_2 = g.ask(
        name="analysis_performance",
        prompt=f"Analyze {topic} from a SECURITY perspective. "
        "What are the key security considerations? "
        "Provide 3-4 sentences."
    )

    analysis_3 = g.ask(
        name="analysis_scalability",
        prompt=f"Analyze {topic} from a SECURITY perspective. "
        "What are the key security considerations? "
        "Provide 3-4 sentences."
    )

    # Each path processes the (identical) result differently

    # Path 1: Extract bullet points (auto-wired via {analysis_1})
    bullets = g.think(
        name="extract_bullets",
        prompt="{analysis_1}\n\nConvert this analysis into 3 bullet points."
    )

    # Path 2: Create a summary
    summary = g.think(
        name="create_summary",
        prompt="{analysis_2}\n\nSummarize this analysis in one sentence."
    )

    # Path 3: Identify action items
    actions = g.think(
        name="identify_actions",
        prompt="{analysis_3}\n\nBased on this analysis, list 2 recommended action items."
    )

    # Merge all results
    final_report = g.merge("final_report", bullets, summary, actions)

    # Final output (auto-wires bullets/summary/actions via {var} refs).
    output = g.print(
        message="=== CSE STRESS TEST RESULT ===\n\n"
        "Bullet Points:\n{bullets}\n\n"
        "Summary:\n{summary}\n\n"
        "Action Items:\n{actions}\n\n"
        "This workflow executed the SAME prompt 3 times in parallel.\n"
        "O0: 3 separate LLM calls (no deduplication)\n"
        "Explicit CSE pass: reuse must be verified from emitted metrics"
    )
    # Control edge keeps the merge node as a synchronization barrier.
    g.add_edge(final_report, output, dependency="Control")

    g.done(output)


if __name__ == "__main__":
    # Output AIR.
    print(cse_stress._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(cse_stress())
    # print(result.content)
