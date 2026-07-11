#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parents[1]
PYTHON_ROOT = Path(__file__).resolve().parent
REPO_ROOT = PACKAGE_ROOT.parents[2]
PYTHON_FRONTEND = REPO_ROOT / "crates" / "compiler" / "frontend" / "python"
for path in (str(PACKAGE_ROOT), str(PYTHON_ROOT), str(PYTHON_FRONTEND)):
    if path not in sys.path:
        sys.path.insert(0, path)

from apxm import CompactionPolicy, ConversationalAgent, GraphRecorder, ToolGroup, compile
from capabilities.handlers.apxm_authoring import plan_workflow
from capabilities.handlers.hooks import (
    compact_conversation,
    gate_compose_workflow,
    inject_apxm_context,
    inject_context,
    redact_tool_results,
)
from capabilities.handlers.local_skills import list_local_skills, read_local_skill
from capabilities.handlers.paper_search import search_papers
from capabilities.handlers.workflow_validation import explain_permission, prepare_validation
from provider_policy import ProviderConnection, capability_groups, provider_capabilities


PERSONA = (
    "You are Gao, an APXM architecture expert. Cite only local paper-search "
    "results and return Studio-compatible workflow drafts."
)
GAO_CAPABILITY_TOOL_BINDINGS = [
    list_local_skills,
    read_local_skill,
    search_papers,
    plan_workflow,
    prepare_validation,
    explain_permission,
]


def build_agent(connection: ProviderConnection | None = None):
    return ConversationalAgent(
        persona=PERSONA,
        memory_space="stm",
        tools=GAO_CAPABILITY_TOOL_BINDINGS,
        capability_groups=capability_groups(connection),
        skills=True,
        compaction=CompactionPolicy(
            keep_recent=4,
            compact_at_tokens=20_000,
            summary_key="gao:conversation:summary",
        ),
        hooks=[
            inject_context,
            inject_apxm_context,
            gate_compose_workflow,
            redact_tool_results,
            compact_conversation,
        ],
        loop="host",
    ).compile()


main = build_agent()


@compile()
def gao_turn(g: GraphRecorder, user_message: str):
    papers = g.invoke_capability(search_papers, query="{user_message}")
    canvas = g.invoke_capability(plan_workflow, request="{user_message}")
    answer = g.ask(
        name="answer",
        prompt=(
            "User question:\n{user_message}\n\n"
            "Local paper search:\n{papers}\n\n"
            "Studio draft:\n{canvas}"
        ),
        system_prompt=PERSONA,
        capability_groups=capability_groups(),
    )
    g.done(source=answer)


def run_live_turn(question: str) -> dict[str, object]:
    paper_result = json.loads(search_papers.fn(question))
    canvas = json.loads(plan_workflow.fn(question))["studio_canvas"]
    matches = paper_result["matches"]
    if matches:
        answer = f"APXM uses dependency-driven capability graphs. Source: {matches[0]['citation']}"
    else:
        answer = "No source found in the local paper corpus."
    return {"answer": answer, "paper_result": paper_result, "canvas": canvas}


if __name__ == "__main__":
    result = main.validate()
    if "--validate" in sys.argv:
        print("VALID" if result.valid else "INVALID")
        for error in result.errors:
            print(f"  ERROR: {error}")
        raise SystemExit(0 if result.valid else 1)
    print(main.to_air())
