#!/usr/bin/env python3
"""memo_cache_stress.py - Benchmark for Memoization Cache Effectiveness

Tests: MemoCache optimization (runtime result caching)
Measures: Cache hit rate for repeated identical prompts

Graph structure: 3 prompts executed twice each (6 total executions)
- First run (cold cache): 3 LLM calls (cache misses)
- Second run (same prompts): 0 LLM calls (cache hits)
- Overall: 6 executions → 3 LLM calls → 50% cache hit rate

Metrics:
- Total LLM calls (should be 3, not 6)
- Cache hit rate (should be 50% - 3 hits, 3 misses)
- Time savings from cache hits
- Average cache lookup latency vs LLM call latency

Note: This tests RUNTIME caching within a single execution.
For cross-execution caching, see the persistent KV-cache benchmarks.

Usage:
  dekk apxm execute memo_cache_stress.air -O0  # No caching (6 LLM calls)
  dekk apxm execute memo_cache_stress.air -O2  # With MemoCache (3 LLM calls)
"""

from apxm import compile, GraphRecorder


@compile()
def memo_cache_stress(g: GraphRecorder):
    """Execute the same 3 prompts twice to test memoization cache."""

    # Define 3 distinct prompts
    # Each will be executed TWICE in this workflow

    # ============================================================
    # FIRST EXECUTION (Cold cache - all misses)
    # ============================================================

    prompt_a_v1 = g.ask(
        name="prompt_a_first",
        prompt="What is the capital of Japan? Answer in one word."
    )

    prompt_b_v1 = g.ask(
        name="prompt_b_first",
        prompt="What is 15 + 27? Answer with just the number."
    )

    prompt_c_v1 = g.ask(
        name="prompt_c_first",
        prompt="Name a primary color. Answer in one word."
    )

    # ============================================================
    # SECOND EXECUTION (Warm cache - all hits)
    # ============================================================
    # These are IDENTICAL prompts to the first execution
    # MemoCache should detect this and return cached results

    prompt_a_v2 = g.ask(
        name="prompt_a_second",
        prompt="What is the capital of Japan? Answer in one word."
    )

    prompt_b_v2 = g.ask(
        name="prompt_b_second",
        prompt="What is 15 + 27? Answer with just the number."
    )

    prompt_c_v2 = g.ask(
        name="prompt_c_second",
        prompt="Name a primary color. Answer in one word."
    )

    # ============================================================
    # VERIFICATION
    # ============================================================
    # Compare first vs second execution results
    # They should be IDENTICAL (same LLM response from cache)

    # Auto-wire: each {var_name} resolves the local NodeRef and creates
    # the Data edge. No need for explicit g.add_edge() calls.
    comparison_a = g.think(
        name="compare_a",
        prompt="First: {prompt_a_v1}\nSecond: {prompt_a_v2}\n\n"
        "Are these two responses identical? Answer yes or no."
    )

    comparison_b = g.think(
        name="compare_b",
        prompt="First: {prompt_b_v1}\nSecond: {prompt_b_v2}\n\n"
        "Are these two responses identical? Answer yes or no."
    )

    comparison_c = g.think(
        name="compare_c",
        prompt="First: {prompt_c_v1}\nSecond: {prompt_c_v2}\n\n"
        "Are these two responses identical? Answer yes or no."
    )

    # ============================================================
    # OUTPUT
    # ============================================================

    # Merge all results
    all_results = g.merge(
        "all_results",
        prompt_a_v1, prompt_a_v2, comparison_a,
        prompt_b_v1, prompt_b_v2, comparison_b,
        prompt_c_v1, prompt_c_v2, comparison_c
    )

    output = g.print(
        message="=== MEMO CACHE STRESS TEST RESULT ===\n\n"
        "Prompt A (Japan capital):\n"
        "  First: {prompt_a_v1}\n"
        "  Second: {prompt_a_v2}\n"
        "  Match: {comparison_a}\n\n"
        "Prompt B (15 + 27):\n"
        "  First: {prompt_b_v1}\n"
        "  Second: {prompt_b_v2}\n"
        "  Match: {comparison_b}\n\n"
        "Prompt C (Primary color):\n"
        "  First: {prompt_c_v1}\n"
        "  Second: {prompt_c_v2}\n"
        "  Match: {comparison_c}\n\n"
        "---\n"
        "This workflow executed 3 prompts TWICE (6 total executions).\n\n"
        "O0 (no caching): 6 LLM calls (100% misses)\n"
        "O2 (with MemoCache): 3 LLM calls (50% cache hit rate)\n\n"
        "Expected metrics:\n"
        "- Total LLM calls: 3 (not 6)\n"
        "- Cache hits: 3\n"
        "- Cache misses: 3\n"
        "- Cache hit rate: 50%\n"
        "- Time savings: ~3x LLM call latency"
    )
    # Control edge keeps the merge as a synchronization barrier without
    # adding an extra Data input to print's input_names.
    g.add_edge(all_results, output, dependency="Control")

    g.done(output)


if __name__ == "__main__":
    # Output the graph as JSON
    print(memo_cache_stress._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(memo_cache_stress())
    # print(result.content)
