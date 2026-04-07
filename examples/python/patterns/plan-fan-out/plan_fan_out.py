#!/usr/bin/env python3
"""plan_fan_out.py - PLAN decomposes a goal, parallel ASK nodes write each section, THINK assembles

Usage: python3 -m examples.python.patterns.plan-fan-out.plan_fan_out
"""

from apxm.graph import compile, GraphRecorder


@compile()
def plan_then_parallelize(g: GraphRecorder):
    """Plan decomposition, parallel execution, then assembly."""
    # Planning phase
    plan_steps = g.ask(
        "plan_steps",
        "You are a technical content planner. Create a 3-part outline for a blog post "
        "about Rust async programming. Format as:\n"
        "PART 1: [title and 2-sentence summary]\n"
        "PART 2: [title and 2-sentence summary]\n"
        "PART 3: [title and 2-sentence summary]"
    )

    # Parallel execution phase - three sections written independently
    section_concepts = g.ask(
        "section_concepts",
        "Based on this plan:\n{plan_steps}\n\n"
        "Write section 1: Core async concepts in Rust (async/await, Futures, Pin). "
        "Target 250 words, technical but accessible."
    )

    section_tokio = g.ask(
        "section_tokio",
        "Based on this plan:\n{plan_steps}\n\n"
        "Write section 2: Tokio runtime internals (work-stealing scheduler, I/O driver, "
        "task spawning). Target 250 words, technical depth."
    )

    section_pitfalls = g.ask(
        "section_pitfalls",
        "Based on this plan:\n{plan_steps}\n\n"
        "Write section 3: Common pitfalls and patterns (blocking in async, cancellation, "
        "select!, join!). Target 250 words with code examples."
    )

    # Assembly phase - merge all sections
    assemble = g.think(
        "assemble",
        "You have three independently written sections for a blog post about Rust async:\n\n"
        "SECTION 1:\n{section_concepts}\n\nSECTION 2:\n{section_tokio}\n\nSECTION 3:\n{section_pitfalls}\n\n"
        "Assemble into a polished, cohesive blog post. Add an intro paragraph and a conclusion. "
        "Fix any inconsistencies between sections. Output the complete post."
    )

    # Print output
    output = g.print("=== ASSEMBLED BLOG POST ===\n{assemble}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(plan_then_parallelize._graph.to_air())
