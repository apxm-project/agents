#!/usr/bin/env python3
"""codex_claude_fix.py - Codex analyzes, Claude fixes

Codex analyzes APXM compiler deeply, Claude implements all fixes.
Both work in the same repo. Codex reads and reports, Claude writes and commits.

Usage: python3 -m examples.python.workflows.codex_claude_fix
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def codex_claude_fix(g: GraphRecorder):
    """Codex analysis followed by Claude fixes."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents
    codex_analyst = g.spawn("codex_analyst", profile="codex", cwd=cwd)
    claude_fixer = g.spawn("claude_fixer", profile="claude", cwd=cwd)

    # Codex deep analysis
    codex_analyst.ask(
        "You are a Rust compiler engineer. Do a FULL deep analysis of the APXM compiler. "
        "Read ALL these files carefully:\n\n"
        "crates/apxm-graph/src/lower_mlir.rs\n"
        "crates/apxm-graph/src/semantic.rs\n"
        "crates/apxm-graph/src/validate.rs\n"
        "crates/apxm-graph/src/optimize.rs\n"
        "crates/apxm-compiler/src/passes/mod.rs\n"
        "crates/apxm-ais/src/operations/definitions.rs\n"
        "crates/apxm-core/src/error/codes.rs\n"
        "crates/apxm-runtime/src/executor/handlers/llm.rs\n"
        "crates/apxm-runtime/src/model_router/mod.rs\n\n"
        "Also run: dekk apxm test\n\n"
        "Report every issue you find: compile warnings, test failures, missing error handling, "
        "TODO/unimplemented sections, performance issues. Be exhaustive with file:line references."
    )

    # Claude fixes everything
    claude_prompt = g.ask(
        "build_claude_prompt",
        template="You are a senior Rust engineer. Codex has done a deep analysis of the APXM compiler and "
        "found issues. Fix ALL of them.\n\nCODEX REPORT:\n{0}\n\n"
        "For each issue:\n"
        "1. Read the relevant file(s)\n"
        "2. Make the minimal correct fix\n"
        "3. Build: dekk apxm build (NEVER bare cargo build)\n"
        "4. Fix any new errors introduced\n"
        "5. Verify: dekk apxm test (must pass)\n\n"
        "When ALL issues are fixed, commit:\n"
        "git add -A && git commit -m 'fix(compiler): comprehensive fixes from codex+claude analysis'"
    )
    codex_analyst.get_last_node() | claude_prompt

    claude_result = g.communicate("claude_result", target_agent="claude_fixer", message="{0}")
    claude_prompt | claude_result

    # Summarize
    summary = g.think(
        "summary",
        template="Summarize what codex found and what claude fixed:\n\n"
        "CODEX REPORT:\n{0}\n\nCLAUDE RESULT:\n{1}"
    )
    codex_analyst.get_last_node() | summary
    claude_result | summary

    # Print and return
    output = g.print_("output", message="=== CODEX + CLAUDE ANALYSIS COMPLETE ===\n{0}")
    summary | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(codex_claude_fix._graph.to_air())
