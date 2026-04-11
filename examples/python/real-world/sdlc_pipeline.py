#!/usr/bin/env python3
"""sdlc_pipeline.py -- Full SDLC pipeline: design, implement, review

Claude (architect) designs the feature, Codex (coder) implements it,
Claude (reviewer) verifies the result. Each stage receives the previous
output as context, enforced by the compiler's data dependency analysis.

Usage: dekk apxm execute examples/python/real-world/sdlc_pipeline.py
"""

from apxm import compile, GraphRecorder
from apxm._generated.agents import claude, codex
import os


@compile()
def sdlc_pipeline(g: GraphRecorder):
    """Three-stage SDLC: architect designs, coder implements, reviewer verifies."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # PARALLEL: agent spawns have no data deps -- compiler launches concurrently
    architect = g.spawn("architect", profile=claude, cwd=cwd)
    coder = g.spawn("coder", profile=codex, cwd=cwd)

    # Stage 1: Architect designs
    architect.ask(
        "You are the architect for the APXM project. Design a CHECKPOINT operation "
        "for the AIS instruction set. A CHECKPOINT saves execution state so a graph "
        "can be resumed later. Provide: (1) required attributes, (2) runtime handler "
        "behavior, (3) suggested wire index. Be structured and concise, under 250 words."
    )
    design = architect.get_last_node()

    print1 = g.print("=== ARCHITECT DESIGN ===\n{design}")

    # Stage 2: Coder implements
    # SEQUENTIAL: coder waits for architect's design -- data dependency enforced
    implement_prompt = g.ask(
        "build_implement_prompt",
        "Based on this design spec, write the stub implementation:\n"
        "1. Rust handler: crates/runtime/apxm-runtime/src/executor/handlers/checkpoint.rs\n"
        "2. Dispatcher match arm in dispatcher.rs\n"
        "Keep it compilable. Follow existing handler patterns.\n\n"
        "Design spec:\n{design}"
    )
    print1 >> implement_prompt

    coder_result = g.communicate(
        target_agent="coder",
        message="{implement_prompt}"
    )

    print2 = g.print("=== CODER IMPLEMENTATION ===\n{coder_result}")

    # Stage 3: Architect reviews
    # SEQUENTIAL: review waits for implementation -- data dependency enforced
    review_prompt = g.ask(
        "build_review_prompt",
        "Review Codex's stub implementation for the CHECKPOINT op you designed. "
        "Does it match your spec? What's correct, what needs fixing?\n\n"
        "Implementation:\n{coder_result}"
    )
    print2 >> review_prompt

    architect.ask("{review_prompt}")
    final_review = architect.get_last_node()

    print3 = g.print("=== ARCHITECT REVIEW ===\n{final_review}")
    g.done(print3)


if __name__ == "__main__":
    print(sdlc_pipeline._graph.to_air())
