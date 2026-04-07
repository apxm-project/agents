#!/usr/bin/env python3
"""ultrathink_coder.py - Ultrathink coding workflow

3-way parallel planning + synthesis + implementation.

Usage: python3 -m examples.python.workflows.ultrathink_coder
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def ultrathink_coder(g: GraphRecorder, task: str):
    """Ultrathink coding workflow with parallel analysis and synthesis.

    Parameters
    ----------
    task : str
        The coding task to implement.
    """
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn the coder agent
    coder = g.spawn("coder", profile="claude", cwd=cwd)

    # Verify task received
    task_verified = g.ask("task_verified", "Confirm task received: {task}")

    # Three parallel planning perspectives
    arch_out = g.think(
        "arch_out",
        "ultrathink. You are a Rust systems architect for the APXM project at ~/projects/agents/apxm. "
        "Read relevant source files first. Produce: which crates affected, exact file paths to create or modify, "
        "public API design (structs, traits, function signatures), integration with existing systems "
        "(ContextStack, MemoCache, Scheduler, ModelRouter), and architectural risks. Task: {0}"
    )
    task_verified | arch_out

    adv_out = g.think(
        "adv_out",
        "ultrathink. You are an adversarial reviewer. What already exists that should NOT be re-implemented? "
        "Minimal viable change vs over-engineering? Top 3 failure modes? What NOT to build in v1? "
        "The ONE thing that breaks everything? Be brutal. Adversary wins on scope. "
        "Read crates/apxm-runtime/src/model_router/ first. Task: {0}"
    )
    task_verified | adv_out

    impl_out = g.think(
        "impl_out",
        "ultrathink. You are a Rust implementation expert. Produce: complete Rust structs and impl blocks "
        "(not pseudocode), unit tests for happy path and error paths, exact Cargo.toml additions. "
        "Build command: dekk apxm build. Read relevant source files first. Task: {0}"
    )
    task_verified | impl_out

    # Synthesize the three perspectives
    synthesis = g.think(
        "synthesis",
        "ultrathink. Synthesize 3 expert analyses into ONE implementation brief. Adversary wins on scope. "
        "Exact file paths + complete Rust code blocks. Test cases. Flag uncertainty with [RISK]. "
        "End with CONSERVATIVE APPROACH and BOLD APPROACH sections. "
        "ARCHITECT: {0} ADVERSARY: {1} IMPL EXPERT: {2}"
    )
    arch_out | synthesis
    adv_out | synthesis
    impl_out | synthesis

    # Format as coding prompt
    coding_prompt = g.think(
        "coding_prompt",
        "Write a precise coding agent instruction from this synthesis. Conservative approach first "
        "(minimal, safe), bold approach second (full vision). Start with: cd ~/projects/agents/apxm. "
        "List exact files with full content. End with: dekk apxm build, fix errors, run tests, git commit. "
        "Synthesis: {0}"
    )
    synthesis | coding_prompt

    # Send to coder
    implementation_result = g.communicate(
        "implementation_result",
        target_agent="coder",
        message="{0}"
    )
    coding_prompt | implementation_result

    # Reflect on results
    summary = g.think(
        "summary",
        "Reflect: what was built? Did it compile? Which approach won? "
        "Summary for the engineer. Result: {0}"
    )
    implementation_result | summary

    # Print and return
    output = g.print_("output", message="=== ULTRATHINK COMPLETE ===\n{0}")
    summary | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(ultrathink_coder._graph.to_air())
