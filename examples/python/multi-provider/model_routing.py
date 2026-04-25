#!/usr/bin/env python3
"""model_routing.py -- Route tasks to registered backend roles.

Different tasks have different requirements: fast models for triage,
powerful models for deep analysis, local models for sensitive data.
APXM's per-node model routing lets you optimize cost, quality, and
privacy in a single workflow.

Usage:
  dekk apxm execute examples/python/multi-provider/model_routing.py
"""

from apxm import GraphRecorder, compile
from apxm.backends import select_backend


ROUTE_ALIAS_FAST = "fast"
ROUTE_ALIAS_LOCAL = "local-sensitive"
ROUTE_ALIAS_POWERFUL = "powerful"
ROUTE_ALIAS_FORMATTER = "formatter"


@compile()
def model_routing(g: GraphRecorder):
    """Route tasks to appropriate models: fast for triage, powerful for analysis."""

    fast_route = select_backend(alias=ROUTE_ALIAS_FAST)
    local_route = select_backend(alias=ROUTE_ALIAS_LOCAL)
    powerful_route = select_backend(alias=ROUTE_ALIAS_POWERFUL)
    formatter_route = select_backend(alias=ROUTE_ALIAS_FORMATTER)

    # Fast route: triage -- cheap model for quick classification
    triage = g.ask(
        name="triage",
        prompt="Customer Ticket #12847\n"
        "Subject: Cannot access premium features after upgrade\n"
        "I upgraded 3 days ago (TXN-9481723) but still can't access "
        "the analytics dashboard or export features.\n\n"
        "Classify this ticket:\n"
        "- Priority: [LOW/MEDIUM/HIGH/URGENT]\n"
        "- Category: [BILLING/TECHNICAL/BUG]\n"
        "- Sentiment: [POSITIVE/NEUTRAL/FRUSTRATED]\n"
        "Respond in 3 lines.",
        route=fast_route,
    )

    # Local route: data extraction -- keep sensitive data on-prem
    extracted = g.think(
        name="extracted",
        prompt="Customer Ticket #12847\n"
        "Subject: Cannot access premium features after upgrade\n"
        "I upgraded 3 days ago (TXN-9481723) but still can't access "
        "the analytics dashboard or export features.\n\n"
        "Extract as JSON: email, transaction_id, plan, issue_type, days_since_issue.",
        route=local_route,
    )

    # Powerful route: deep analysis -- quality matters here
    solution = g.reason(
        name="solution",
        prompt="Triage: {triage}\nData: {extracted}\n\n"
        "Generate: root cause, remediation steps, DB queries to check status, "
        "and a customer communication template.",
        route=powerful_route,
    )

    # Formatter route: use the registered model for customer-facing copy.
    response = g.ask(
        name="response",
        prompt="Solution: {solution}\n\n"
        "Write a friendly 200-word customer email. Be empathetic and clear.",
        route=formatter_route,
    )

    report = g.merge("report", triage, extracted, solution, response)

    output = g.print(
        message="TRIAGE:\n{triage}\n\n"
        "EXTRACTED:\n{extracted}\n\n"
        "SOLUTION:\n{solution}\n\n"
        "RESPONSE:\n{response}"
    )
    g.add_edge(output, report, dependency="Control")
    g.done(report)


if __name__ == "__main__":
    import apxm

    result = apxm.run(model_routing())
    print(result.content)
