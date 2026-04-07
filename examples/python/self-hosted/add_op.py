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
    PYTHONPATH=crates/apxm-frontend/python python3 examples/python/self-hosted/add_op.py > /tmp/add_op.air
    dekk apxm compile /tmp/add_op.air -o /tmp/add_op.apxmobj
    dekk apxm execute /tmp/add_op.air "CHECKPOINT" "Save execution state for later resume"
"""

import os
from apxm import compile, GraphRecorder
from apxm._generated.agents import claude, codex


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
    architect = g.spawn("architect", profile=claude, cwd=cwd)
    compiler_dev = g.spawn("compiler_dev", profile=claude, cwd=cwd)
    runtime_dev = g.spawn("runtime_dev", profile=codex, cwd=cwd)
    reviewer = g.spawn("reviewer", profile=claude, cwd=cwd)

    # Step 1: Architect analyzes and creates implementation plan
    architect_task = g.text(value="""You are the architect for APXM. You need to create an implementation plan
for adding a new AIS operation to the APXM codebase.

Operation name: {op_name}
Description: {op_description}

Read the following files to understand the pattern:
- crates/apxm-ais/src/definitions.rs (AISOperationType enum)
- crates/apxm-compiler/src/lower/ArtifactEmitter.cpp (MLIR lowering)
- crates/apxm-compiler/mlir/AISOps.td (TableGen definitions)
- crates/apxm-runtime/src/executor/handlers/spawn_agent.rs (example handler)
- crates/apxm-runtime/src/executor/mod.rs (dispatcher)

Create a structured plan with:
1. Wire index (next available after checking definitions.rs)
2. Required and optional node attributes
3. MLIR mnemonic (should be lowercase, e.g., "ais.checkpoint")
4. Handler pseudo-code (what should the runtime do)
5. TableGen definition structure
6. Test strategy

Keep the plan under 400 words but be specific about attribute names and types.
"""
    )

    architect.ask("{architect_task}")

    print1 = g.print("=== ARCHITECT PLAN ===\n{architect_plan}")
    architect_plan = architect.get_last_node()

    # Step 2: Build implementation prompts for parallel execution
    compiler_prompt = g.ask(
        "build_compiler_prompt",
        """Based on this plan, implement the compiler-side changes:

Plan:
{architect_plan}

You need to modify:
1. crates/apxm-ais/src/definitions.rs
   - Add variant to AISOperationType enum
   - Add to from_wire_index() match

2. crates/apxm-compiler/mlir/AISOps.td
   - Add def AIS_<OpName>Op block following the pattern of other ops

3. crates/apxm-compiler/src/lower/ArtifactEmitter.cpp
   - Add case to the switch in emitAISOperation()

Make sure the wire index matches the plan. Use the same attribute names.
Only modify what's necessary — don't refactor surrounding code.
"""
    )

    runtime_prompt = g.ask(
        "build_runtime_prompt",
        """Based on this plan, implement the runtime-side changes:

Plan:
{architect_plan}

You need to:
1. Create crates/apxm-runtime/src/executor/handlers/<op_name>.rs
   - Implement the handler function following the pattern in spawn_agent.rs
   - Extract attributes from the node
   - Execute the operation logic
   - Return a ResultToken

2. Modify crates/apxm-runtime/src/executor/mod.rs
   - Add mod <op_name> in the handlers module section
   - Add a new arm to the execute_node() match statement

3. Add tests in the handler file
   - Basic success case
   - Error handling if applicable

Follow APXM conventions: use apxm-core types, proper error handling with context.
"""
    )

    # Step 3: Both devs work in parallel
    compiler_dev.ask("{compiler_prompt}")
    compiler_impl = compiler_dev.get_last_node()

    runtime_dev.ask("{runtime_prompt}")
    runtime_impl = runtime_dev.get_last_node()

    print2 = g.print("=== COMPILER IMPL ===\n{compiler_impl}")
    print3 = g.print("=== RUNTIME IMPL ===\n{runtime_impl}")

    # Step 4: Wait for both to complete, then review
    wait = g.wait_all("wait_implementations", compiler_impl, runtime_impl)
    print2 >> wait
    print3 >> wait

    review_task = g.ask(
        "build_review_task",
        """Review both implementations and verify they work together:

Compiler implementation:
{compiler_impl}

Runtime implementation:
{runtime_impl}

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
    wait >> review_task

    reviewer.ask("{review_task}")
    review_result = reviewer.get_last_node()

    print4 = g.print("=== REVIEW ===\n{review_result}")

    # Final synthesis
    final = g.think(
        "synthesis",
        """Synthesize the add-op workflow results:

Plan: {architect_plan}
Compiler impl: {compiler_impl}
Runtime impl: {runtime_impl}
Review: {review_result}

Summary:
- Operation name and wire index
- Implementation status (ready/needs-fixes)
- Test results
- Next steps (if any)
"""
    )
    print4 >> final

    print5 = g.print("=== FINAL SUMMARY ===\n{final}")

    g.done(print5)


if __name__ == "__main__":
    print(add_op_workflow._graph.to_air())
