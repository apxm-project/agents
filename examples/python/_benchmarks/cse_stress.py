#!/usr/bin/env python3
"""cse_stress.py - Benchmark for Common Subexpression Elimination

Tests: CSE (Common Subexpression Elimination) optimization pass
Measures: Elimination of duplicate LLM calls with identical prompts

Graph structure: Same prompt "Analyze {topic}" fed to 3 different downstream paths
- O0: 3 identical LLM calls (no deduplication)
- O2 with CSE: should reduce to 1 LLM call + result reuse (3→1)

Metrics:
- Total unique LLM calls (should drop from 3 to 1)
- Total execution time (should improve with fewer LLM calls)
- Cache hit rate (CSE uses internal result caching)

Usage:
  dekk apxm execute cse_stress.air -O0  # No CSE (3 identical LLM calls)
  dekk apxm execute cse_stress.air -O2  # With CSE (1 LLM call, reused 3x)
"""

from apxm import compile, GraphRecorder


@compile()
def cse_stress(g: GraphRecorder):
    """Three branches with identical ASK prompts to test CSE optimization."""

    # Define a topic constant
    topic = "microservices architecture patterns"

    # The SAME prompt executed three times in parallel
    # In O0: 3 separate LLM calls
    # In O2 with CSE: 1 LLM call, result shared across all 3 paths

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
        "O2 with CSE: 1 LLM call, result reused 3 times"
    )
    # Control edge keeps the merge node as a synchronization barrier.
    g.add_edge(final_report, output, dependency="Control")

    g.done(output)


if __name__ == "__main__":
    # Output the graph as JSON
    print(cse_stress._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(cse_stress())
    # print(result.content)
