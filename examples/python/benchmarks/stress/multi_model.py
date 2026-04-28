#!/usr/bin/env python3
"""multi_model.py - Benchmark for per-node model/backend routing

Tests: Per-node model specification and backend routing
Measures: Cost vs quality tradeoffs, latency distribution across models

Usage:
  dekk apxm execute multi_model.air -O2

Note: Requires registered backend model aliases:
  - fast
  - local-sensitive
  - powerful
  - formatter
"""

from apxm import GraphRecorder, compile
from apxm.backends import select_backend


ROUTE_ALIAS_FAST = "fast"
ROUTE_ALIAS_LOCAL = "local-sensitive"
ROUTE_ALIAS_POWERFUL = "powerful"
ROUTE_ALIAS_FORMATTER = "formatter"


@compile()
def multi_model(g: GraphRecorder):
    """Route different tasks to appropriate models based on requirements."""

    fast_route = select_backend(alias=ROUTE_ALIAS_FAST)
    local_route = select_backend(alias=ROUTE_ALIAS_LOCAL)
    powerful_route = select_backend(alias=ROUTE_ALIAS_POWERFUL)
    formatter_route = select_backend(alias=ROUTE_ALIAS_FORMATTER)

    # Input: customer support ticket
    ticket_text = """Customer Support Ticket #12847

        From: user@example.com
        Subject: Cannot access premium features after upgrade

        I upgraded to Premium plan 3 days ago (transaction ID: TXN-9481723)
        but I still cannot access the advanced analytics dashboard or
        export features. My payment went through successfully.

        This is blocking my team's work. Please help urgently.
        """

    # Fast model: initial triage and classification
    triage = g.ask(
        name="triage",
        prompt=ticket_text + "\n\n"
        "Classify this support ticket:\n"
        "- Priority: [LOW/MEDIUM/HIGH/URGENT]\n"
        "- Category: [BILLING/TECHNICAL/FEATURE_REQUEST/BUG]\n"
        "- Sentiment: [POSITIVE/NEUTRAL/FRUSTRATED/ANGRY]\n\n"
        "Respond in 3 lines only.",
        route=fast_route,
    )

    # Local model: extract structured data (sensitive - keep local)
    extracted_data = g.think(
        name="extracted_data",
        prompt=ticket_text + "\n\n"
        "Extract structured information:\n"
        "- Email: \n"
        "- Transaction ID: \n"
        "- Plan: \n"
        "- Issue Type: \n"
        "- Days Since Issue: \n\n"
        "Output as JSON format only.",
        route=local_route,
    )

    # Powerful model: deep analysis and solution
    solution = g.reason(
        name="solution",
        prompt="Ticket: " + ticket_text + "\n\n"
        "Triage: {triage}\n\n"
        "Extracted Data: {extracted_data}\n\n"
        "Generate a comprehensive solution:\n"
        "1. Root cause analysis\n"
        "2. Immediate remediation steps\n"
        "3. Database queries to check account status\n"
        "4. Customer communication template\n"
        "5. Process improvements to prevent recurrence\n\n"
        "Be thorough and precise.",
        route=powerful_route,
    )

    # Fast model: format customer response
    customer_response = g.ask(
        name="customer_response",
        prompt="Solution: {solution}\n\n"
        "Create a friendly, professional customer email response.\n"
        "Maximum 200 words. Be empathetic and clear.\n"
        "Include specific next steps.",
        route=formatter_route,
    )

    # Final report
    final_output = g.merge(
        "final_output",
        triage,
        extracted_data,
        solution,
        customer_response
    )

    output = g.print(
        message="=== MULTI-MODEL ROUTING BENCHMARK ===\n\n"
        "TRIAGE (Fast Model):\n{triage}\n\n"
        "---\n\n"
        "EXTRACTED DATA (Local Model - Privacy):\n{extracted_data}\n\n"
        "---\n\n"
        "SOLUTION (Powerful Model - Quality):\n{solution}\n\n"
        "---\n\n"
        "CUSTOMER RESPONSE (Fast Model - Formatting):\n{customer_response}"
    )

    g.add_edge(output, final_output, dependency="Control")
    g.done(final_output)


if __name__ == "__main__":
    # Output AIR.
    print(multi_model._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(multi_model())
    # print(result.content)
