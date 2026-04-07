#!/usr/bin/env python3
"""ais_writer.py - Workflow that writes workflows

Give it a plan/brief. It designs and writes APXM Python workflow files.

Usage: python3 examples/python/workflows/ais-writer/ais_writer.py
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def ais_writer(g: GraphRecorder):
    """Meta-workflow that writes APXM workflow files from a plan."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Define the plan
    plan_and_dir = g.ask(
        "plan_and_dir",
        template="clic-designer: AI-first commercial app builder on APXM. Enforced design phase before code. "
        "Phase 1 DISCOVER: 4 parallel specialists. Phase 2A COMMON PARTS: component registry. "
        "Phase 2B COMMON SCREENS: shared screens. Phase 2C FEATURE SCREENS: N screens x V variants. "
        "2-STAGE VERIFICATION: Qwen3 structural + Playwright visual. Phase 3A CONVERT: component widgets. "
        "Phase 3B IMPLEMENT: backend + wiring. Phase 4 TEST: stack tests + quality gate. "
        "Output dir: examples/workflows/appbuilder"
    )

    # Phase 1: Analyze the plan
    analyst = g.spawn("analyst", profile=claude, cwd=cwd)

    output_dir = g.ask(
        "output_dir",
        template="Extract the output directory from this plan. Return ONLY the directory path. Plan: {plan_and_dir}"
    )

    manifest_prompt = g.ask(
        "manifest_prompt",
        template="You are an APXM Workflow Analyst. Analyze this plan and produce a precise file manifest. "
        "Produce JSON with: output_dir, entry_files, phase_files, sub_files, agent_roster, data_flow. "
        "Plan: {plan_and_dir}"
    )

    manifest_comm = g.communicate("manifest", target_agent="analyst", message="{manifest_prompt}")

    # Phase 2: Design + skeptic review
    architect = g.spawn("architect", profile=claude, cwd=cwd)
    skeptic = g.spawn("skeptic", profile=claude, cwd=cwd)

    topology_prompt = g.ask(
        "topology_prompt",
        template="Design an APXM agent topology for this workflow. Manifest: {manifest_comm}. "
        "For each agent, design: system prompt, inputs, outputs, constraints. "
        "Output JSON map of agent_name -> full_context_block."
    )

    agent_contexts = g.communicate("agent_contexts", target_agent="architect", message="{topology_prompt}")

    skeptic_prompt = g.ask(
        "skeptic_prompt",
        template="Challenge this workflow design. Manifest: {manifest_comm}. Agent contexts: {agent_contexts}. "
        "Find: over-engineering, missing pieces, wrong order, minimum viable. "
        "Output JSON with findings and recommended_first_file."
    )

    skeptic_review = g.communicate("skeptic_review", target_agent="skeptic", message="{skeptic_prompt}")

    # Phase 3: Write Python workflow files
    writer = g.spawn("writer", profile=claude, cwd=cwd)

    entry_write_prompt = g.ask(
        "entry_write_prompt",
        template="Write entry point APXM Python workflow files. Manifest: {manifest_comm}. Agent contexts: {agent_contexts}. Skeptic: {skeptic_review}. "
        "Use the Python frontend and target canonical .air emission for execution. "
        "Output JSON map: filename -> complete_python_content."
    )

    writer.ask("{entry_write_prompt}")
    writer_result = writer.get_last_node()

    # Phase 4: Generate summary
    summary = g.ask(
        "summary",
        template="Format a final summary: ## WORKFLOW-WRITER COMPLETE\n### What Was Designed\n"
        "### Files Created\n### How to Run\n### First Step. "
        "Based on output dir: {output_dir} and files: {writer_result}"
    )

    output = g.print("WORKFLOW-WRITER COMPLETE\n\n{summary}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(ais_writer._graph.to_air())
