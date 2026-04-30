#!/usr/bin/env python3
"""Case 01 — ReviewSynthesis Skill (Pinned-Prefix Fanout).

What this case shows
--------------------
A reusable review skill that lets the APXM compiler discover a shared
prompt prefix across a 6-way fanout and lower it to vLLM as a pinned
KV-cache region. Two ACP sub-agents (Claude as architect, Codex as
implementation reviewer) run in parallel for narrative; the vLLM portion
is the optimization story.

Graph shape
-----------
    review_corpus (tool, static ~2 KB)
        │
        ├─→ aspect_security      ┐
        ├─→ aspect_performance   │
        ├─→ aspect_reliability   │ 6 vLLM `ask` nodes, all using the same
        ├─→ aspect_compliance    │ leading template segment + {corpus} in
        ├─→ aspect_ux            │ position 1, only diverging at the end
        ├─→ aspect_cost          ┘
        │
    claude_architect (ACP) ──┐ both ACP nodes run in parallel with the
    codex_reviewer   (ACP) ──┤ fanout, never gating it
        │                    │
        └─→ gemma4_synthesis (think) ──→ gemma4_validation_checklist (reason)
                                                    │
                                                    ▼
                                          print_metrics_report

Why it matters
--------------
- The 6 fanout templates share a bit-identical leading segment that
  consumes `{corpus}` before diverging. At O2,
  `SharedPrefixAnalysis` detects this, stamps `shared_prefix_group` and
  `shared_prefix_est_tokens`, and the runtime registers the graph + sets
  `pin_policy.mode="prefix"` on the vLLM HTTP body so the fork pins KV
  blocks instead of letting them get evicted under pressure.
- At O0 the pass does not run, no pin hint reaches vLLM, and the fork
  falls back to best-effort prefix caching with no pin guarantee.

Claim metrics (the only ones to quote from this case)
-----------------------------------------------------
- `cached_input_tokens` — vLLM-reported cached input across the fanout
- `pinned_blocks`, `pinned_handles` — APXM graph-status snapshot
- `observed_critical_path_ms` — runtime-observed critical path through
  the vLLM fanout
- `wall_ms`, `graph_duration_ms` — end-to-end and graph-internal timings

Do NOT quote O2 token-saved or call-count drops from this case — that is
case 02's claim. This case shows *prefix pinning works end-to-end*.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

# Inline sys.path bootstrap. The workflow must run from any cwd via
# `dekk apxm compile <file>` or `python3 <file>`, so we cannot rely on
# `shared/` already being importable. Walk up to the APXM repo root, then
# inject the frontend package and the demo root onto sys.path before any
# `apxm.*` or `shared.*` imports.
_HERE = Path(__file__).resolve()
_REPO_ROOT = next(
    (p for p in (_HERE, *_HERE.parents)
     if (p / "Cargo.toml").exists() and (p / "crates").is_dir()),
    Path.cwd().resolve(),
)
_FRONTEND = _REPO_ROOT / "crates" / "compiler" / "apxm-frontend" / "python"
_DEMO_ROOT = _HERE.parents[1]
for _p in (str(_FRONTEND), str(_DEMO_ROOT)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from apxm import (  # noqa: E402  (import after sys.path bootstrap)
    GraphRecorder,
    agent_cwd,
    compile,
    emit_air_if_requested,
    tool,
)
from apxm._generated.agents import claude, codex  # noqa: E402

from shared.routes import SHOWCASE_ROUTE  # noqa: E402


# ---------------------------------------------------------------------------
# Node names — keep stable; benchmark CSVs reference them.
# ---------------------------------------------------------------------------

NODE_CLAUDE_AGENT = "claude_architect"
NODE_CODEX_AGENT = "codex_reviewer"
NODE_REVIEW_CORPUS = "review_corpus"
NODE_VLLM_SYNTHESIS = "gemma4_synthesis"
NODE_VLLM_CHECKLIST = "gemma4_validation_checklist"
NODE_PRINT_REPORT = "print_metrics_report"

ENV_SHOWCASE_CWD = "APXM_SHOWCASE_CWD"
ENV_SHOWCASE_TASK = "APXM_SHOWCASE_TASK"

ASPECT_TOKEN_BUDGET = 600

# ---------------------------------------------------------------------------
# Static corpus — deterministic across runs so the prefix hashes identically
# every iteration. ~2 KB of plain text; large enough for the prefill cost to
# dominate per-call latency on a stock open-weights model.
# ---------------------------------------------------------------------------

REVIEW_CORPUS = """
Platform context.

The platform under review is an order-management service that processes
high-volume retail checkout traffic. The service is deployed across three
regions, fronted by an API gateway, and writes order state to a primary
relational store with a downstream event bus. Checkout traffic is bursty,
peaks at three times the steady-state mean, and is sensitive to tail latency
above 900 ms. Inventory reservation, fulfillment routing, and customer
notification are all driven by an order_authorized event published by the
payment orchestrator.

Recent incident summary.

A recent production incident produced duplicate order_authorized events
during a payment provider degradation. The checkout API retried the
provider call after a 700 ms timeout while the original authorization was
still in flight. The payment orchestrator was not idempotent on retry,
which caused two events with different event ids and the same cart id.
Inventory reservation deduplicates by order id rather than cart id, which
allowed both events to reserve stock. The fulfillment router created two
pick tickets for the same cart when the duplicate events arrived more than
forty seconds apart. The notification worker delivered two confirmations
to the customer.

Constraints the review must respect.

The platform must keep customer-visible confirmations under one second at
the ninety-ninth percentile, must never publish a fulfillment instruction
that would create a duplicate physical shipment, and must keep the audit
log complete enough that compliance can reconstruct any single order from
its raw events. The on-call team has limited budget for new infrastructure
this quarter; structural fixes that require a new datastore or a new
broker tier are out of scope. Fixes that require coordination across the
checkout-api, payment-orchestrator, inventory-reservation, fulfillment-
router, and notification-worker teams are in scope but should be flagged
as such.

Review playbook.

Each reviewer focuses on exactly one aspect of the platform. Reviewers
must read the context above, identify the most important risk in their
aspect, propose one concrete change, and name one observable signal that
would let the on-call team verify the change is working in production.
Reviewers must not invent measured numbers. Reviewers must keep responses
short and structured.
""".strip()


# ---------------------------------------------------------------------------
# Tools
# ---------------------------------------------------------------------------


@tool
def review_corpus_tool() -> str:
    """Return the static review corpus.

    Returning a constant string keeps the prefix bit-identical across runs,
    which is what `SharedPrefixAnalysis` and the vLLM fork's prefix matcher
    both need to detect a cache hit.
    """
    return REVIEW_CORPUS


# ---------------------------------------------------------------------------
# Prompt construction
# ---------------------------------------------------------------------------

# Every aspect node uses the SAME leading segment. Only the suffix
# (focus + question) differs. Because `{corpus}` is the only operand and it
# appears before the divergence point, `SharedPrefixAnalysis` groups the
# six nodes into one shared_prefix_group at O2.
ASPECT_PREFIX = (
    "You are one of six independent reviewers of a software platform. "
    "Read the corpus, then answer for your assigned aspect only. Do not "
    "invent measured numbers, do not claim cache hits or speedups, and "
    "do not reference the other reviewers.\n\n"
    "Corpus:\n{corpus}\n\n"
)

ASPECT_RESPONSE_FORMAT = (
    "Return three short lines:\n"
    "  RISK: <one sentence>\n"
    "  CHANGE: <one sentence>\n"
    "  SIGNAL: <one observable production signal>"
)

PROMPT_ASPECT_SECURITY = (
    ASPECT_PREFIX + "Aspect: security and audit.\n" + ASPECT_RESPONSE_FORMAT
)
PROMPT_ASPECT_PERFORMANCE = (
    ASPECT_PREFIX + "Aspect: latency and throughput at peak load.\n" + ASPECT_RESPONSE_FORMAT
)
PROMPT_ASPECT_RELIABILITY = (
    ASPECT_PREFIX + "Aspect: idempotency, retries, and duplicate prevention.\n" + ASPECT_RESPONSE_FORMAT
)
PROMPT_ASPECT_COMPLIANCE = (
    ASPECT_PREFIX + "Aspect: audit completeness and order reconstruction.\n" + ASPECT_RESPONSE_FORMAT
)
PROMPT_ASPECT_UX = (
    ASPECT_PREFIX + "Aspect: customer-visible behavior on duplicate confirmations.\n" + ASPECT_RESPONSE_FORMAT
)
PROMPT_ASPECT_COST = (
    ASPECT_PREFIX + "Aspect: cost of duplicate fulfillment and refund operations.\n" + ASPECT_RESPONSE_FORMAT
)


# ACP agents — narrative context only; not on the vLLM critical path.
PROMPT_CLAUDE_ARCHITECT = (
    "Read the platform context below from a system architecture perspective. "
    "Return three bullets: graph boundary, observability evidence, one risk. "
    "Do not invent measured numbers. Do not claim speedup or cache effects.\n\n"
    "Context:\n{corpus}\n\n"
    "Task framing:\n{task}"
)

PROMPT_CODEX_REVIEWER = (
    "Read the platform context below from an implementation perspective. "
    "Return three bullets: code path, validation command, one failure mode. "
    "Do not invent measured numbers. Do not claim speedup or cache effects.\n\n"
    "Context:\n{corpus}\n\n"
    "Task framing:\n{task}"
)

# Synthesis joins the 6 aspect outputs with the 2 ACP reports.
PROMPT_VLLM_SYNTHESIS = (
    "Combine six aspect reviews and two agent reports into one short live-demo "
    "plan. Treat agent reports as review notes, not measured evidence. State "
    "what runs in parallel, what runs on the registered vLLM route, and what "
    "metrics should be inspected after execution.\n\n"
    "Aspect security:\n{aspect_security}\n\n"
    "Aspect performance:\n{aspect_performance}\n\n"
    "Aspect reliability:\n{aspect_reliability}\n\n"
    "Aspect compliance:\n{aspect_compliance}\n\n"
    "Aspect UX:\n{aspect_ux}\n\n"
    "Aspect cost:\n{aspect_cost}\n\n"
    "Claude architect report:\n{claude_report}\n\n"
    "Codex reviewer report:\n{codex_report}"
)

PROMPT_VLLM_CHECKLIST = (
    "Convert this plan into a Markdown checklist for a demo operator. Use "
    "only these headings: Graph Metrics, Node Metrics, Backend Graph "
    "Telemetry, Agent Output Caveats. Mention only these evidence sources: "
    "runtime.execution.nodes_executed, runtime.execution.nodes_failed, "
    "runtime.token_accounting.total.cached_input_tokens, "
    "runtime.graph_metrics.graph.processes.spawn_count, "
    "backends.graphs[].registered, backends.graphs[].pinned_blocks, "
    "backends.graphs[].pinned_handles, session traces, vLLM probe result. "
    "Keep it under eight bullets and do not mention optimization claims.\n\n"
    "{vllm_synthesis}"
)

OUTPUT_TEMPLATE = (
    "=== APXM Gemma 4 ReviewSynthesis Skill ===\n"
    "backend={backend}\n"
    "model={model}\n"
    "fanout=6 aspect reviews sharing pinned prefix\n\n"
    "{vllm_checklist}"
)

DEFAULT_SHOWCASE_TASK = (
    "Demonstrate a six-way aspect review where the compiler discovers the "
    "shared prefix automatically and the vLLM backend pins it as a KV-cache "
    "region for the duration of the fanout."
)


# ---------------------------------------------------------------------------
# Graph
# ---------------------------------------------------------------------------


@compile(default_route=SHOWCASE_ROUTE)
def review_synthesis_skill(g: GraphRecorder, task: str):
    """ACP fan-out + 6 vLLM aspect reviews sharing a pinned prefix."""

    cwd = os.environ.get(ENV_SHOWCASE_CWD, agent_cwd())

    # Static corpus. Deterministic across runs so prefix bytes match.
    corpus = g.invoke_tool(review_corpus_tool, name=NODE_REVIEW_CORPUS)

    # 6-way vLLM fanout. Each prompt's leading segment is identical and
    # consumes `{corpus}` before diverging. SharedPrefixAnalysis groups
    # them at O2 and stamps shared_prefix_group + shared_prefix_est_tokens.
    aspect_security = g.ask(
        name="aspect_security",
        prompt=PROMPT_ASPECT_SECURITY,
        temperature=0.0,
        token_budget=ASPECT_TOKEN_BUDGET,
    )
    aspect_performance = g.ask(
        name="aspect_performance",
        prompt=PROMPT_ASPECT_PERFORMANCE,
        temperature=0.0,
        token_budget=ASPECT_TOKEN_BUDGET,
    )
    aspect_reliability = g.ask(
        name="aspect_reliability",
        prompt=PROMPT_ASPECT_RELIABILITY,
        temperature=0.0,
        token_budget=ASPECT_TOKEN_BUDGET,
    )
    aspect_compliance = g.ask(
        name="aspect_compliance",
        prompt=PROMPT_ASPECT_COMPLIANCE,
        temperature=0.0,
        token_budget=ASPECT_TOKEN_BUDGET,
    )
    aspect_ux = g.ask(
        name="aspect_ux",
        prompt=PROMPT_ASPECT_UX,
        temperature=0.0,
        token_budget=ASPECT_TOKEN_BUDGET,
    )
    aspect_cost = g.ask(
        name="aspect_cost",
        prompt=PROMPT_ASPECT_COST,
        temperature=0.0,
        token_budget=ASPECT_TOKEN_BUDGET,
    )

    # ACP agents — parallel, not on the vLLM critical path.
    claude_agent = g.spawn(NODE_CLAUDE_AGENT, profile=claude, cwd=cwd)
    codex_agent = g.spawn(NODE_CODEX_AGENT, profile=codex, cwd=cwd)
    claude_report = claude_agent.ask(PROMPT_CLAUDE_ARCHITECT)
    codex_report = codex_agent.ask(PROMPT_CODEX_REVIEWER)

    vllm_synthesis = g.think(
        name=NODE_VLLM_SYNTHESIS,
        prompt=PROMPT_VLLM_SYNTHESIS,
        temperature=0.0,
    )
    vllm_checklist = g.reason(
        name=NODE_VLLM_CHECKLIST,
        prompt=PROMPT_VLLM_CHECKLIST,
        temperature=0.0,
    )

    report = g.print(
        name=NODE_PRINT_REPORT,
        message=OUTPUT_TEMPLATE.format(
            backend=SHOWCASE_ROUTE.backend,
            model=SHOWCASE_ROUTE.model,
            vllm_checklist="{vllm_checklist}",
        ),
    )
    g.done(report)

    # Reference unused locals so static analyzers see they are used —
    # they are consumed by the auto-wired placeholders above.
    _ = (
        aspect_security,
        aspect_performance,
        aspect_reliability,
        aspect_compliance,
        aspect_ux,
        aspect_cost,
        claude_report,
        codex_report,
        vllm_synthesis,
        vllm_checklist,
        task,
    )


if __name__ == "__main__":
    if emit_air_if_requested(review_synthesis_skill):
        raise SystemExit(0)

    import apxm

    task = os.environ.get(ENV_SHOWCASE_TASK, DEFAULT_SHOWCASE_TASK)
    result = apxm.run(review_synthesis_skill(task))
    print(result.content)
