#!/usr/bin/env python3
"""ais_writer.py - Workflow that writes workflows

Give it a plan/brief. It designs and writes APXM Python workflow files.

Usage: python3 examples/python/workflows/ais-writer/ais_writer.py
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def ais_writer(g: GraphRecorder):
    """Meta-workflow that writes APXM workflow files from a plan."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Define the plan
    plan_and_dir = g.ask(
        "plan_and_dir",
        "clic-designer: AI-first commercial app builder on APXM. Enforced design phase before code. "
        "Phase 1 DISCOVER: 4 parallel specialists. Phase 2A COMMON PARTS: component registry. "
        "Phase 2B COMMON SCREENS: shared screens. Phase 2C FEATURE SCREENS: N screens x V variants. "
        "2-STAGE VERIFICATION: Qwen3 structural + Playwright visual. Phase 3A CONVERT: component widgets. "
        "Phase 3B IMPLEMENT: backend + wiring. Phase 4 TEST: stack tests + quality gate. "
        "Output dir: examples/workflows/appbuilder"
    )

    # Phase 1: Analyze the plan
    analyst = g.spawn("analyst", profile="claude", cwd=cwd)

    output_dir = g.ask(
        "output_dir",
        "Extract the output directory from this plan. Return ONLY the directory path. Plan: {0}"
    )
    plan_and_dir | output_dir

    manifest_prompt = g.ask(
        "manifest_prompt",
        "You are an APXM Workflow Analyst. Analyze this plan and produce a precise file manifest. "
        "Produce JSON with: output_dir, entry_files, phase_files, sub_files, agent_roster, data_flow. "
        "Plan: {0}"
    )
    plan_and_dir | manifest_prompt

    manifest_comm = g.communicate("manifest", target_agent="analyst", message="{0}")
    manifest_prompt | manifest_comm

    # Phase 2: Design + skeptic review
    architect = g.spawn("architect", profile="claude", cwd=cwd)
    skeptic = g.spawn("skeptic", profile="claude", cwd=cwd)

    topology_prompt = g.ask(
        "topology_prompt",
        "Design an APXM agent topology for this workflow. Manifest: {0}. "
        "For each agent, design: system prompt, inputs, outputs, constraints. "
        "Output JSON map of agent_name -> full_context_block."
    )
    manifest_comm | topology_prompt

    agent_contexts = g.communicate("agent_contexts", target_agent="architect", message="{0}")
    topology_prompt | agent_contexts

    skeptic_prompt = g.ask(
        "skeptic_prompt",
        "Challenge this workflow design. Manifest: {0}. Agent contexts: {1}. "
        "Find: over-engineering, missing pieces, wrong order, minimum viable. "
        "Output JSON with findings and recommended_first_file."
    )
    manifest_comm | skeptic_prompt
    agent_contexts | skeptic_prompt

    skeptic_review = g.communicate("skeptic_review", target_agent="skeptic", message="{0}")
    skeptic_prompt | skeptic_review

    # Phase 3: Write Python workflow files
    writer = g.spawn("writer", profile="claude", cwd=cwd)

    entry_write_prompt = g.ask(
        "entry_write_prompt",
        "Write entry point APXM Python workflow files. Manifest: {0}. Agent contexts: {1}. Skeptic: {2}. "
        "Use the Python frontend and target canonical .air emission for execution. "
        "Output JSON map: filename -> complete_python_content."
    )
    manifest_comm | entry_write_prompt
    agent_contexts | entry_write_prompt
    skeptic_review | entry_write_prompt

    writer.ask("{0}")
    entry_write_prompt | writer.get_last_node()

    # Phase 4: Generate summary
    summary = g.ask(
        "summary",
        "Format a final summary: ## WORKFLOW-WRITER COMPLETE\n### What Was Designed\n"
        "### Files Created\n### How to Run\n### First Step. "
        "Based on output dir: {0} and files: {1}"
    )
    output_dir | summary
    writer.get_last_node() | summary

    output = g.print_("output", message="WORKFLOW-WRITER COMPLETE\n\n{0}")
    summary | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(ais_writer._graph.to_air())
