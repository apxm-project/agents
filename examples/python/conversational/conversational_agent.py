#!/usr/bin/env python3
"""conversational_agent.py — per-turn body of a conversational APXM agent.

`apxm chat` drives the outer turn loop; this graph runs once per user message
(one DAG = one turn). It performs task-memory recall, description-based skill
discovery, and a tool-using ASK, then records a fact before returning.

Run (requires a running apxm-server):
    dekk agents execute examples/python/conversational/conversational_agent.py --emit-air > agent.air
    apxm chat --air agent.air --server http://127.0.0.1:18800

Validate without a server:
    PYTHONPATH=crates/compiler/frontend/python \\
        python3 examples/python/conversational/conversational_agent.py --validate
"""

from apxm import GraphRecorder, ToolGroup, compile

PERSONA = (
    "You are APXM Assistant, a precise, helpful conversational agent. "
    "Consult recalled context before answering, prefer a relevant skill when one "
    "fits, use tools only when they add information, and keep replies concise "
    "and well-structured. If you are unsure, say so."
)


@compile()
def conversational_agent(g: GraphRecorder, conversation: str):
    """One conversational turn — the agent body the host loop runs per message.

    Parameters
    ----------
    conversation : str
        The full running transcript, ending in ``User: <msg>\\nAssistant:``,
        supplied by ``apxm chat`` each turn.
    """
    # Recall relevant facts from task memory (deliberate program decision).
    history = g.query_memory(name="recall", query="relevant prior facts", space="stm")

    # Description-based skill discovery via the real `search_skills` capability —
    # ranks installed skills by their description (lexical match over the skill
    # catalogue), so the agent never hard-codes a skill id.
    skill = g.skill_search(name="discover_skill", query="the latest user message")

    # Tool-using ASK; `{history}` and `{skill}` auto-wire data edges above.
    answer = g.ask(
        name="answer",
        prompt=(
            "{conversation}\n\n"
            "Recalled context:\n{history}\n\n"
            "Most relevant skill (by description):\n{skill}"
        ),
        system_prompt=PERSONA,      # middleware may enrich this at runtime
        capability_groups=[ToolGroup.WEB],
    )

    # Record a fact for later turns, fenced so the write is ordered.
    note = g.update_memory(
        name="remember",
        data="a fact worth keeping from this turn",
        key="relevant prior facts",
        space="stm",
    )
    g.add_edge(answer, note)
    commit = g.fence(name="commit")
    g.add_edge(note, commit)

    # The answer is the user-visible reply.
    g.done(source=answer)
    _ = (history, skill)  # referenced via templates; keep linters quiet


if __name__ == "__main__":
    import sys

    if "--validate" in sys.argv:
        from apxm.ir import validate_against_apxm

        result = validate_against_apxm(conversational_agent._graph)
        print("VALID" if result.valid else "INVALID")
        for err in result.errors:
            print(f"  ERROR: {err}")
        for warn in result.warnings:
            print(f"  WARN:  {warn}")
        sys.exit(0 if result.valid else 1)

    print(conversational_agent._graph.to_air())
