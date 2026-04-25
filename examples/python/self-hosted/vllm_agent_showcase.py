#!/usr/bin/env python3
"""Multi-agent APXM showcase backed by a graph-aware vLLM endpoint.

This example demonstrates one APXM execution graph that combines:

- parallel ACP agent nodes for Claude and Codex,
- vLLM-backed LLM nodes for local synthesis and arbitration,
- session traces and metrics that separate node activity from backend graph
  telemetry.

Usage:
  dekk apxm execute \
    --emit-session /tmp/apxm-vllm-agent-session \
    --emit-metrics /tmp/apxm-vllm-agent-metrics.json \
    examples/python/self-hosted/vllm_agent_showcase.py

Routing:
  Register one model under a vLLM backend with alias "showcase", or replace
  SHOWCASE_ROUTE with another explicit registered backend/model selector.

Optional environment overrides:
  - APXM_SHOWCASE_CWD
  - APXM_SHOWCASE_TASK
  - APXM_EMIT_AIR=1
"""

from __future__ import annotations

import os

from apxm import GraphRecorder, compile
from apxm._generated.agents import claude, codex
from apxm._generated.providers import VLLM
from apxm.backends import select_backend
from apxm.constants import ENV_APXM_EMIT_AIR, ENV_FLAG_ENABLED


SHOWCASE_MODEL_ALIAS = "showcase"
ENV_SHOWCASE_CWD = "APXM_SHOWCASE_CWD"
ENV_SHOWCASE_TASK = "APXM_SHOWCASE_TASK"
ENV_EMIT_AIR = ENV_APXM_EMIT_AIR

NODE_CLAUDE_AGENT = "claude_architect"
NODE_CODEX_AGENT = "codex_reviewer"
NODE_WAIT_FOR_AGENTS = "wait_for_agent_council"
NODE_VLLM_SYNTHESIS = "vllm_synthesis"
NODE_VLLM_DECISION = "vllm_decision"
NODE_PRINT_REPORT = "print_showcase_report"

PROMPT_CLAUDE_ARCHITECT = (
    "You are the architecture reviewer for an APXM workflow demo. "
    "Evaluate this task for graph structure, boundaries, and observability. "
    "Return 4 bullets: graph shape, risks, metrics to inspect, and one "
    "recommended improvement.\n\nTask:\n{task}"
)
PROMPT_CODEX_REVIEWER = (
    "You are the implementation reviewer for an APXM workflow demo. "
    "Evaluate this task for concrete code changes, test strategy, and failure "
    "modes. Return 4 bullets with file or command references when useful.\n\n"
    "Task:\n{task}"
)
PROMPT_VLLM_SYNTHESIS = (
    "Synthesize these two agent reports into a concise APXM execution plan. "
    "Keep the output operational: what runs in parallel, what runs on the "
    "vLLM backend, and what metrics prove the run worked.\n\n"
    "Task:\n{task}\n\n"
    "Claude architecture report:\n{claude_report}\n\n"
    "Codex implementation report:\n{codex_report}"
)
PROMPT_VLLM_DECISION = (
    "Convert this APXM execution plan into a validation checklist. "
    "Separate graph metrics, node metrics, backend graph telemetry, and "
    "agent-output caveats. Keep it under 10 bullets.\n\n"
    "{vllm_synthesis}"
)
OUTPUT_TEMPLATE = (
    "=== APXM vLLM + ACP Agent Showcase ===\n"
    "backend={backend}\n"
    "model={model}\n\n"
    "{vllm_decision}"
)
DEFAULT_SHOWCASE_TASK = (
    "Evaluate APXM's value for running a workflow where Claude and Codex "
    "inspect the same engineering problem in parallel, then a local "
    "graph-aware vLLM backend synthesizes the final validation plan."
)


SHOWCASE_ROUTE = select_backend(protocol=VLLM.protocol, alias=SHOWCASE_MODEL_ALIAS)


@compile(default_route=SHOWCASE_ROUTE)
def vllm_agent_showcase(g: GraphRecorder, task: str):
    """Run a parallel ACP agent council and local vLLM synthesis."""

    cwd = os.environ.get(ENV_SHOWCASE_CWD, os.getcwd())

    claude_agent = g.spawn(NODE_CLAUDE_AGENT, profile=claude, cwd=cwd)
    codex_agent = g.spawn(NODE_CODEX_AGENT, profile=codex, cwd=cwd)

    claude_report = claude_agent.ask(PROMPT_CLAUDE_ARCHITECT)
    codex_report = codex_agent.ask(PROMPT_CODEX_REVIEWER)
    agent_council = g.wait_all(
        NODE_WAIT_FOR_AGENTS,
        [claude_report, codex_report],
    )

    vllm_synthesis = g.think(
        name=NODE_VLLM_SYNTHESIS,
        prompt=PROMPT_VLLM_SYNTHESIS,
    )
    g.add_edge(agent_council, vllm_synthesis, dependency="Control")

    vllm_decision = g.reason(
        name=NODE_VLLM_DECISION,
        prompt=PROMPT_VLLM_DECISION,
    )

    report = g.print(
        name=NODE_PRINT_REPORT,
        message=OUTPUT_TEMPLATE.format(
            backend=SHOWCASE_ROUTE.backend,
            model=SHOWCASE_ROUTE.model,
            vllm_decision="{vllm_decision}",
        ),
    )
    g.done(report)


if __name__ == "__main__":
    if os.environ.get(ENV_EMIT_AIR) == ENV_FLAG_ENABLED:
        print(vllm_agent_showcase._air_text)
    else:
        import apxm

        task = os.environ.get(ENV_SHOWCASE_TASK, DEFAULT_SHOWCASE_TASK)
        result = apxm.run(vllm_agent_showcase(task))
        print(result.content)
