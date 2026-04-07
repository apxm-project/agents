#!/usr/bin/env python3
"""dev_workflow.py - APXM dev workflow: Claude architects, Codex implements + tests, Claude reviews

Usage: python3 -m examples.python.acp-agents.dev_workflow
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def apxm_dev_workflow(g: GraphRecorder):
    """Full development workflow with architect, coder, and reviewer."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents
    architect = g.spawn("architect", profile="claude", cwd=cwd)
    coder = g.spawn("coder", profile="codex", cwd=cwd)

    # Define the task
    task = (
        "Add a CHECKPOINT operation to the AIS instruction set at wire index 38. "
        "Follow the exact pattern of the existing Pause op in: "
        "(1) crates/apxm-ais/src/operations/definitions.rs - add the enum variant, all match arms, and OperationSpec. "
        "(2) crates/apxm-runtime/src/executor/handlers/checkpoint.rs - create stub handler. "
        "(3) crates/apxm-runtime/src/executor/handlers/mod.rs - register the module. "
        "(4) crates/apxm-runtime/src/executor/dispatcher.rs - add match arm and update the count. "
        "Required attribute: checkpoint_id (string). The handler should write to STM and return a token."
    )

    # Architect plans
    architect_prompt = g.ask(
        "build_architect_prompt",
        template="{0}\n\nYou are the lead architect for APXM. Produce a detailed, file-by-file implementation plan for this task."
    )
    task_node = g.const_("task", value=task)
    task_node | architect_prompt

    architect.ask("{0}")
    architect_prompt | architect.get_last_node()

    # Print architect's plan
    print1 = g.print_("print_plan", message="=== ARCHITECT PLAN ===\n{0}")
    architect.get_last_node() | print1

    # Coder implements
    coder_prompt = g.ask(
        "build_coder_prompt",
        template="Implement exactly what the architect planned below. Make the changes in the codebase. "
        "Then run: cargo test --workspace --exclude apxm-compiler 2>&1 | tail -10. "
        "Report what you changed and test results.\n\nPlan:\n{0}"
    )
    architect.get_last_node() | coder_prompt
    print1 >> coder_prompt

    coder.ask("{0}")
    coder_prompt | coder.get_last_node()

    # Print coder's implementation
    print2 = g.print_("print_impl", message="=== CODER IMPLEMENTATION ===\n{0}")
    coder.get_last_node() | print2

    # Architect reviews
    review_prompt = g.ask(
        "build_review_prompt",
        template="Review Codex's implementation. Does it match your plan? Any bugs or missing pieces? "
        "Ship it or iterate?\n\nCodex's report:\n{0}"
    )
    coder.get_last_node() | review_prompt
    print2 >> review_prompt

    architect.ask("{0}")
    review_prompt | architect.get_last_node()

    # Print final review
    print3 = g.print_("print_review", message="=== ARCHITECT REVIEW ===\n{0}")
    architect.get_last_node() | print3

    g.return_("result", source=print3)
    


if __name__ == "__main__":
    import json
    print(apxm_dev_workflow._graph.to_air())
