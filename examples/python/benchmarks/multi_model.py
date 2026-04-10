#!/usr/bin/env python3
"""multi_model.py - Benchmark for per-node model/backend routing

Tests: Per-node model specification and backend routing
Measures: Cost vs quality tradeoffs, latency distribution across models

Usage:
  dekk apxm execute multi_model.apxm -O2

Note: Requires multiple backends configured:
  - Fast model (e.g., gpt-3.5-turbo, claude-haiku)
  - Powerful model (e.g., gpt-4, claude-opus)
  - Local model (e.g., llama via ollama)
"""

from apxm import compile, GraphRecorder


@compile()
def multi_model(g: GraphRecorder):
    """Route different tasks to appropriate models based on requirements."""

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
    # This should use a fast, cheap model (gpt-3.5-turbo, claude-haiku)
    triage = g.ask(
        "triage",
        ticket_text + "\n\n"
        "Classify this support ticket:\n"
        "- Priority: [LOW/MEDIUM/HIGH/URGENT]\n"
        "- Category: [BILLING/TECHNICAL/FEATURE_REQUEST/BUG]\n"
        "- Sentiment: [POSITIVE/NEUTRAL/FRUSTRATED/ANGRY]\n\n"
        "Respond in 3 lines only.",
        # In real usage, would specify: model="gpt-3.5-turbo" or backend="fast"
    )

    # Local model: extract structured data (sensitive - keep local)
    # This should use a local model for data privacy
    extracted_data = g.think(
        "extracted_data",
        "{ticket_text}\n\n"
        "Extract structured information:\n"
        "- Email: \n"
        "- Transaction ID: \n"
        "- Plan: \n"
        "- Issue Type: \n"
        "- Days Since Issue: \n\n"
        "Output as JSON format only.",
        # In real usage, would specify: backend="local-ollama"
    )

    # Powerful model: deep analysis and solution
    # This should use the most capable model (gpt-4, claude-opus)
    solution = g.reason(
        "solution",
        "Ticket: {ticket_text}\n\n"
        "Triage: {triage}\n\n"
        "Extracted Data: {extracted_data}\n\n"
        "Generate a comprehensive solution:\n"
        "1. Root cause analysis\n"
        "2. Immediate remediation steps\n"
        "3. Database queries to check account status\n"
        "4. Customer communication template\n"
        "5. Process improvements to prevent recurrence\n\n"
        "Be thorough and precise.",
        # In real usage, would specify: model="gpt-4" or backend="powerful"
    )

    # Fast model: format customer response
    # Back to fast/cheap model for formatting
    customer_response = g.ask(
        "customer_response",
        "Solution: {solution}\n\n"
        "Create a friendly, professional customer email response.\n"
        "Maximum 200 words. Be empathetic and clear.\n"
        "Include specific next steps.",
        # In real usage, would specify: model="gpt-3.5-turbo"
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
        "=== MULTI-MODEL ROUTING BENCHMARK ===\n\n"
        "TRIAGE (Fast Model):\n{triage}\n\n"
        "---\n\n"
        "EXTRACTED DATA (Local Model - Privacy):\n{extracted_data}\n\n"
        "---\n\n"
        "SOLUTION (Powerful Model - Quality):\n{solution}\n\n"
        "---\n\n"
        "CUSTOMER RESPONSE (Fast Model - Formatting):\n{customer_response}"
    )

    output >> final_output
    g.done(final_output)


if __name__ == "__main__":
    # Output the graph as JSON
    print(multi_model._graph.to_air())
