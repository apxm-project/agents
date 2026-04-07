#!/usr/bin/env python3
"""remove_op.py - Remove AIS Operation

Safely removes an operation from the full stack: compiler, runtime, and tests.

Graph structure:
- think: analyze impact — what uses this op
- spawn compiler_dev (claude) — removes from enum, MLIR, wire index
- spawn runtime_dev (codex) — removes handler, dispatcher, tests
- spawn verifier (claude) — runs build + tests to ensure nothing broke

Usage:
    PYTHONPATH=crates/apxm-frontend/python python3 examples/python/self-hosted/remove_op.py > /tmp/remove_op.air
    dekk apxm compile /tmp/remove_op.air -o /tmp/remove_op.apxmobj
    dekk apxm execute /tmp/remove_op.air "OBSOLETE_OP"
"""

import os
from apxm.graph import compile, GraphRecorder


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
        "impact_analysis",
        template="""Analyze the impact of removing operation: {0}

Check:
1. Where is this operation defined?
   - crates/apxm-ais/src/definitions.rs (enum variant)
   - crates/apxm-compiler/mlir/AISOps.td (TableGen def)
   - crates/apxm-compiler/src/lower/ArtifactEmitter.cpp (lowering)
   - crates/apxm-runtime/src/executor/handlers/<op>.rs (handler)
   - crates/apxm-runtime/src/executor/mod.rs (dispatcher)

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

    print1 = g.print_("print_analysis", message="=== IMPACT ANALYSIS ===\n{0}")
    impact_analysis | print1

    # Spawn agents
    compiler_dev = g.spawn("compiler_dev", profile="claude", cwd=cwd)
    runtime_dev = g.spawn("runtime_dev", profile="codex", cwd=cwd)
    verifier = g.spawn("verifier", profile="claude", cwd=cwd)

    # Step 2: Build removal prompts
    compiler_task = g.ask(
        "build_compiler_task",
        template="""Remove operation from the compiler:

Analysis: {0}

Remove from:
1. crates/apxm-ais/src/definitions.rs
   - Remove enum variant
   - Remove from from_wire_index() match (or add a comment "// Reserved: <wire_index>")

2. crates/apxm-compiler/mlir/AISOps.td
   - Remove def AIS_<OpName>Op block
   - Add comment if wire index is reserved

3. crates/apxm-compiler/src/lower/ArtifactEmitter.cpp
   - Remove case from emitAISOperation() switch

Be careful:
- Don't accidentally remove similar-named operations
- Preserve wire index comments for protocol stability
- Update any operation count constants if they exist
"""
    )
    impact_analysis | compiler_task
    print1 >> compiler_task

    runtime_task = g.ask(
        "build_runtime_task",
        template="""Remove operation from the runtime:

Analysis: {0}

Remove from:
1. crates/apxm-runtime/src/executor/handlers/<op>.rs
   - Delete the entire file

2. crates/apxm-runtime/src/executor/mod.rs
   - Remove mod <op> declaration
   - Remove match arm from execute_node()

3. Remove test files:
   - Any tests in handlers/<op>.rs tests module
   - Any integration tests that use this operation

Be thorough but careful — don't break adjacent code.
"""
    )
    impact_analysis | runtime_task
    print1 >> runtime_task

    # Step 3: Both devs work in parallel
    compiler_dev.ask("{0}")
    compiler_task | compiler_dev.get_last_node()

    runtime_dev.ask("{0}")
    runtime_task | runtime_dev.get_last_node()

    print2 = g.print_("print_compiler_removal", message="=== COMPILER REMOVAL ===\n{0}")
    compiler_dev.get_last_node() | print2

    print3 = g.print_("print_runtime_removal", message="=== RUNTIME REMOVAL ===\n{0}")
    runtime_dev.get_last_node() | print3

    # Step 4: Verify nothing broke
    wait = g.wait_all("wait_removals", compiler_dev.get_last_node(), runtime_dev.get_last_node())
    print2 >> wait
    print3 >> wait

    verify_task = g.ask(
        "build_verify_task",
        template="""Verify the removal was clean:

Compiler changes: {0}
Runtime changes: {1}

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
    compiler_dev.get_last_node() | verify_task
    runtime_dev.get_last_node() | verify_task
    wait >> verify_task

    verifier.ask("{0}")
    verify_task | verifier.get_last_node()

    print4 = g.print_("print_verification", message="=== VERIFICATION ===\n{0}")
    verifier.get_last_node() | print4

    # Final summary
    final = g.think(
        "removal_summary",
        template="""Generate removal summary:

Impact analysis: {0}
Compiler changes: {1}
Runtime changes: {2}
Verification: {3}

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
    impact_analysis | final
    compiler_dev.get_last_node() | final
    runtime_dev.get_last_node() | final
    verifier.get_last_node() | final
    print4 >> final

    print5 = g.print_("print_summary", message="=== REMOVAL SUMMARY ===\n{0}")
    final | print5

    g.return_("result", source=print5)


if __name__ == "__main__":
    print(remove_op_workflow._graph.to_air())
