#!/usr/bin/env python3
"""add_op.py - Add New AIS Operation

Orchestrates the full 8-step procedure to add a new operation to APXM.
Uses parallel agents for compiler-side and runtime-side implementation.

Graph structure:
- spawn architect (claude) — reads the project, plans the implementation
- spawn compiler_dev (claude) — implements compiler side (enum, MLIR lowering, AISOps.td)
- spawn runtime_dev (codex) — implements runtime side (handler, dispatcher, tests)
- spawn reviewer (claude) — reviews both implementations, runs tests

Usage:
    PYTHONPATH=crates/compiler/apxm-frontend/python python3 examples/python/self-hosted/add_op.py > /tmp/add_op.air
    dekk apxm compile /tmp/add_op.air -o /tmp/add_op.apxmobj
    dekk apxm execute /tmp/add_op.air "CHECKPOINT" "Save execution state for later resume"
"""

import os
from apxm import compile, GraphRecorder


@compile()
def add_op_workflow(g: GraphRecorder):
    """Add a new AIS operation to APXM.

    Parameters:
        op_name (str): Name of the operation (e.g., "CHECKPOINT")
        op_description (str): What the operation does
    """
    # Add parameters
    g.param("op_name", "str")
    g.param("op_description", "str")

    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn agents
    architect = g.spawn("architect", profile="claude", cwd=cwd)
    compiler_dev = g.spawn("compiler_dev", profile="claude", cwd=cwd)
    runtime_dev = g.spawn("runtime_dev", profile="codex", cwd=cwd)
    reviewer = g.spawn("reviewer", profile="claude", cwd=cwd)

    # Step 1: Architect analyzes and creates implementation plan
    architect.ask("""You are the architect for APXM. You need to create an implementation plan
for adding a new AIS operation to the APXM codebase.

Operation name: {op_name}
Description: {op_description}

Read the following files to understand the pattern:
- crates/core/apxm-ais/src/definitions.rs (AISOperationType enum)
- crates/compiler/apxm-compiler/src/lower/ArtifactEmitter.cpp (MLIR lowering)
- crates/compiler/apxm-compiler/mlir/AISOps.td (TableGen definitions)
- crates/runtime/apxm-runtime/src/executor/handlers/spawn_agent.rs (example handler)
- crates/runtime/apxm-runtime/src/executor/mod.rs (dispatcher)

Create a structured plan with:
1. Wire index (next available after checking definitions.rs)
2. Required and optional node attributes
3. MLIR mnemonic (should be lowercase, e.g., "ais.checkpoint")
4. Handler pseudo-code (what should the runtime do)
5. TableGen definition structure
6. Test strategy

Keep the plan under 400 words but be specific about attribute names and types.
""")

    print1 = g.print(name="print_plan", message="=== ARCHITECT PLAN ===\n{architect}")
    g.add_edge(architect.get_last_node(), print1)

    # Step 2: Build implementation prompts for parallel execution
    compiler_prompt = g.ask(
        name="build_compiler_prompt",
        prompt="""Based on this plan, implement the compiler-side changes:

Plan:
{architect}

You need to modify:
1. crates/core/apxm-ais/src/definitions.rs
   - Add variant to AISOperationType enum
   - Add to from_wire_index() match

2. crates/compiler/apxm-compiler/mlir/AISOps.td
   - Add def AIS_<OpName>Op block following the pattern of other ops

3. crates/compiler/apxm-compiler/src/lower/ArtifactEmitter.cpp
   - Add case to the switch in emitAISOperation()

Make sure the wire index matches the plan. Use the same attribute names.
Only modify what's necessary — don't refactor surrounding code.
"""
    )
    g.add_edge(architect.get_last_node(), compiler_prompt)
    g.add_edge(print1, compiler_prompt, dependency="Control")

    runtime_prompt = g.ask(
        name="build_runtime_prompt",
        prompt="""Based on this plan, implement the runtime-side changes:

Plan:
{architect}

You need to:
1. Create crates/runtime/apxm-runtime/src/executor/handlers/<op_name>.rs
   - Implement the handler function following the pattern in spawn_agent.rs
   - Extract attributes from the node
   - Execute the operation logic
   - Return a ResultToken

2. Modify crates/runtime/apxm-runtime/src/executor/mod.rs
   - Add mod <op_name> in the handlers module section
   - Add a new arm to the execute_node() match statement

3. Add tests in the handler file
   - Basic success case
   - Error handling if applicable

Follow APXM conventions: use apxm-core types, proper error handling with context.
"""
    )
    g.add_edge(architect.get_last_node(), runtime_prompt)
    g.add_edge(print1, runtime_prompt, dependency="Control")

    # Step 3: Both devs work in parallel
    compiler_dev.ask("{compiler_prompt}")
    g.add_edge(compiler_prompt, compiler_dev.get_last_node())

    runtime_dev.ask("{runtime_prompt}")
    g.add_edge(runtime_prompt, runtime_dev.get_last_node())

    print2 = g.print(name="print_compiler_impl", message="=== COMPILER IMPL ===\n{compiler_dev}")
    g.add_edge(compiler_dev.get_last_node(), print2)

    print3 = g.print(name="print_runtime_impl", message="=== RUNTIME IMPL ===\n{runtime_dev}")
    g.add_edge(runtime_dev.get_last_node(), print3)

    # Step 4: Wait for both to complete, then review
    wait = g.wait_all(name="wait_implementations", compiler_dev.get_last_node(), runtime_dev.get_last_node())
    g.add_edge(print2, wait, dependency="Control")
    g.add_edge(print3, wait, dependency="Control")

    review_task = g.ask(
        name="build_review_task",
        prompt="""Review both implementations and verify they work together:

Compiler implementation:
{compiler_dev}

Runtime implementation:
{runtime_dev}

Check:
1. Wire index consistency across all files
2. Attribute names match between TableGen and handler
3. MLIR mnemonic follows conventions
4. Handler properly extracts and validates attributes
5. Error handling is present

Then run:
  dekk apxm build
  cargo test -p apxm-runtime

Report:
- What's correct
- What's wrong (if anything)
- Test results
- Whether the implementation is ready to merge

If tests fail, suggest fixes.
"""
    )
    g.add_edge(compiler_dev.get_last_node(), review_task)
    g.add_edge(runtime_dev.get_last_node(), review_task)
    g.add_edge(wait, review_task, dependency="Control")

    reviewer.ask("{review_task}")
    g.add_edge(review_task, reviewer.get_last_node())

    print4 = g.print(name="print_review", message="=== REVIEW ===\n{reviewer}")
    g.add_edge(reviewer.get_last_node(), print4)

    # Final synthesis
    final = g.think(
        name="synthesis",
        prompt="""Synthesize the add-op workflow results:

Plan: {architect}
Compiler impl: {compiler_dev}
Runtime impl: {runtime_dev}
Review: {reviewer}

Summary:
- Operation name and wire index
- Implementation status (ready/needs-fixes)
- Test results
- Next steps (if any)
"""
    )
    g.add_edge(architect.get_last_node(), final)
    g.add_edge(compiler_dev.get_last_node(), final)
    g.add_edge(runtime_dev.get_last_node(), final)
    g.add_edge(reviewer.get_last_node(), final)
    g.add_edge(print4, final, dependency="Control")

    print5 = g.print(name="print_final", message="=== FINAL SUMMARY ===\n{final}")
    g.add_edge(final, print5)

    g.done(print5)


if __name__ == "__main__":
    print(add_op_workflow._graph.to_air())
