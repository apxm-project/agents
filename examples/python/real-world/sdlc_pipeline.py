#!/usr/bin/env python3
"""sdlc_pipeline.py -- Full SDLC pipeline: design, implement, review

Claude (architect) designs the feature, Codex (coder) implements it,
Claude (reviewer) verifies the result. Each stage receives the previous
output as context, enforced by the compiler's data dependency analysis.

Usage: dekk apxm execute examples/python/real-world/sdlc_pipeline.py
"""

from apxm import GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude, codex


@compile()
def sdlc_pipeline(g: GraphRecorder):
    """Three-stage SDLC: architect designs, coder implements, reviewer verifies."""
    cwd = agent_cwd()

    # PARALLEL: agent spawns have no data deps -- compiler launches concurrently
    architect = g.spawn("architect", profile=claude, cwd=cwd)
    coder = g.spawn("coder", profile=codex, cwd=cwd)

    # Stage 1: Architect designs
    design = architect.ask(
        "You are the architect for the APXM project. Design a CHECKPOINT operation "
        "for the AIS instruction set. A CHECKPOINT saves execution state so a graph "
        "can be resumed later. Provide: (1) required attributes, (2) runtime handler "
        "behavior, (3) suggested wire index. Be structured and concise, under 250 words."
    )

    print1 = g.print(message="=== ARCHITECT DESIGN ===\n{design}")

    # Stage 2: Coder implements
    # SEQUENTIAL: coder waits for architect's design -- data dependency enforced
    implement_prompt = g.ask(
        name="build_implement_prompt",
        prompt="Based on this design spec, write the stub implementation:\n"
        "1. Rust handler: crates/runtime/apxm-runtime/src/executor/handlers/checkpoint.rs\n"
        "2. Dispatcher match arm in dispatcher.rs\n"
        "Keep it compilable. Follow existing handler patterns.\n\n"
        "Design spec:\n{design}"
    )
    g.add_edge(print1, implement_prompt, dependency="Control")

    coder_result = g.communicate(
        target_agent="coder",
        message="{implement_prompt}"
    )

    print2 = g.print(message="=== CODER IMPLEMENTATION ===\n{coder_result}")

    # Stage 3: Architect reviews
    # SEQUENTIAL: review waits for implementation -- data dependency enforced
    review_prompt = g.ask(
        name="build_review_prompt",
        prompt="Review Codex's stub implementation for the CHECKPOINT op you designed. "
        "Does it match your spec? What's correct, what needs fixing?\n\n"
        "Implementation:\n{coder_result}"
    )
    g.add_edge(print2, review_prompt, dependency="Control")

    final_review = architect.ask("{review_prompt}")

    print3 = g.print(message="=== ARCHITECT REVIEW ===\n{final_review}")
    g.done(print3)


if __name__ == "__main__":
    import apxm

    result = apxm.run(sdlc_pipeline())
    print(result.content)
