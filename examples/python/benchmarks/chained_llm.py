#!/usr/bin/env python3
"""chained_llm.py - Benchmark for LLM pipelining optimization

Tests: Pipelining across sequential LLM operations
Measures: Total latency, time-to-first-token, end-to-end completion time

Usage:
  dekk apxm execute chained_llm.apxm -O0  # No pipelining
  dekk apxm execute chained_llm.apxm -O2  # With pipelining optimizations
"""

from apxm import compile, GraphRecorder


@compile()
def chained_llm(g: GraphRecorder):
    """Sequential chain of ASK → THINK → REASON operations."""

    # Initial question - ASK operation
    initial_response = g.ask(
        "initial_response",
        "You are an AI assistant helping with software architecture.\n\n"
        "Question: Design a URL shortening service similar to bit.ly. "
        "What are the key components and how should they interact?\n\n"
        "Provide a high-level architectural overview in 4-5 sentences."
    )

    # Deep analysis - THINK operation
    analysis = g.think(
        "analysis",
        "Given this architectural overview:\n{initial_response}\n\n"
        "Analyze the design in detail:\n"
        "1. What are the scalability challenges?\n"
        "2. How should data be partitioned?\n"
        "3. What caching strategies are needed?\n"
        "4. How to handle conflicts in short URL generation?\n\n"
        "Provide a detailed technical analysis covering these points."
    )

    # Deep reasoning - REASON operation
    final_design = g.reason(
        "final_design",
        "Based on this analysis:\n{analysis}\n\n"
        "Synthesize a complete system design that addresses all concerns.\n"
        "Include:\n"
        "- Database schema\n"
        "- API endpoints\n"
        "- Caching layers\n"
        "- Scaling strategy\n"
        "- Monitoring approach\n\n"
        "Provide a comprehensive technical specification."
    )

    # Format output
    output = g.print(
        "=== CHAINED LLM BENCHMARK RESULT ===\n\n"
        "Initial Response:\n{initial_response}\n\n"
        "---\n\n"
        "Detailed Analysis:\n{analysis}\n\n"
        "---\n\n"
        "Final Design:\n{final_design}\n"
    )

    g.done(output)


if __name__ == "__main__":
    # Output the graph as JSON
    print(chained_llm._graph.to_json())
