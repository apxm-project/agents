#!/usr/bin/env python3
"""negotiate_consensus.py - Two agents negotiate to reach consensus

Usage: python3 -m examples.python.patterns.multi-agent-negotiate.negotiate_consensus
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def negotiate_consensus(g: GraphRecorder):
    """Two agents negotiate a technical decision to reach consensus."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn negotiating agents
    agent_a = g.spawn("agent_a", profile="claude", cwd=cwd)
    agent_b = g.spawn("agent_b", profile="claude", cwd=cwd)

    # Define negotiation topic
    topic = g.ask(
        "topic",
        template="What technical architecture decision should we negotiate? "
        "Describe a specific choice with trade-offs."
    )

    # Initial proposals (parallel)
    proposal_a_comm = g.communicate(
        "proposal_a",
        target_agent="agent_a",
        message="You are Agent A. Propose your preferred solution for: {0}. "
        "Be specific about your recommendation and why."
    )
    topic | proposal_a_comm

    proposal_b_comm = g.communicate(
        "proposal_b",
        target_agent="agent_b",
        message="You are Agent B. Propose your preferred solution for: {0}. "
        "Be specific about your recommendation and why."
    )
    topic | proposal_b_comm

    # Negotiation round: A responds to B's proposal
    response_a_comm = g.communicate(
        "response_a",
        target_agent="agent_a",
        message="Agent B proposed: {0}\n\n"
        "Respond: do you agree, partially agree, or disagree? What compromise can you offer?"
    )
    proposal_b_comm | response_a_comm

    # Consensus: B responds to A's proposal and response
    consensus_comm = g.communicate(
        "consensus",
        target_agent="agent_b",
        message="Agent A proposed: {0} and responded: {1}\n\n"
        "Can you reach consensus? State the agreed solution."
    )
    proposal_a_comm | consensus_comm
    response_a_comm | consensus_comm

    # Summarize the negotiation
    summary = g.think(
        "summary",
        template="Two agents negotiated. Summarize the final consensus:\n\n"
        "PROPOSAL A:\n{0}\n\nPROPOSAL B:\n{1}\n\nCONSENSUS:\n{2}"
    )
    proposal_a_comm | summary
    proposal_b_comm | summary
    consensus_comm | summary

    # Print and return
    output = g.print_("output", message="=== CONSENSUS ===\n{0}")
    summary | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(negotiate_consensus._graph.to_air())
