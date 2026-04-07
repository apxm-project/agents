#!/usr/bin/env python3
"""refactor.py - Refactor Module

Takes a module/crate name and refactoring goal, spawns agents to analyze,
implement, and verify the refactoring.

Graph structure:
- spawn analyzer (claude) — reads the crate, identifies refactoring opportunities
- spawn implementer (codex) — does the refactoring
- spawn test_runner (claude) — runs tests, fixes breakage
- analyzer → implementer → test_runner → summary

Usage:
    PYTHONPATH=crates/apxm-frontend/python python3 examples/python/self-hosted/refactor.py > /tmp/refactor.air
    dekk apxm compile /tmp/refactor.air -o /tmp/refactor.apxmobj
    dekk apxm execute /tmp/refactor.air "apxm-runtime" "Extract scheduler into its own module"
"""

import os
from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude, codex


@compile()
def refactor_workflow(g: GraphRecorder):
    """Refactor a module or crate.

    Parameters:
        target (str): Module/crate name (e.g., "apxm-runtime" or "crates/apxm-runtime/src/executor/mod.rs")
        goal (str): Refactoring goal (e.g., "Extract scheduler into its own module")
    """
    g.param("target", "str")
    g.param("goal", "str")

    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents
    analyzer = g.spawn("analyzer", profile=claude, cwd=cwd)
    implementer = g.spawn("implementer", profile=codex, cwd=cwd)
    test_runner = g.spawn("test_runner", profile=claude, cwd=cwd)

    # Step 1: Analyzer reads the code and identifies opportunities
    analyzer_task = g.text(value="""You are the code analyzer for APXM. Analyze this refactoring request:

Target: {target}
Goal: {goal}

Read the target code:
- If it's a crate name, read the crate's src/ directory
- If it's a file path, read that file and related files

Produce a refactoring analysis:
1. Current structure (what exists now)
   - Key types, functions, modules
   - Current organization
   - Code smells or anti-patterns

2. Proposed structure (what should exist after refactoring)
   - New module boundaries
   - What moves where
   - What gets renamed
   - New abstractions needed

3. Impact analysis
   - What breaks (imports, tests, examples)
   - Public API changes (breaking vs non-breaking)
   - What needs to be updated

4. Step-by-step refactoring plan
   - Order of changes (to maintain compilability)
   - Which changes can be mechanical (search-replace) vs manual

Keep under 500 words but be specific about file paths and identifiers.
"""
    )

    analyzer.ask("{analyzer_task}")
    analysis = analyzer.get_last_node()

    print1 = g.print("=== REFACTORING ANALYSIS ===\n{analysis}")

    # Step 2: Implementer does the refactoring
    implementer_task = g.ask(
        "build_implementer_task",
        template="""Based on this analysis, implement the refactoring:

Analysis: {analysis}

Follow the step-by-step plan. For each step:
1. Make the change
2. Ensure the code still compiles (cargo check)
3. Move to the next step

Key guidelines:
- Preserve functionality — this is a refactor, not a rewrite
- Update imports wherever needed
- Keep commits small and incremental (if using git)
- Use rust-analyzer or clippy to catch broken references

After all changes:
- Run cargo fmt
- Run cargo clippy
- Report what was changed (file paths, line counts, moved items)

Be methodical. If something doesn't compile, fix it before moving on.
"""
    )
    print1 >> implementer_task

    implementer.ask("{implementer_task}")
    impl_result = implementer.get_last_node()

    print2 = g.print("=== REFACTORING CHANGES ===\n{impl_result}")

    # Step 3: Test runner verifies nothing broke
    test_task = g.ask(
        "build_test_task",
        template="""Verify the refactoring didn't break anything:

Changes: {impl_result}

Run the test suite:

1. Build the project:
   dekk apxm build

2. Run all tests:
   cargo test

3. Run autofix to check examples:
   python3 scripts/apxm-autofix.py

4. Check for warnings:
   cargo clippy -- -D warnings

Report:
- Build status (success/failure)
- Test results (how many passed/failed)
- Which tests failed (if any)
- Clippy warnings (if any)
- Autofix issues (if any)

If there are failures:
1. Analyze what broke
2. Fix the issues
3. Re-run tests
4. Report fixes made

Keep iterating until all tests pass.
"""
    )
    print2 >> test_task

    test_runner.ask("{test_task}")
    test_result = test_runner.get_last_node()

    print3 = g.print("=== TEST RESULTS ===\n{test_result}")

    # Step 4: Summary
    summary = g.think(
        "refactoring_summary",
        template="""Summarize the refactoring:

Analysis: {analysis}
Implementation: {impl_result}
Test results: {test_result}

Summary:
- Target: <what was refactored>
- Goal: <what was the refactoring goal>
- Changes made:
  * Files created: <list>
  * Files modified: <list>
  * Files deleted: <list>
  * Items moved: <list>
- Test status: <all passed / N failures / fixed>
- Breaking changes: <yes/no, details>
- Status: <complete/incomplete/failed>

If status is incomplete or failed, list remaining issues.
"""
    )
    print3 >> summary

    print4 = g.print("=== REFACTORING SUMMARY ===\n{summary}")

    g.done(print4)


if __name__ == "__main__":
    print(refactor_workflow._graph.to_air())
