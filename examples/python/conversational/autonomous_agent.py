#!/usr/bin/env python3
"""autonomous_agent.py — the LOOP element of the vision, as a real APXM program.

    A conversational agent is an APXM program
    (a loop + middleware + prompts that do
     context injection, skill discovery and tool execution).

`conversational_agent.py` is the per-turn body (host drives the turn loop). This
example makes **the loop itself part of the program**: `ais.autonomous` is a real
goal-directed loop — plan → act → evaluate, iterating until the goal is satisfied
or `max_iterations` — so the iteration lives in the APXM graph, not the host.

Mapping to the vision sentence:
  - LOOP              `g.autonomous(...)` — a genuine iterating runtime loop.
  - MIDDLEWARE        context injection + conversation-memory + token-budget are
                      applied by the runtime middleware chain around every node
                      (registered server-side; not written into the graph).
  - PROMPTS           the persona + goal prompt below.
  - CONTEXT INJECTION the system prompt is enriched from AGENTS.md/memory by the
                      context-injection layer; the loop also accrues memory.
  - SKILL DISCOVERY   the `skills` tool group exposes `search_skills` (scoped to
                      the visible set) so the loop can find skills by description.
  - TOOL EXECUTION    the `web` + `skills` tool groups; the act step runs tools.

Run (requires a running apxm-server):
    dekk apxm execute examples/python/conversational/autonomous_agent.py --emit-air > agent.air
    apxm run agent.air --server http://127.0.0.1:18800

Validate without a server:
    PYTHONPATH=crates/compiler/apxm-frontend/python \
        python3 examples/python/conversational/autonomous_agent.py --validate
"""

from apxm import GraphRecorder, compile

PERSONA = (
    "You are APXM Assistant, a precise, autonomous agent. Pursue the goal step "
    "by step: discover a relevant skill by description when one fits, use tools "
    "only when they add information, and stop as soon as the goal is satisfied."
)


@compile()
def autonomous_agent(g: GraphRecorder, goal: str):
    """A goal-directed agent whose LOOP lives in the program (AUTONOMOUS)."""
    # THE LOOP: plan -> act -> evaluate, iterating until the goal is met.
    result = g.autonomous(
        prompt=f"{PERSONA}\n\nGoal: {{goal}}",
        max_iterations=8,
        # TOOL EXECUTION + SKILL DISCOVERY: web tools and the scoped skill-search.
        tool_groups=["web", "skills"],
    )
    g.done(result)


if __name__ == "__main__":
    import sys

    if "--validate" in sys.argv:
        from apxm.ir import validate_against_apxm

        report = validate_against_apxm(autonomous_agent._graph)
        print("VALID" if report.valid else "INVALID")
        for err in report.errors:
            print(f"  ERROR: {err}")
        sys.exit(0 if report.valid else 1)

    print(autonomous_agent._graph.to_air())
