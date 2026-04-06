#!/usr/bin/env python3
"""graph_builder.py - Meta-workflow: builds APXM workflows from goals

Give it a dream. It designs and writes the optimal APXM workflow to achieve it.

Usage: python3 -m examples.python.workflows.graph-builder.graph_builder
"""

from apxm.graph import compile, GraphRecorder
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
    strategist = team.add("strategist", profile="claude", cwd=cwd)
    architect = team.add("architect", profile="claude", cwd=cwd)
    skeptic = team.add("skeptic", profile="claude", cwd=cwd)

    # Strategy analysis
    strategy_prompt = g.ask(
        "strategy_prompt",
        "Build a workflow strategy analysis prompt for this goal: {0}. Ask a Workflow Strategist "
        "to produce strategy.json with: goal_type, phases (name/purpose/inputs/outputs/can_parallelize), "
        "critical_path, parallelization_opportunities, verification_gates, human_checkpoints, "
        "estimated_complexity. Keep concise and direct."
    )
    mission | strategy_prompt

    strategist.ask("{0}")
    strategy_prompt | strategist.get_last_node()

    # Topology design
    arch_prompt = g.ask(
        "arch_prompt",
        "Build an agent topology design prompt. Goal: {0}. Strategy: {1}. Ask an Agent Topology "
        "Architect to produce topology.json with: agents (name/role/profile/specialization/inputs/outputs/calls), "
        "entry_points, sub_workflows, data_artifacts. Profiles are claude/codex/qwen."
    )
    mission | arch_prompt
    strategist.get_last_node() | arch_prompt

    architect.ask("{0}")
    arch_prompt | architect.get_last_node()

    # Skeptic review
    skeptic_prompt = g.ask(
        "skeptic_prompt",
        "Build a skeptic challenge prompt. Goal: {0}. Strategy: {1}. Topology: {2}. Ask a Workflow Skeptic "
        "to find: over-engineering, missing pieces, wrong ordering, bad profile fits, and define the minimum "
        "viable version. Output skeptic_review.json."
    )
    mission | skeptic_prompt
    strategist.get_last_node() | skeptic_prompt
    architect.get_last_node() | skeptic_prompt

    skeptic.ask("{0}")
    skeptic_prompt | skeptic.get_last_node()

    # Phase 2: Generate Python workflow files
    workflow_writer = g.spawn(
        "workflow_writer", profile="claude", cwd=cwd
    )

    workflow_prompt = g.ask(
        "workflow_prompt",
        "Build a prompt for an APXM Python frontend expert to write complete workflow source files. "
        "Goal: {0}. Strategy: {1}. Topology: {2}. Skeptic feedback: {3}. "
        "Use the Python frontend as the authoring format and assume JSON graphs will be emitted for CLI execution. "
        "Ask for a JSON map of filename -> complete_python_content for all entry, phase, and sub-workflow files."
    )
    mission | workflow_prompt
    strategist.get_last_node() | workflow_prompt
    architect.get_last_node() | workflow_prompt
    skeptic.get_last_node() | workflow_prompt

    workflow_writer.ask("{0}")
    workflow_prompt | workflow_writer.get_last_node()

    # Agent profiles
    profiles_prompt = g.ask(
        "profiles_prompt",
        "Build a prompt for writing focused agent context profiles. Topology: {0}. Ask for 100-200 word "
        "context blocks for each agent specifying their role, input format, output format, constraints. "
        "Output JSON map of agent_name -> context_text."
    )
    architect.get_last_node() | profiles_prompt

    workflow_writer.ask("{0}")
    profiles_prompt | workflow_writer.get_last_node()

    # Phase 3: Final synthesis
    summary = g.ask(
        "summary",
        "Synthesize a complete actionable summary. Goal: {0}. Workflow files generated: {1}. "
        "Agent profiles: {2}. Format: ## WORKFLOW DESIGN: [name]\n### What This Builds\n"
        "### Files to Create (filename: description)\n### How to Run It (exact commands)\n"
        "### Agent Roster (agent: role + profile)\n### Minimum Viable First Run (what to build first)"
    )
    mission | summary
    workflow_writer.get_last_node() | summary

    # Print and return
    output = g.print_("output", message="GRAPH-BUILDER OUTPUT\n\n{0}")
    summary | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(json.dumps(graph_builder._graph.to_dict(), indent=2))
