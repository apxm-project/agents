#!/usr/bin/env python3
"""fan_out_synthesize.py -- Plan, fan-out in parallel, synthesize

A planner decomposes a goal into sections. Three writers execute in
parallel (no data dependencies between them), then an assembler merges
the results.

O0: sections run sequentially (~3x slower).
O2: compiler detects independent nodes and schedules them concurrently.

Usage: dekk apxm execute examples/python/parallelism/fan_out_synthesize.py
"""

from apxm import compile, GraphRecorder


@compile()
def fan_out_synthesize(g: GraphRecorder):
    """Plan decomposition, parallel fan-out, then synthesis."""

    # Planning phase
    plan = g.ask(
        "plan",
        "Create a 3-part outline for a blog post about Rust async programming. "
        "Format as:\nPART 1: [title]\nPART 2: [title]\nPART 3: [title]"
    )

    # PARALLEL: these 3 nodes share no data deps between them --
    # the compiler schedules them concurrently
    section_1 = g.ask(
        "section_concepts",
        "Based on this plan:\n{plan}\n\n"
        "Write section 1: Core async concepts (async/await, Futures, Pin). 250 words."
    )

    section_2 = g.ask(
        "section_tokio",
        "Based on this plan:\n{plan}\n\n"
        "Write section 2: Tokio runtime internals (work-stealing, I/O driver). 250 words."
    )

    section_3 = g.ask(
        "section_pitfalls",
        "Based on this plan:\n{plan}\n\n"
        "Write section 3: Common pitfalls (blocking in async, cancellation). 250 words."
    )

    # SEQUENTIAL: assembly waits for all 3 sections -- DAG enforces barrier
    assemble = g.think(
        "assemble",
        "Assemble these independently-written sections into a polished blog post:\n\n"
        "SECTION 1:\n{section_concepts}\n\n"
        "SECTION 2:\n{section_tokio}\n\n"
        "SECTION 3:\n{section_pitfalls}\n\n"
        "Add intro and conclusion. Fix inconsistencies."
    )

    output = g.print("=== BLOG POST ===\n{assemble}")
    g.done(output)


if __name__ == "__main__":
    import asyncio

    result = asyncio.run(fan_out_synthesize())
    print(result.content)
