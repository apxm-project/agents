#!/usr/bin/env python3
"""graph_builder.py - Meta-workflow: builds APXM workflows from goals

Give it a dream. It designs and writes the optimal APXM workflow to achieve it.

Usage: python3 -m examples.python.workflows.graph-builder.graph_builder
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def graph_builder(g: GraphRecorder):
    """Meta-workflow that generates complete APXM workflows from goals."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Define the mission/goal
    mission = g.ask(
        "mission",
        "clic-designer: AI-first commercial app builder. Takes a plain-language brief and produces "
        "a production-ready app. Enforces a full design phase before implementation: 4 parallel specialist "
        "agents produce 6 JSON spec artifacts (entity_map, navigation_spec, competitor_analysis, design_system, "
        "product_spec, screen_manifest), then generate ALL screens x ALL states as verified HTML. 2-stage "
        "verification gate: Qwen3 structural audit + Playwright screenshot to Qwen-VL visual review. "
        "Then convert verified HTML to any framework (Flutter/React/Vue/SwiftUI). Phases: DISCOVER, GENERATE "
        "(2A components + 2B shared + 2C feature screens), CONVERT, IMPLEMENT, TEST. Design the complete "
        "APXM workflow infrastructure for this."
    )

    # Phase 1: Parallel analysis
    team = g.team("analysis_team")
    strategist = team.add("strategist", profile=claude, cwd=cwd)
    architect = team.add("architect", profile=claude, cwd=cwd)
    skeptic = team.add("skeptic", profile=claude, cwd=cwd)

    # Strategy analysis
    strategy_prompt = g.ask(
        "strategy_prompt",
        "Build a workflow strategy analysis prompt for this goal: {mission}. Ask a Workflow Strategist "
        "to produce strategy.json with: goal_type, phases (name/purpose/inputs/outputs/can_parallelize), "
        "critical_path, parallelization_opportunities, verification_gates, human_checkpoints, "
        "estimated_complexity. Keep concise and direct."
    )

    strategist.ask("{strategy_prompt}")
    strategy_result = strategist.get_last_node()

    # Topology design
    arch_prompt = g.ask(
        "arch_prompt",
        "Build an agent topology design prompt. Goal: {mission}. Strategy: {strategy_result}. Ask an Agent Topology "
        "Architect to produce topology.json with: agents (name/role/profile/specialization/inputs/outputs/calls), "
        "entry_points, sub_workflows, data_artifacts. Profiles are claude/codex/qwen."
    )

    architect.ask("{arch_prompt}")
    arch_result = architect.get_last_node()

    # Skeptic review
    skeptic_prompt = g.ask(
        "skeptic_prompt",
        "Build a skeptic challenge prompt. Goal: {mission}. Strategy: {strategy_result}. Topology: {arch_result}. Ask a Workflow Skeptic "
        "to find: over-engineering, missing pieces, wrong ordering, bad profile fits, and define the minimum "
        "viable version. Output skeptic_review.json."
    )

    skeptic.ask("{skeptic_prompt}")
    skeptic_result = skeptic.get_last_node()

    # Phase 2: Generate Python workflow files
    workflow_writer = g.spawn(
        "workflow_writer", profile=claude, cwd=cwd
    )

    workflow_prompt = g.ask(
        "workflow_prompt",
        "Build a prompt for an APXM Python frontend expert to write complete workflow source files. "
        "Goal: {mission}. Strategy: {strategy_result}. Topology: {arch_result}. Skeptic feedback: {skeptic_result}. "
        "Use the Python frontend as the authoring format and assume canonical .air will be emitted for CLI execution. "
        "Ask for a JSON map of filename -> complete_python_content for all entry, phase, and sub-workflow files."
    )

    workflow_writer.ask("{workflow_prompt}")
    workflow_files = workflow_writer.get_last_node()

    # Agent profiles
    profiles_prompt = g.ask(
        "profiles_prompt",
        "Build a prompt for writing focused agent context profiles. Topology: {arch_result}. Ask for 100-200 word "
        "context blocks for each agent specifying their role, input format, output format, constraints. "
        "Output JSON map of agent_name -> context_text."
    )

    workflow_writer.ask("{profiles_prompt}")
    agent_profiles = workflow_writer.get_last_node()

    # Phase 3: Final synthesis
    summary = g.ask(
        "summary",
        "Synthesize a complete actionable summary. Goal: {mission}. Workflow files generated: {workflow_files}. "
        "Agent profiles: {agent_profiles}. Format: ## WORKFLOW DESIGN: [name]\n### What This Builds\n"
        "### Files to Create (filename: description)\n### How to Run It (exact commands)\n"
        "### Agent Roster (agent: role + profile)\n### Minimum Viable First Run (what to build first)"
    )

    # Print and return
    output = g.print("GRAPH-BUILDER OUTPUT\n\n{summary}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(graph_builder._graph.to_air())
