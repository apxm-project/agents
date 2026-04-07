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

    Demonstrates the new auto-wiring API: {var_name} in templates automatically
    creates data edges from the referenced node.

    Parameters
    ----------
    task : str
        The coding task to implement.
    """
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn the coder agent
    coder = g.spawn("coder", profile="claude", cwd=cwd)

    # Verify task received (compile parameter {task} is NOT auto-wired)
    task_ok = g.ask("Confirm task received: {task}")

    # Three parallel planning perspectives (auto-wired from {task_ok})
    arch = g.think(
        "ultrathink. You are a Rust systems architect for the APXM project at ~/projects/agents/apxm. "
        "Read relevant source files first. Produce: which crates affected, exact file paths to create or modify, "
        "public API design (structs, traits, function signatures), integration with existing systems "
        "(ContextStack, MemoCache, Scheduler, ModelRouter), and architectural risks. Task: {task_ok}"
    )

    adv = g.think(
        "ultrathink. You are an adversarial reviewer. What already exists that should NOT be re-implemented? "
        "Minimal viable change vs over-engineering? Top 3 failure modes? What NOT to build in v1? "
        "The ONE thing that breaks everything? Be brutal. Adversary wins on scope. "
        "Read crates/apxm-runtime/src/model_router/ first. Task: {task_ok}"
    )

    impl_ = g.think(
        "ultrathink. You are a Rust implementation expert. Produce: complete Rust structs and impl blocks "
        "(not pseudocode), unit tests for happy path and error paths, exact Cargo.toml additions. "
        "Build command: dekk apxm build. Read relevant source files first. Task: {task_ok}"
    )

    # Synthesize the three perspectives (auto-wired: {arch}→{0}, {adv}→{1}, {impl_}→{2})
    synthesis = g.think(
        "ultrathink. Synthesize 3 expert analyses into ONE implementation brief. Adversary wins on scope. "
        "Exact file paths + complete Rust code blocks. Test cases. Flag uncertainty with [RISK]. "
        "End with CONSERVATIVE APPROACH and BOLD APPROACH sections. "
        "ARCHITECT: {arch} ADVERSARY: {adv} IMPL EXPERT: {impl_}"
    )

    # Format as coding prompt (auto-wired from {synthesis})
    prompt = g.think(
        "Write a precise coding agent instruction from this synthesis. Conservative approach first "
        "(minimal, safe), bold approach second (full vision). Start with: cd ~/projects/agents/apxm. "
        "List exact files with full content. End with: dekk apxm build, fix errors, run tests, git commit. "
        "Synthesis: {synthesis}"
    )

    # Send to coder (auto-wired from {prompt})
    result = coder.ask("{prompt}")

    # Reflect on results (auto-wired from {result})
    summary = g.think(
        "Reflect: what was built? Did it compile? Which approach won? "
        "Summary for the engineer. Result: {result}"
    )

    # Print and return (auto-wired from {summary})
    g.print("=== ULTRATHINK COMPLETE ===\n{summary}")
    g.done(summary)


if __name__ == "__main__":
    import json
    print(ultrathink_coder._graph.to_air())
