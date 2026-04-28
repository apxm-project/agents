#!/usr/bin/env python3
"""Case 02 — Checkout Context Pruning.

What this case shows
--------------------
A graph that *intentionally* extracts five separate contexts (checkout, API,
observability, compliance, rollout) but the final decision prompt references
only `{checkout_context}`. The other four data edges are stale.

- O0 keeps every branch (5 LLM calls upstream + 1 decision = 6 calls).
- Default O2 proves four operands are dead, removes their edges, and lets the
  canonical cleanup pass drop the now-unused upstream LLM nodes (2 calls).

Why it matters
--------------
This is the **token / call / capacity** demo. It is the source of the
`6 → 2 LLM calls` story when fresh `runtime-o0-o2.csv` rows reproduce it.

Caveat
------
This is *not* a latency promise. The four removed branches were parallel-able,
so wall-clock latency may or may not move with token count. The deck must show
the call/token deltas, not a wall-clock chart.
"""

from __future__ import annotations

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

from apxm import GraphRecorder, compile, emit_air_if_requested  # noqa: E402
from apxm.constants import INPUT_NAMES  # noqa: E402

from shared.routes import VLLM, BENCHMARK_ROUTE  # noqa: E402


# ---------------------------------------------------------------------------
# Prompts — five context branches plus one decision template.
# ---------------------------------------------------------------------------

CHECKOUT_PROMPT = (
    "Extract six concise facts about checkout idempotency from this "
    "production note: retries after provider timeouts can publish two "
    "order_authorized events for one cart; the previous key used cart_id "
    "and payment_intent_version; the new key uses order_id and attempt_no; "
    "the hotfix cannot require a database migration; duplicate attempts "
    "must still leave audit records; customer notifications must stay "
    "under a two minute end-to-end target."
)

API_PROMPT = (
    "Extract six concise facts about unrelated API authorization work: "
    "service tokens can call batch import endpoints; every denied request "
    "must emit actor, route, and reason; mixed cookie plus service-token "
    "requests are under-tested; middleware overhead must stay under five "
    "milliseconds p95; customer identifiers and billing metadata are "
    "present in import payloads."
)

OBSERVABILITY_PROMPT = (
    "Extract six concise facts about unrelated observability work: "
    "dashboards track p50, p95, and p99 latency; alerts fire on queue "
    "depth and error budget burn; trace sampling is one percent for "
    "normal traffic and one hundred percent during incident windows; "
    "metrics are retained for thirteen months."
)

COMPLIANCE_PROMPT = (
    "Extract six concise facts about unrelated compliance work: SOC2 "
    "evidence requires change approval, access review, incident timeline, "
    "customer-data minimization, encryption proof, and audit-log retention; "
    "new logs must avoid plaintext payment identifiers."
)

ROLLOUT_PROMPT = (
    "Extract six concise facts about unrelated rollout work: deploy API "
    "changes before worker changes; canary one percent of traffic; watch "
    "checkout confirmation latency, duplicate-event rate, refund tickets, "
    "and manual fulfillment review; rollback if duplicate rate rises."
)

DECISION_PROMPT = (
    "Checkout evidence:\n{checkout_context}\n\n"
    "Write one root cause, one hotfix action, and one regression test. "
    "Do not use API, observability, compliance, or rollout notes."
)


# ---------------------------------------------------------------------------
# Graph
# ---------------------------------------------------------------------------


@compile(default_provider=VLLM, default_route=BENCHMARK_ROUTE)
def checkout_context_pruning(g: GraphRecorder):
    """Five context branches feed one decision; only one is referenced."""

    checkout_context = g.ask(name="Checkout_Context", prompt=CHECKOUT_PROMPT, temperature=0.0)
    api_context = g.ask(name="Api_Context", prompt=API_PROMPT, temperature=0.0)
    observability_context = g.ask(name="Observability_Context", prompt=OBSERVABILITY_PROMPT, temperature=0.0)
    compliance_context = g.ask(name="Compliance_Context", prompt=COMPLIANCE_PROMPT, temperature=0.0)
    rollout_context = g.ask(name="Rollout_Context", prompt=ROLLOUT_PROMPT, temperature=0.0)

    decision = g.think(
        name="Checkout_Decision",
        prompt=DECISION_PROMPT,
        temperature=0.0,
        **{
            INPUT_NAMES: [
                "checkout_context",
                "api_context",
                "observability_context",
                "compliance_context",
                "rollout_context",
            ]
        },
    )

    # Stale data edges. O0 carries them through. Default O2 prunes them
    # because the decision template references only {checkout_context}.
    g.add_edge(api_context, decision)
    g.add_edge(observability_context, decision)
    g.add_edge(compliance_context, decision)
    g.add_edge(rollout_context, decision)

    output = g.print(
        name="Print_Context_Pruning_Result",
        message=(
            "=== APXM CONTEXT PRUNING BENCHMARK ===\n\n"
            "Decision:\n{decision}\n\n"
            "O0 keeps five context extraction branches. "
            "O2 removes the four branches that are not referenced by the "
            "final synthesis prompt."
        ),
    )
    g.done(output)


if __name__ == "__main__":
    if emit_air_if_requested(checkout_context_pruning):
        raise SystemExit(0)
    print(checkout_context_pruning.to_air())
