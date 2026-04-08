#!/usr/bin/env python3
"""showcase.py - APXM Optimization Showcase

This workflow demonstrates ALL APXM optimization features:
1. Auto-wiring - templates reference nodes by name
2. Typed profiles - import from _generated
3. Per-node model routing - different models for different ops
4. Parallel execution - fan-out pattern
5. Agent spawning - real ACP agents
6. Memory operations - query_memory/update_memory
7. Compile-time optimization potential:
   - Adjacent THINKs that can fuse
   - Dead context elimination
   - Shared prefix reuse
   - Parallel scheduling

Usage:
  # Compile at O0 (no optimizations)
  dekk apxm compile showcase.apxm -O0 -o showcase_O0.apxmobj

  # Compile at O2 (all optimizations)
  dekk apxm compile showcase.apxm -O2 -o showcase_O2.apxmobj

  # Execute with session tracing
  dekk apxm execute showcase.apxm --emit-session --emit-metrics metrics.json

  # View MLIR
  python3 -m examples.python.demo.showcase
"""

from apxm import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def apxm_showcase(g: GraphRecorder):
    """APXM Showcase: demonstrates all optimization features in one workflow."""

    # User input topic
    topic_input = g.ask(
        "topic_input",
        "What technical topic should we analyze? (e.g., 'Rust async runtimes', "
        "'LLVM optimization passes', 'distributed consensus algorithms')"
    )

    # Phase 1: Fast triage (cheap model for quick classification)
    # Demonstrates: per-node model routing
    # In production: model="gpt-4o-mini"
    triage = g.ask(
        "triage",
        "Quick triage of this topic: {topic_input}\n\n"
        "In 3 bullet points:\n"
        "- Architecture complexity: [LOW/MEDIUM/HIGH]\n"
        "- Security considerations: [MINIMAL/MODERATE/CRITICAL]\n"
        "- Performance impact: [LOW/MEDIUM/HIGH]"
    )

    # Phase 2: Deep parallel analysis (powerful model, fan-out)
    # Demonstrates: parallel execution, auto-wiring by name, shared prefix reuse
    # In production: model="claude-sonnet-4"
    arch = g.think(
        "architecture_analysis",
        "Based on triage: {triage}\n\n"
        "Provide deep architecture analysis of: {topic_input}\n\n"
        "Cover: core components, design patterns, trade-offs, scalability.\n"
        "Be thorough (300-400 words)."
    )

    security = g.think(
        "security_analysis",
        "Based on triage: {triage}\n\n"
        "Provide deep security analysis of: {topic_input}\n\n"
        "Cover: attack vectors, mitigation strategies, threat model, best practices.\n"
        "Be thorough (300-400 words)."
    )

    performance = g.think(
        "performance_analysis",
        "Based on triage: {triage}\n\n"
        "Provide deep performance analysis of: {topic_input}\n\n"
        "Cover: bottlenecks, optimization strategies, benchmarking, profiling.\n"
        "Be thorough (300-400 words)."
    )

    # Phase 3: Memory operations
    # Demonstrates: update_memory (UMEM), query_memory (QMEM)
    store_arch = g.update_memory(
        "store_analysis",
        data=arch,
        key="last_architecture_analysis"
    )
    arch | store_arch  # Data dependency

    recalled = g.query_memory(
        "recall_history",
        query="previous analyses of similar topics"
    )

    # Phase 4: Synthesis with context
    # Demonstrates: complex data dependencies, memory context from AAM
    synthesis = g.think(
        "synthesis",
        "Synthesize these analyses into executive summary (use any relevant historical context from memory):\n\n"
        "ARCHITECTURE:\n{arch}\n\n"
        "SECURITY:\n{security}\n\n"
        "PERFORMANCE:\n{performance}\n\n"
        "Create a cohesive 500-word executive summary highlighting key insights, "
        "trade-offs, and recommendations."
    )
    recalled >> synthesis  # Control dependency to ensure memory is queried first
    store_arch >> synthesis  # Control dependency for memory

    # Phase 5: Agent implementation
    # Demonstrates: SPAWN_AGENT, ACP protocol, agent communication
    cwd = os.environ.get("APXM_HOME", os.getcwd())
    coder = g.spawn("coder", profile=claude, cwd=cwd)

    implementation_task = g.ask(
        "implementation_task",
        "Based on this synthesis:\n{synthesis}\n\n"
        "Create a detailed implementation task for an AI coding agent. "
        "Specify: what to build, key requirements, acceptance criteria. "
        "Be concrete and actionable."
    )

    # Agent receives the task via COMMUNICATE
    agent_result = g.communicate(
        target_agent="coder",
        message="{implementation_task}"
    )

    # Phase 6: Review and validation
    # Demonstrates: adjacent THINK nodes (fusion candidate)
    review = g.think(
        "review",
        "Review this implementation:\n{agent_result}\n\n"
        "Does it meet the requirements? What's good? What needs improvement?"
    )

    # Additional validation (adjacent THINK - optimization opportunity)
    validation = g.think(
        "validation",
        "Validate completeness:\n{agent_result}\n\n"
        "Check: error handling, edge cases, testing strategy, documentation."
    )

    # Phase 7: Final report assembly
    final_report = g.merge(
        "final_report",
        synthesis,
        review,
        validation
    )

    # Output
    output1 = g.print(
        "=== APXM SHOWCASE COMPLETE ===\n\n"
        "TOPIC: {topic_input}\n\n"
        "SYNTHESIS:\n{synthesis}\n\n"
    )

    output2 = g.print(
        "REVIEW:\n{review}\n\n"
        "VALIDATION:\n{validation}"
    )
    output1 >> output2

    g.done(final_report)


if __name__ == "__main__":
    """
    Generate MLIR (.air format) to demonstrate optimization potential.

    Key optimization opportunities visible in the MLIR:
    - FuseReasoning: adjacent THINK nodes (review + validation)
    - DeadContextElimination: unused intermediate values
    - SharedPrefixReuse: parallel fan-out from triage
    - MemoCache: repeated queries with same prompt
    - PriorityScheduling: critical path through synthesis -> review
    """
    print("=" * 80)
    print("APXM SHOWCASE WORKFLOW - MLIR Output (.air)")
    print("=" * 80)
    print()
    print(apxm_showcase._graph.to_air())
    print()
    print("=" * 80)
    print("OPTIMIZATION ANALYSIS")
    print("=" * 80)
    print()
    print("This workflow demonstrates optimization potential at:")
    print()
    print("1. PARALLEL EXECUTION")
    print("   - architecture_analysis, security_analysis, performance_analysis fan out from triage")
    print("   - All three can execute concurrently (no data dependencies between them)")
    print("   - Expected speedup: ~3x if all run in parallel vs sequential")
    print()
    print("2. FUSION OPPORTUNITIES")
    print("   - review + validation are adjacent THINK nodes")
    print("   - FuseReasoning pass can combine them into single LLM call")
    print("   - Benefit: reduced API overhead, faster total time")
    print()
    print("3. SHARED PREFIX REUSE")
    print("   - All three parallel analyses share prefix: 'Based on triage: {triage}'")
    print("   - With KV cache, can reuse prefix computation")
    print("   - Benefit: reduced token processing, lower cost")
    print()
    print("4. MEMORY OPERATIONS")
    print("   - update_memory stores architecture analysis (UMEM)")
    print("   - query_memory recalls historical context (QMEM)")
    print("   - Demonstrates AAM (Beliefs) integration")
    print()
    print("5. AGENT SPAWNING")
    print("   - SPAWN_AGENT creates ACP subprocess (coder)")
    print("   - COMMUNICATE sends task to agent")
    print("   - Demonstrates multi-agent coordination")
    print()
    print("6. PER-NODE MODEL ROUTING")
    print("   - triage could use gpt-4o-mini (fast/cheap)")
    print("   - Deep analyses could use claude-sonnet-4 (powerful)")
    print("   - Demonstrates cost/quality trade-offs")
    print()
    print("7. DEAD CONTEXT ELIMINATION")
    print("   - If final_report doesn't use all merged inputs")
    print("   - DeadContextElimination can prune unused computations")
    print()
    print("To see these optimizations in action:")
    print("  dekk apxm compile showcase.apxm -O2 --emit-diagnostics diag.json")
    print("  dekk apxm analyze showcase.apxm  # parallelism + critical path analysis")
    print("  dekk apxm execute showcase.apxm --emit-metrics metrics.json")
    print()
