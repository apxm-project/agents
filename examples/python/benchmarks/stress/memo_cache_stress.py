#!/usr/bin/env python3
"""memo_cache_stress.py - Benchmark for Memoization Cache Effectiveness

Tests: runtime memoization under repeated identical prompts.
Measures: emitted cache-hit and cache-miss metrics.

Graph structure: 3 prompts executed twice each (6 total executions)
- First group: expected cache misses
- Second group: eligible for cache hits when runtime memoization is enabled

Metrics:
- Total LLM calls
- Cache hits and misses
- Time delta between cache lookups and LLM calls
- Average cache lookup latency vs LLM call latency

Note: This tests RUNTIME caching within a single execution.

Usage:
  dekk apxm execute examples/python/benchmarks/stress/memo_cache_stress.py -O0
  dekk apxm execute examples/python/benchmarks/stress/memo_cache_stress.py -O2
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
        "Compare emitted cache-hit, cache-miss, LLM-call, and latency metrics."
    )
    # Control edge keeps the merge as a synchronization barrier without
    # adding an extra Data input to print's input_names.
    g.add_edge(all_results, output, dependency="Control")

    g.done(output)


if __name__ == "__main__":
    # Output AIR.
    print(memo_cache_stress._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(memo_cache_stress())
    # print(result.content)
