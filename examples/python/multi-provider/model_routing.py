#!/usr/bin/env python3
"""model_routing.py -- Route tasks to the right model

Different tasks have different requirements: fast models for triage,
powerful models for deep analysis, local models for sensitive data.
APXM's per-node model routing lets you optimize cost, quality, and
privacy in a single workflow.

Usage: dekk apxm execute examples/python/multi-provider/model_routing.py
"""

from apxm import compile, GraphRecorder


@compile()
def model_routing(g: GraphRecorder):
    """Route tasks to appropriate models: fast for triage, powerful for analysis."""

    ticket = g.text(
        "ticket",
        value=(
            "Customer Ticket #12847\n"
            "Subject: Cannot access premium features after upgrade\n"
            "I upgraded 3 days ago (TXN-9481723) but still can't access "
            "the analytics dashboard or export features."
        )
    )

    # FAST MODEL: triage -- cheap model for quick classification
    triage = g.ask(
        "triage",
        "{ticket}\n\n"
        "Classify this ticket:\n"
        "- Priority: [LOW/MEDIUM/HIGH/URGENT]\n"
        "- Category: [BILLING/TECHNICAL/BUG]\n"
        "- Sentiment: [POSITIVE/NEUTRAL/FRUSTRATED]\n"
        "Respond in 3 lines."
    )

    # LOCAL MODEL: data extraction -- keep sensitive data on-prem
    extracted = g.think(
        "extracted",
        "{ticket}\n\nExtract as JSON: email, transaction_id, plan, issue_type, days_since_issue."
    )

    # POWERFUL MODEL: deep analysis -- quality matters here
    solution = g.reason(
        "solution",
        "Ticket: {ticket}\nTriage: {triage}\nData: {extracted}\n\n"
        "Generate: root cause, remediation steps, DB queries to check status, "
        "and a customer communication template."
    )

    # FAST MODEL: formatting -- cheap model for template output
    response = g.ask(
        "response",
        "Solution: {solution}\n\n"
        "Write a friendly 200-word customer email. Be empathetic and clear."
    )

    report = g.merge("report", triage, extracted, solution, response)

    output = g.print(
        "TRIAGE:\n{triage}\n\n"
        "EXTRACTED:\n{extracted}\n\n"
        "SOLUTION:\n{solution}\n\n"
        "RESPONSE:\n{response}"
    )
    output >> report
    g.done(report)


if __name__ == "__main__":
    print(model_routing._graph.to_air())
