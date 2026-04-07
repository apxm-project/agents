#!/usr/bin/env python3
"""apxm_planner.py - Strategic planning council

4 parallel analysts -> council + adversarial -> chair decision

Usage: python3 -m examples.python.workflows.planner.apxm_planner
"""

from apxm.graph import compile, GraphRecorder
import os


@compile()
def planner(g: GraphRecorder):
    """Strategic planning council with multiple analysts and chair."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn all agents
    team = g.team("planning_council")
    strategist = team.add("strategist", profile="claude", cwd=cwd)
    skeptic = team.add("skeptic", profile="claude", cwd=cwd)
    technologist = team.add("technologist", profile="claude", cwd=cwd)
    marketeer = team.add("marketeer", profile="claude", cwd=cwd)
    council = g.spawn("council", profile="claude", cwd=cwd)
    chair = g.spawn("chair", profile="claude", cwd=cwd)

    # Get plan name
    plan_name = g.ask(
        "plan_name",
        "What strategic plan or product idea should the council analyze? Be specific."
    )

    # Four parallel analyst streams
    strategist.ask(
        "STRATEGIST analyzing: {0}. Cover WHY NOW, THE GAP, THE MOAT, THE WEDGE, LONG-TERM. "
        "400-600 words with headers."
    )
    plan_name | strategist.get_last_node()

    skeptic.ask(
        "SKEPTIC analyzing: {0}. Find HARDEST PART, MISSING PIECES, SCOPE TRAP, FATAL RISK, "
        "HONEST TIMELINE. 400-600 words."
    )
    plan_name | skeptic.get_last_node()

    technologist.ask(
        "TECHNOLOGIST analyzing: {0}. Assess APXM FIT, EXTERNAL DEPS, BUILD ORDER, "
        "INCREMENTAL v0.1, GAPS. 400-600 words."
    )
    plan_name | technologist.get_last_node()

    marketeer.ask(
        "MARKETEER analyzing: {0}. Find THE BUYER, THE PAIN, THE SWITCH, FIRST 10 CUSTOMERS, "
        "PRICING MODEL. 400-600 words."
    )
    plan_name | marketeer.get_last_node()

    # Synthesize all 4 reports
    council_prompt = g.think(
        "council_prompt",
        "EXPERT COUNCIL CHAIR. Synthesize 4 analyst reports:\n\n"
        "STRATEGIST:\n{0}\n\nSKEPTIC:\n{1}\n\nTECHNOLOGIST:\n{2}\n\nMARKETEER:\n{3}\n\n"
        "Structure:\n## CONVERGENCE\n## DIVERGENCE\n## TENSIONS\n## KEY DECISIONS\n## COUNCIL VERDICT"
    )
    strategist.get_last_node() | council_prompt
    skeptic.get_last_node() | council_prompt
    technologist.get_last_node() | council_prompt
    marketeer.get_last_node() | council_prompt

    # Council synthesizes
    council.ask("{0}")
    council_prompt | council.get_last_node()

    # Adversarial stress test
    council.ask(
        "Stress-test: 1) What single assumption, if wrong, kills everything? "
        "2) What could a large tech co ship in 6 months that makes this irrelevant? "
        "3) Final verdict?"
    )

    # Chair makes final decision
    chair_prompt = g.think(
        "chair_prompt",
        "CHAIR with fresh eyes. Full package:\n\n"
        "STRATEGIST:\n{0}\n\nSKEPTIC:\n{1}\n\nCOUNCIL:\n{2}\n\nADVERSARIAL:\n{3}\n\n"
        "Decide:\n## DECISION (GO/NO-GO/PIVOT)\n## THE WEDGE\n## 3 NEXT STEPS\n## THE ONE THING"
    )
    strategist.get_last_node() | chair_prompt
    skeptic.get_last_node() | chair_prompt
    council.get_last_node() | chair_prompt

    chair.ask("{0}")
    chair_prompt | chair.get_last_node()

    # Print and return
    output = g.print_(
        "output",
        message="=== APXM PLANNER DECISION: {0} ===\n\n{1}"
    )
    plan_name | output
    chair.get_last_node() | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(planner._graph.to_air())
