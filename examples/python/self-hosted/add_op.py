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
    dekk agents execute examples/python/self-hosted/add_op.py \
      "SUMMARIZE" "Summarize input text"
"""

from apxm import DependencyType, GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude, codex


@compile()
def add_op_workflow(g: GraphRecorder):
    """Add a new AIS operation to APXM.

    Parameters:
        op_name (str): Name of the operation (e.g., "SUMMARIZE")
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

definitions.rs is the source of truth: the wire enum + C++ lowering cases
are generated from it; AISOps.td is a hand-maintained mirror. Read:
- crates/machine/ais/src/operations/definitions.rs (AISOperationType enum,
  WIRE_INDEXED_OPERATIONS table, the AIS_OPERATIONS OperationSpec list)
- crates/machine/ais/src/operations/attrs.rs (attribute-name constants)
- crates/compiler/pipeline/mlir/include/ais/Dialect/AIS/IR/AISOps.td (hand-written dialect)
- crates/runtime/engine/src/executor/handlers/spawn_agent.rs (example handler)
- crates/runtime/engine/src/executor/dispatcher.rs (op -> handler dispatch)

Create a structured plan with:
1. Wire index (next free index in WIRE_INDEXED_OPERATIONS; append-only, 30 reserved)
2. OperationSpec: category, required/optional attribute fields, example_json, emission
3. Attribute names (each a new attrs.rs constant in ALL_ATTR_NAMES)
4. MLIR mnemonic (lowercase, e.g. "ais.checkpoint") and AISOps.td arguments
5. Handler pseudo-code (what the runtime does)
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
1. crates/machine/ais/src/operations/definitions.rs
   - Add the AISOperationType variant
   - Append (N, AISOperationType::<OpName>) to WIRE_INDEXED_OPERATIONS (append-only, 30 reserved)
   - Add the OperationSpec entry to AIS_OPERATIONS (category, field schema, example_json, emission)
   - The Display / FromStr / mlir_mnemonic / to_tablegen_name matches are
     exhaustive and will fail to compile until you add each arm — follow the compiler.

2. crates/machine/ais/src/operations/attrs.rs
   - Add each attribute name as a const and list it in ALL_ATTR_NAMES (no string literals).

3. crates/compiler/pipeline/mlir/include/ais/Dialect/AIS/IR/AISOps.td
   - Hand-write the AIS_<OpName>Op def; its `arguments` mirror the spec fields
     by the SAME attr names (typed OptionalAttr<...>, not bare attr-dict).
   - Do NOT edit ArtifactEmitter.cpp: its lowering cases are generated from
     WIRE_INDEXED_OPERATIONS unless the op needs bespoke lowering.

Make sure the wire index matches the plan. Use the same attribute names across
spec, attrs.rs, and AISOps.td. Only modify what's necessary.
"""
    )

    runtime_prompt = g.ask(
        name="build_runtime_prompt",
        prompt="""Based on this plan, implement the runtime-side changes:

Plan:
{architect_plan}

You need to:
1. Create crates/runtime/engine/src/executor/handlers/<op_name>.rs
   - Implement the handler function following the pattern in spawn_agent.rs
   - Extract attributes from the node
   - Execute the operation logic
   - Return a ResultToken

2. Wire up dispatch:
   - Add `pub mod <op_name>;` to crates/runtime/engine/src/executor/handlers/mod.rs
   - Add a dispatch arm to the `match node.op_type` in
     crates/runtime/engine/src/executor/dispatcher.rs

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
    g.add_edge(print2, wait, dependency=DependencyType.CONTROL)
    g.add_edge(print3, wait, dependency=DependencyType.CONTROL)

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
  dekk agents build
  dekk agents test

Report:
- What's correct
- What's wrong (if anything)
- Test results
- Whether the implementation is ready to merge

If tests fail, suggest fixes.
"""
    )
    g.add_edge(wait, review_task, dependency=DependencyType.CONTROL)

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
    g.add_edge(print4, final, dependency=DependencyType.CONTROL)

    print5 = g.print(message="=== FINAL SUMMARY ===\n{final}")

    g.done(print5)


if __name__ == "__main__":
    import apxm

    result = apxm.run(add_op_workflow("SUMMARIZE", "Summarize input text"))
    print(result.content)
