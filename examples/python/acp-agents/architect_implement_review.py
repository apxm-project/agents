#!/usr/bin/env python3
"""architect_implement_review.py - Full SDLC in 3 stages

Claude (architect) designs CHECKPOINT op, Codex writes the stub implementation,
Claude reviews it. Each stage receives the previous output as context.

Usage: python3 -m examples.python.acp-agents.architect_implement_review
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude, codex
import os


@compile()
def architect_implement_review(g: GraphRecorder):
    """Three-stage SDLC workflow: design, implement, review."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents with typed profiles
    architect = g.spawn("architect", profile=claude, cwd=cwd)
    coder = g.spawn("coder", profile=codex, cwd=cwd)

    # Stage 1: Architect designs
    design_prompt = (
        "You are the architect for the APXM project. Design a CHECKPOINT operation for the "
        "AIS instruction set. A CHECKPOINT allows a running graph to save its current execution "
        "state so it can be resumed later (useful for long-running agent workflows). Provide: "
        "(1) MLIR mnemonic, (2) required and optional node attributes, (3) what the runtime handler "
        "should do at a high level, (4) suggested wire index (next available after 37). Be structured "
        "and concise, under 250 words."
    )
    architect.ask(design_prompt)

    # Get architect's design
    architect_design = architect.get_last_node()

    print1 = g.print("=== ARCHITECT DESIGN ===\n{architect_design}")

    # Stage 2: Coder implements
    implement_prompt = g.ask(
        template="Based on the following design spec, write the stub implementation. Produce exactly two sections:\n"
        "1. Rust handler: crates/apxm-runtime/src/executor/handlers/checkpoint.rs\n"
        "2. TableGen op: the def AIS_CheckpointOp block for AISOps.td\n"
        "Keep it compilable. Base it on how spawn_agent.rs and AISOps.td are structured in this repo.\n\n"
        "Design spec:\n{architect_design}"
    )
    print1 >> implement_prompt

    coder_comm = g.communicate(target_agent="coder", message="{implement_prompt}")

    print2 = g.print("=== CODER IMPLEMENTATION ===\n{coder_comm}")

    # Stage 3: Architect reviews
    review_prompt = g.ask(
        template="Review the following stub implementation from Codex for the CHECKPOINT op you designed. "
        "Does it match your spec? What's correct, what's wrong, and what would you change? Be specific.\n\n"
        "Codex's implementation:\n{coder_comm}"
    )
    print2 >> review_prompt

    architect.ask("{review_prompt}")

    # Get final review
    final_review = architect.get_last_node()

    print3 = g.print("=== ARCHITECT REVIEW ===\n{final_review}")

    g.done(print3)
    


if __name__ == "__main__":
    import json
    print(architect_implement_review._graph.to_air())
