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
        template="You are a technical content planner. Create a 3-part outline for a blog post "
        "about Rust async programming. Format as:\n"
        "PART 1: [title and 2-sentence summary]\n"
        "PART 2: [title and 2-sentence summary]\n"
        "PART 3: [title and 2-sentence summary]"
    )

    # Parallel execution phase - three sections written independently
    section_concepts = g.ask(
        "section_concepts",
        template="Based on this plan:\n{0}\n\n"
        "Write section 1: Core async concepts in Rust (async/await, Futures, Pin). "
        "Target 250 words, technical but accessible."
    )
    plan_steps | section_concepts

    section_tokio = g.ask(
        "section_tokio",
        template="Based on this plan:\n{0}\n\n"
        "Write section 2: Tokio runtime internals (work-stealing scheduler, I/O driver, "
        "task spawning). Target 250 words, technical depth."
    )
    plan_steps | section_tokio

    section_pitfalls = g.ask(
        "section_pitfalls",
        template="Based on this plan:\n{0}\n\n"
        "Write section 3: Common pitfalls and patterns (blocking in async, cancellation, "
        "select!, join!). Target 250 words with code examples."
    )
    plan_steps | section_pitfalls

    # Assembly phase - merge all sections
    assemble = g.think(
        "assemble",
        template="You have three independently written sections for a blog post about Rust async:\n\n"
        "SECTION 1:\n{0}\n\nSECTION 2:\n{1}\n\nSECTION 3:\n{2}\n\n"
        "Assemble into a polished, cohesive blog post. Add an intro paragraph and a conclusion. "
        "Fix any inconsistencies between sections. Output the complete post."
    )
    section_concepts | assemble
    section_tokio | assemble
    section_pitfalls | assemble

    # Print output
    output = g.print_("final_post", message="=== ASSEMBLED BLOG POST ===\n{0}")
    assemble | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(plan_then_parallelize._graph.to_air())
