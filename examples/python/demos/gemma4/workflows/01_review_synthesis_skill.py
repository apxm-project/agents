#!/usr/bin/env python3
"""Case 01 — ReviewSynthesis Skill.

What this case shows
--------------------
A reusable skill that fans out to two ACP sub-agents (Claude as architect,
Codex as implementation reviewer), then synthesizes the result through a Gemma
model on the registered vLLM backend. The whole thing is one APXM graph: ACP
spawn/ask, vLLM `think`/`reason`, and a final `print` node.

Why it matters
--------------
- Skill-library boundary: the same compiled artifact can run inside any host
  agent without leaking host context.
- Backend graph registration: vLLM nodes carry the same route alias, so the
  backend sees a registered graph instead of a stream of isolated calls.
- Compile-once / run-many: the artifact below is what `dekk apxm run` executes
  repeatedly during benchmarks.

Claim boundary
--------------
This is the **story** demo, not the optimization claim. Do **not** quote O2
speedup, token reduction, or call-count drops from this case unless a fresh
`runtime-o0-o2.csv` row reproduces it. Use case 02 (context pruning) for those
claims.
"""

from __future__ import annotations

import os

from shared.bootstrap import bootstrap_paths

bootstrap_paths(__file__)

from apxm import (  # noqa: E402  (import after sys.path bootstrap)
    GraphRecorder,
    agent_cwd,
    compile,
    emit_air_if_requested,
)
from apxm._generated.agents import claude, codex  # noqa: E402

from shared.routes import SHOWCASE_ROUTE  # noqa: E402


# ---------------------------------------------------------------------------
# Node names — keep stable; deck slides reference them.
# ---------------------------------------------------------------------------

NODE_CLAUDE_AGENT = "claude_architect"
NODE_CODEX_AGENT = "codex_reviewer"
NODE_VLLM_SYNTHESIS = "gemma4_synthesis"
NODE_VLLM_CHECKLIST = "gemma4_validation_checklist"
NODE_PRINT_REPORT = "print_metrics_report"

ENV_SHOWCASE_CWD = "APXM_SHOWCASE_CWD"
ENV_SHOWCASE_TASK = "APXM_SHOWCASE_TASK"

# ---------------------------------------------------------------------------
# Prompts — every claim-discipline rule the agents must follow lives here.
# ---------------------------------------------------------------------------

PROMPT_CLAUDE_ARCHITECT = (
    "Review this APXM demo task from a system architecture perspective. "
    "Return 3 bullets: graph boundaries, observability evidence, and one risk. "
    "Do not claim measured speedup, cache hits, token savings, or cost savings.\n\n"
    "Task:\n{task}"
)

PROMPT_CODEX_REVIEWER = (
    "Review this APXM demo task from an implementation perspective. "
    "Return 3 bullets: code path, validation commands, and one failure mode. "
    "Do not claim measured speedup, cache hits, token savings, or cost savings.\n\n"
    "Task:\n{task}"
)

PROMPT_VLLM_SYNTHESIS = (
    "Combine the two agent reports into a concise live-demo plan. Treat the "
    "agent reports as review notes, not measured evidence. State what runs in "
    "parallel, what runs on the registered vLLM route, and what metrics should "
    "be inspected after execution.\n\n"
    "Task:\n{task}\n\n"
    "Claude report:\n{claude_report}\n\n"
    "Codex report:\n{codex_report}"
)

PROMPT_VLLM_CHECKLIST = (
    "Convert this plan into a Markdown checklist for a demo operator. Use only "
    "these headings: Graph Metrics, Node Metrics, Backend Graph Telemetry, "
    "Agent Output Caveats. Mention only these evidence sources: "
    "runtime.execution.nodes_executed, runtime.execution.nodes_failed, "
    "runtime.token_accounting.total.call_count, "
    "runtime.graph_metrics.graph.processes.spawn_count, "
    "backends.graphs[].registered, backends.graphs[].node_count, session "
    "traces, and the vLLM probe result. Keep it under 8 bullets and do not "
    "mention optimization claims.\n\n"
    "{vllm_synthesis}"
)

OUTPUT_TEMPLATE = (
    "=== APXM Gemma 4 Workflow Metrics Demo ===\n"
    "backend={backend}\n"
    "model={model}\n\n"
    "{vllm_checklist}"
)

DEFAULT_SHOWCASE_TASK = (
    "Evaluate APXM's value for running a workflow where Claude and Codex "
    "inspect the same engineering problem in parallel, then Gemma 4 on the "
    "registered vLLM backend synthesizes the validation checklist."
)


# ---------------------------------------------------------------------------
# Graph
# ---------------------------------------------------------------------------


@compile(default_route=SHOWCASE_ROUTE)
def review_synthesis_skill(g: GraphRecorder, task: str):
    """Two ACP agents in parallel, then one Gemma synthesis on vLLM."""

    _ = task  # passed to nodes via runtime arg; bound here for documentation
    cwd = os.environ.get(ENV_SHOWCASE_CWD, agent_cwd())

    claude_agent = g.spawn(NODE_CLAUDE_AGENT, profile=claude, cwd=cwd)
    codex_agent = g.spawn(NODE_CODEX_AGENT, profile=codex, cwd=cwd)

    claude_agent.ask(PROMPT_CLAUDE_ARCHITECT)
    codex_agent.ask(PROMPT_CODEX_REVIEWER)

    g.think(name=NODE_VLLM_SYNTHESIS, prompt=PROMPT_VLLM_SYNTHESIS)
    g.reason(name=NODE_VLLM_CHECKLIST, prompt=PROMPT_VLLM_CHECKLIST)

    report = g.print(
        name=NODE_PRINT_REPORT,
        message=OUTPUT_TEMPLATE.format(
            backend=SHOWCASE_ROUTE.backend,
            model=SHOWCASE_ROUTE.model,
            vllm_checklist="{vllm_checklist}",
        ),
    )
    g.done(report)


if __name__ == "__main__":
    if emit_air_if_requested(review_synthesis_skill):
        raise SystemExit(0)

    import apxm

    task = os.environ.get(ENV_SHOWCASE_TASK, DEFAULT_SHOWCASE_TASK)
    result = apxm.run(review_synthesis_skill(task))
    print(result.content)
