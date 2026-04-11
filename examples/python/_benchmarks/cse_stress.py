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
  dekk apxm execute cse_stress.apxm -O0  # No CSE (3 identical LLM calls)
  dekk apxm execute cse_stress.apxm -O2  # With CSE (1 LLM call, reused 3x)
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
        "analysis_security",
        f"Analyze {topic} from a SECURITY perspective. "
        "What are the key security considerations? "
        "Provide 3-4 sentences."
    )

    analysis_2 = g.ask(
        "analysis_performance",
        f"Analyze {topic} from a SECURITY perspective. "
        "What are the key security considerations? "
        "Provide 3-4 sentences."
    )

    analysis_3 = g.ask(
        "analysis_scalability",
        f"Analyze {topic} from a SECURITY perspective. "
        "What are the key security considerations? "
        "Provide 3-4 sentences."
    )

    # Each path processes the (identical) result differently

    # Path 1: Extract bullet points
    bullets = g.think(
        "extract_bullets",
        "{0}\n\nConvert this analysis into 3 bullet points."
    )
    analysis_1 | bullets

    # Path 2: Create a summary
    summary = g.think(
        "create_summary",
        "{0}\n\nSummarize this analysis in one sentence."
    )
    analysis_2 | summary

    # Path 3: Identify action items
    actions = g.think(
        "identify_actions",
        "{0}\n\nBased on this analysis, list 2 recommended action items."
    )
    analysis_3 | actions

    # Merge all results
    final_report = g.merge("final_report", bullets, summary, actions)

    # Final output
    output = g.print(
        "=== CSE STRESS TEST RESULT ===\n\n"
        "Bullet Points:\n{0}\n\n"
        "Summary:\n{1}\n\n"
        "Action Items:\n{2}\n\n"
        "This workflow executed the SAME prompt 3 times in parallel.\n"
        "O0: 3 separate LLM calls (no deduplication)\n"
        "O2 with CSE: 1 LLM call, result reused 3 times"
    )
    final_report | output

    g.done(output)


if __name__ == "__main__":
    # Output the graph as JSON
    print(cse_stress._graph.to_air())
