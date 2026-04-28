#!/usr/bin/env python3
"""Case 03 — vLLM Backend Hints.

What this case shows
--------------------
A graph deliberately shaped to create vLLM queue pressure: four long background
audit prompts are submitted first, then a short user-visible critical chain
becomes ready while the backend is busy. The compiler stamps the critical
chain with APXM priority hints. The runtime lowers those hints to vLLM
`apxm_hints` so vLLM's priority scheduler can preempt the background queue.

Why it matters
--------------
- Compile-time priority hints survive the lowering and reach the backend.
- The critical chain is short enough to fit a small token budget.
- The whole graph still completes — background work is awaited so the session
  trace stays auditable.

Claim metric
------------
Wall-clock to the `Critical_User_Visible_Result` milestone (NOT whole-graph
duration). The backend probe in `scripts/measure_vllm_hints.py` corroborates
that priority + prefix-cache hints are observed at the vLLM HTTP boundary.
"""

from __future__ import annotations

import time

from shared.bootstrap import bootstrap_paths

bootstrap_paths(__file__)

from apxm import GraphRecorder, compile, emit_air_if_requested, tool  # noqa: E402
from apxm import constants as graph_keys  # noqa: E402

from shared.routes import VLLM, BENCHMARK_ROUTE  # noqa: E402


# ---------------------------------------------------------------------------
# Tunables — cap background prompts and hold critical work briefly so the
# background fanout has time to enter the queue before critical work is ready.
# ---------------------------------------------------------------------------

BACKGROUND_TOKEN_BUDGET = 6000
CRITICAL_TOKEN_BUDGET = 1200
CRITICAL_RELEASE_DELAY_SEC = 0.45

LLM_MEASUREMENT_ATTRS = {
    graph_keys.MEMOIZABLE: False,
    "vllm_cache_salt": "execution",
}


# Inlined dossier — single source of truth for this case. Kept verbatim so
# benchmark traces remain comparable across runs.
INCIDENT_DOSSIER = """
Incident: customer checkout latency and duplicate fulfillment risk.

Business context:
- Region: North America retail checkout.
- Daily order volume: 1.8 million orders.
- Target service level: p99 checkout confirmation under 900 ms.
- Revenue exposure: failed confirmation can trigger cart abandonment,
  duplicate support contacts, and manual fulfillment review.
- Compliance context: order events contain customer identifiers, shipping
  regions, payment intent ids, and warehouse routing metadata.

Current architecture:
- API gateway terminates TLS and forwards checkout requests to checkout-api.
- checkout-api validates carts, creates order rows, and calls payment-orchestrator.
- payment-orchestrator calls the payment service provider and publishes an
  order_authorized event.
- inventory-reservation consumes order_authorized and reserves stock by SKU.
- fulfillment-router consumes order_authorized and assigns a warehouse route.
- notification-worker sends email, SMS, and webhook confirmations.
- analytics-stream copies order events into the data lake.

Observed failure:
- During traffic bursts, checkout-api retries payment-orchestrator after a
  700 ms timeout.
- payment-orchestrator is not idempotent for provider calls when the retry
  arrives after the provider authorization completed.
- Two order_authorized events can be published with different event ids and the
  same cart id.
- inventory-reservation deduplicates by order id, not cart id.
- fulfillment-router can create two pick tickets when duplicate events arrive
  more than 40 seconds apart.
- notification-worker sends both confirmations, creating customer confusion.
""".strip()


# ---------------------------------------------------------------------------
# Tools
# ---------------------------------------------------------------------------


@tool
def incident_dossier() -> str:
    """Return the fixed incident dossier for paired O0/O2 benchmark runs."""
    return INCIDENT_DOSSIER


@tool
def critical_release_gate() -> str:
    """Hold the critical branch long enough for background work to queue."""
    time.sleep(CRITICAL_RELEASE_DELAY_SEC)
    return "critical branch released after background fanout queued"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _pressure_facts(lane: str, count: int = 120) -> str:
    return "\n".join(
        f"{lane} pressure fact {index:04d}: retry storm shard={index % 17}, "
        f"provider_timeout={index % 11}, audit_event={index % 23}, "
        "customer-impact, rollback guardrail, metrics cross-check."
        for index in range(count)
    )


def _background_prompt(lane: str, instruction: str) -> str:
    return (
        f"Background lane {lane}. This is not user-visible critical-path work.\n"
        f"{_pressure_facts(lane)}\n\n"
        "Incident dossier:\n{dossier}\n\n"
        f"{instruction}\n\n"
        "Return exactly two lines:\n"
        f"BACKGROUND_LANE={lane}\n"
        "STATUS=queued_audit_complete"
    )


# ---------------------------------------------------------------------------
# Graph
# ---------------------------------------------------------------------------


@compile(default_provider=VLLM, default_route=BENCHMARK_ROUTE)
def vllm_backend_hints(g: GraphRecorder):
    """Queue-pressure graph that lets APXM priority hints reach vLLM."""

    dossier = g.invoke_tool(incident_dossier, name="Incident_Dossier")
    gate = g.invoke_tool(critical_release_gate, name="Critical_Release_Gate")
    g.add_edge(dossier, gate, dependency=graph_keys.DEPENDENCY_CONTROL)

    background_security = g.ask(
        name="Background_Security_Audit",
        prompt=_background_prompt(
            "security",
            "Find sensitive-data and audit-log risks in the incident response.",
        ),
        temperature=0.0,
        token_budget=BACKGROUND_TOKEN_BUDGET,
        **LLM_MEASUREMENT_ATTRS,
    )
    background_reliability = g.ask(
        name="Background_Reliability_Audit",
        prompt=_background_prompt(
            "reliability",
            "Find retry, idempotency, and queue-drain risks in the mitigation.",
        ),
        temperature=0.0,
        token_budget=BACKGROUND_TOKEN_BUDGET,
        **LLM_MEASUREMENT_ATTRS,
    )
    background_finance = g.ask(
        name="Background_Finance_Audit",
        prompt=_background_prompt(
            "finance",
            "Find refund, dispute, and reconciliation risks in the mitigation.",
        ),
        temperature=0.0,
        token_budget=BACKGROUND_TOKEN_BUDGET,
        **LLM_MEASUREMENT_ATTRS,
    )
    background_rollout = g.ask(
        name="Background_Rollout_Audit",
        prompt=_background_prompt(
            "rollout",
            "Find deployment, canary, rollback, and operator handoff risks.",
        ),
        temperature=0.0,
        token_budget=BACKGROUND_TOKEN_BUDGET,
        **LLM_MEASUREMENT_ATTRS,
    )

    g.ask(
        name="Critical_Triage",
        prompt=(
            "Gate: {gate}\n\n"
            "A checkout incident is active. Return the single safest mitigation "
            "decision in two short bullets."
        ),
        temperature=0.0,
        token_budget=CRITICAL_TOKEN_BUDGET,
        benchmark_milestone="critical_triage",
        **LLM_MEASUREMENT_ATTRS,
    )
    g.think(
        name="Critical_Mitigation_Plan",
        prompt=(
            "Critical triage:\n{critical_triage}\n\n"
            "Convert this into a three-step operator plan. Keep every step short."
        ),
        temperature=0.0,
        token_budget=CRITICAL_TOKEN_BUDGET,
        benchmark_milestone="critical_plan",
        **LLM_MEASUREMENT_ATTRS,
    )
    g.think(
        name="Critical_Executive_Summary",
        prompt=(
            "Mitigation plan:\n{critical_plan}\n\n"
            "Write the user-visible executive decision with one rollback trigger."
        ),
        temperature=0.0,
        token_budget=CRITICAL_TOKEN_BUDGET,
        benchmark_milestone="critical_summary",
        **LLM_MEASUREMENT_ATTRS,
    )

    critical_output = g.print(
        name="Critical_User_Visible_Result",
        message="=== CRITICAL USER-VISIBLE RESULT ===\n\n{critical_summary}",
        benchmark_milestone="critical_result",
    )

    background_findings = g.merge(
        "Background_Findings",
        background_security,
        background_reliability,
        background_finance,
        background_rollout,
    )
    background_output = g.print(
        name="Background_Result",
        message="=== BACKGROUND AUDITS ===\n\n{background_findings}",
    )

    all_work = g.wait_all("All_Work_Complete", critical_output, background_output)
    g.done(all_work)


if __name__ == "__main__":
    if emit_air_if_requested(vllm_backend_hints):
        raise SystemExit(0)
    print(vllm_backend_hints.to_air())
