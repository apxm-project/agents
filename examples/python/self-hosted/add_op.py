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
    dekk apxm execute examples/python/self-hosted/add_op.py \
      "CHECKPOINT" "Save execution state for later resume"
"""

from apxm import GraphRecorder, agent_cwd, compile
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

    cwd = agent_cwd()

    # Spawn agents
    architect = g.spawn("architect", profile=claude, cwd=cwd)
    compiler_dev = g.spawn("compiler_dev", profile=claude, cwd=cwd)
    runtime_dev = g.spawn("runtime_dev", profile=codex, cwd=cwd)
    reviewer = g.spawn("reviewer", profile=claude, cwd=cwd)

    # Step 1: Architect analyzes and creates implementation plan
    architect_plan = architect.ask(prompt="""You are the architect for APXM. You need to create an implementation plan
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

    print1 = g.print(message="=== ARCHITECT PLAN ===\n{architect_plan}")

    # Step 2: Build implementation prompts for parallel execution
    compiler_prompt = g.ask(
        name="build_compiler_prompt",
        prompt="""Based on this plan, implement the compiler-side changes:

Plan:
{architect_plan}

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

    runtime_prompt = g.ask(
        name="build_runtime_prompt",
        prompt="""Based on this plan, implement the runtime-side changes:

Plan:
{architect_plan}

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

    # Step 3: Both devs work in parallel
    compiler_impl = compiler_dev.ask("{compiler_prompt}")

    runtime_impl = runtime_dev.ask("{runtime_prompt}")

    print2 = g.print(message="=== COMPILER IMPL ===\n{compiler_impl}")
    print3 = g.print(message="=== RUNTIME IMPL ===\n{runtime_impl}")

    # Step 4: Wait for both to complete, then review
    wait = g.wait_all("wait_implementations", compiler_impl, runtime_impl)
    g.add_edge(print2, wait, dependency="Control")
    g.add_edge(print3, wait, dependency="Control")

    review_task = g.ask(
        name="build_review_task",
        prompt="""Review both implementations and verify they work together:

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
    g.add_edge(wait, review_task, dependency="Control")

    review_result = reviewer.ask("{review_task}")

    print4 = g.print(message="=== REVIEW ===\n{review_result}")

    # Final synthesis
    final = g.think(
        name="synthesis",
        prompt="""Synthesize the add-op workflow results:

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
    g.add_edge(print4, final, dependency="Control")

    print5 = g.print(message="=== FINAL SUMMARY ===\n{final}")

    g.done(print5)


if __name__ == "__main__":
    import apxm

    result = apxm.run(add_op_workflow("SUMMARIZE", "Summarize input text"))
    print(result.content)
