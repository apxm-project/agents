#!/usr/bin/env python3
"""autonomous_agent.py — goal-directed agent whose loop lives in the program.

`ais.autonomous` iterates plan → act → evaluate until the goal is satisfied or
`max_iterations` is reached. Tools (web + skills) run inside each act step.

Run (requires a running apxm-server):
    dekk agents execute examples/python/conversational/autonomous_agent.py --emit-air > agent.air
    apxm run agent.air --server http://127.0.0.1:18800

Validate without a server:
    PYTHONPATH=crates/compiler/frontend/python \\
        python3 examples/python/conversational/autonomous_agent.py --validate
"""

from apxm import GraphRecorder, ToolGroup, compile

PERSONA = (
    "You are APXM Assistant, a precise, autonomous agent. Pursue the goal step "
    "by step: discover a relevant skill by description when one fits, use tools "
    "only when they add information, and stop as soon as the goal is satisfied."
)


@compile()
def autonomous_agent(g: GraphRecorder, goal: str):
    """A goal-directed agent whose loop lives in the program (AUTONOMOUS)."""
    result = g.autonomous(
        prompt=f"{PERSONA}\n\nGoal: {{goal}}",
        max_iterations=8,
        capability_groups=[ToolGroup.WEB, ToolGroup.SKILLS],
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
