#!/usr/bin/env python3
"""remove_op.py - Remove AIS Operation

Safely removes an operation from the full stack: compiler, runtime, and tests.

Graph structure:
- think: analyze impact — what uses this op
- spawn compiler_dev (claude) — removes from enum, MLIR, wire index
- spawn runtime_dev (codex) — removes handler, dispatcher, tests
- spawn verifier (claude) — runs build + tests to ensure nothing broke

Usage:
    PYTHONPATH=crates/compiler/apxm-frontend/python python3 examples/python/self-hosted/remove_op.py > /tmp/remove_op.air
    dekk apxm compile /tmp/remove_op.air -o /tmp/remove_op.apxmobj
    dekk apxm execute /tmp/remove_op.air "OBSOLETE_OP"
"""

import os
from apxm import compile, GraphRecorder


@compile()
def remove_op_workflow(g: GraphRecorder):
    """Remove an AIS operation from APXM.

    Parameters:
        op_name (str): Name of the operation to remove (e.g., "OBSOLETE_OP")
    """
    g.param("op_name", "str")

    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Step 1: Analyze impact
    impact_analysis = g.think(
        name="impact_analysis",
        prompt="""Analyze the impact of removing operation: {op_name}

Check:
1. Where is this operation defined?
   - crates/core/apxm-ais/src/definitions.rs (enum variant)
   - crates/compiler/apxm-compiler/mlir/AISOps.td (TableGen def)
   - crates/compiler/apxm-compiler/src/lower/ArtifactEmitter.cpp (lowering)
   - crates/runtime/apxm-runtime/src/executor/handlers/<op>.rs (handler)
   - crates/runtime/apxm-runtime/src/executor/mod.rs (dispatcher)

2. What code uses this operation?
   - Search examples/ for usage
   - Search tests/ for test cases
   - Check if any built-in workflows depend on it

3. Wire index collision risk:
   - What's its wire index?
   - Can we mark it as reserved/deprecated instead of removing?

Output a structured removal plan:
- Files to modify
- Lines to delete
- Wire index handling strategy (remove vs deprecate)
- Example/test files to update
- Estimated risk (low/medium/high)
"""
    )

    print1 = g.print(name="print_analysis", message="=== IMPACT ANALYSIS ===\n{impact_analysis}")
    g.add_edge(impact_analysis, print1)

    # Spawn agents
    compiler_dev = g.spawn("compiler_dev", profile="claude", cwd=cwd)
    runtime_dev = g.spawn("runtime_dev", profile="codex", cwd=cwd)
    verifier = g.spawn("verifier", profile="claude", cwd=cwd)

    # Step 2: Build removal prompts
    compiler_task = g.ask(
        name="build_compiler_task",
        prompt="""Remove operation from the compiler:

Analysis: {impact_analysis}

Remove from:
1. crates/core/apxm-ais/src/definitions.rs
   - Remove enum variant
   - Remove from from_wire_index() match (or add a comment "// Reserved: <wire_index>")

2. crates/compiler/apxm-compiler/mlir/AISOps.td
   - Remove def AIS_<OpName>Op block
   - Add comment if wire index is reserved

3. crates/compiler/apxm-compiler/src/lower/ArtifactEmitter.cpp
   - Remove case from emitAISOperation() switch

Be careful:
- Don't accidentally remove similar-named operations
- Preserve wire index comments for protocol stability
- Update any operation count constants if they exist
"""
    )
    g.add_edge(impact_analysis, compiler_task)
    g.add_edge(print1, compiler_task, dependency="Control")

    runtime_task = g.ask(
        name="build_runtime_task",
        prompt="""Remove operation from the runtime:

Analysis: {impact_analysis}

Remove from:
1. crates/runtime/apxm-runtime/src/executor/handlers/<op>.rs
   - Delete the entire file

2. crates/runtime/apxm-runtime/src/executor/mod.rs
   - Remove mod <op> declaration
   - Remove match arm from execute_node()

3. Remove test files:
   - Any tests in handlers/<op>.rs tests module
   - Any integration tests that use this operation

Be thorough but careful — don't break adjacent code.
"""
    )
    g.add_edge(impact_analysis, runtime_task)
    g.add_edge(print1, runtime_task, dependency="Control")

    # Step 3: Both devs work in parallel
    compiler_dev.ask("{compiler_task}")
    g.add_edge(compiler_task, compiler_dev.get_last_node())

    runtime_dev.ask("{runtime_task}")
    g.add_edge(runtime_task, runtime_dev.get_last_node())

    print2 = g.print(name="print_compiler_removal", message="=== COMPILER REMOVAL ===\n{compiler_dev}")
    g.add_edge(compiler_dev.get_last_node(), print2)

    print3 = g.print(name="print_runtime_removal", message="=== RUNTIME REMOVAL ===\n{runtime_dev}")
    g.add_edge(runtime_dev.get_last_node(), print3)

    # Step 4: Verify nothing broke
    wait = g.wait_all(name="wait_removals", compiler_dev.get_last_node(), runtime_dev.get_last_node())
    g.add_edge(print2, wait, dependency="Control")
    g.add_edge(print3, wait, dependency="Control")

    verify_task = g.ask(
        name="build_verify_task",
        prompt="""Verify the removal was clean:

Compiler changes: {compiler_dev}
Runtime changes: {runtime_dev}

Run the verification steps:

1. Build the project:
   dekk apxm build

2. Run all tests:
   cargo test

3. Run autofix to catch any missed references:
   python3 scripts/apxm-autofix.py

4. Check for lingering references:
   grep -r "<op_name>" crates/ examples/

Report:
- Build status (success/failure)
- Test results (passed/failed, which tests)
- Any lingering references found
- Whether the removal is complete

If there are failures, identify what was missed and suggest fixes.
"""
    )
    g.add_edge(compiler_dev.get_last_node(), verify_task)
    g.add_edge(runtime_dev.get_last_node(), verify_task)
    g.add_edge(wait, verify_task, dependency="Control")

    verifier.ask("{verify_task}")
    g.add_edge(verify_task, verifier.get_last_node())

    print4 = g.print(name="print_verification", message="=== VERIFICATION ===\n{verifier}")
    g.add_edge(verifier.get_last_node(), print4)

    # Final summary
    final = g.think(
        name="removal_summary",
        prompt="""Generate removal summary:

Impact analysis: {impact_analysis}
Compiler changes: {compiler_dev}
Runtime changes: {runtime_dev}
Verification: {verifier}

Summary:
- Operation removed: <name>
- Wire index: <number> (removed/deprecated)
- Files modified: <list>
- Build status: <success/failure>
- Test status: <passed/failed>
- Remaining issues: <none/list>
- Status: <complete/needs-fixes>
"""
    )
    g.add_edge(impact_analysis, final)
    g.add_edge(compiler_dev.get_last_node(), final)
    g.add_edge(runtime_dev.get_last_node(), final)
    g.add_edge(verifier.get_last_node(), final)
    g.add_edge(print4, final, dependency="Control")

    print5 = g.print(name="print_summary", message="=== REMOVAL SUMMARY ===\n{final}")
    g.add_edge(final, print5)

    g.done(print5)


if __name__ == "__main__":
    print(remove_op_workflow._graph.to_air())
