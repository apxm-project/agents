#!/usr/bin/env python3
"""negotiate_consensus.py - Two agents negotiate to reach consensus

Usage: python3 -m examples.python.patterns.multi-agent-negotiate.negotiate_consensus
"""

from apxm import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def negotiate_consensus(g: GraphRecorder):
    """Two agents negotiate a technical decision to reach consensus."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn negotiating agents
    agent_a = g.spawn("agent_a", profile=claude, cwd=cwd)
    agent_b = g.spawn("agent_b", profile=claude, cwd=cwd)

    # Define negotiation topic
    topic = g.ask(
        "topic",
        "What technical architecture decision should we negotiate? "
        "Describe a specific choice with trade-offs."
    )

    # Initial proposals (parallel)
    proposal_a_comm = g.communicate(
        "proposal_a",
        target_agent="agent_a",
        message="You are Agent A. Propose your preferred solution for: {topic}. "
        "Be specific about your recommendation and why."
    )

    proposal_b_comm = g.communicate(
        "proposal_b",
        target_agent="agent_b",
        message="You are Agent B. Propose your preferred solution for: {topic}. "
        "Be specific about your recommendation and why."
    )

    # Negotiation round: A responds to B's proposal
    response_a_comm = g.communicate(
        "response_a",
        target_agent="agent_a",
        message="Agent B proposed: {proposal_b_comm}\n\n"
        "Respond: do you agree, partially agree, or disagree? What compromise can you offer?"
    )

    # Consensus: B responds to A's proposal and response
    consensus_comm = g.communicate(
        "consensus",
        target_agent="agent_b",
        message="Agent A proposed: {proposal_a_comm} and responded: {response_a_comm}\n\n"
        "Can you reach consensus? State the agreed solution."
    )

    # Summarize the negotiation
    summary = g.think(
        "summary",
        "Two agents negotiated. Summarize the final consensus:\n\n"
        "PROPOSAL A:\n{proposal_a_comm}\n\nPROPOSAL B:\n{proposal_b_comm}\n\nCONSENSUS:\n{consensus_comm}"
    )

    # Print and return
    output = g.print("=== CONSENSUS ===\n{summary}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(negotiate_consensus._graph.to_air())
