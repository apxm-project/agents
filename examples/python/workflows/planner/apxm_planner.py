#!/usr/bin/env python3
"""apxm_planner.py - Strategic planning council

4 parallel analysts -> council + adversarial -> chair decision

Usage: python3 -m examples.python.workflows.planner.apxm_planner
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def planner(g: GraphRecorder):
    """Strategic planning council with multiple analysts and chair."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn all agents
    team = g.team("planning_council")
    strategist = team.add("strategist", profile=claude, cwd=cwd)
    skeptic = team.add("skeptic", profile=claude, cwd=cwd)
    technologist = team.add("technologist", profile=claude, cwd=cwd)
    marketeer = team.add("marketeer", profile=claude, cwd=cwd)
    council = g.spawn("council", profile=claude, cwd=cwd)
    chair = g.spawn("chair", profile=claude, cwd=cwd)

    # Get plan name
    plan_name = g.ask(
        "plan_name",
        "What strategic plan or product idea should the council analyze? Be specific."
    )

    # Four parallel analyst streams
    strategist.ask(
        "STRATEGIST analyzing: {plan_name}. Cover WHY NOW, THE GAP, THE MOAT, THE WEDGE, LONG-TERM. "
        "400-600 words with headers."
    )

    skeptic.ask(
        "SKEPTIC analyzing: {plan_name}. Find HARDEST PART, MISSING PIECES, SCOPE TRAP, FATAL RISK, "
        "HONEST TIMELINE. 400-600 words."
    )

    technologist.ask(
        "TECHNOLOGIST analyzing: {plan_name}. Assess APXM FIT, EXTERNAL DEPS, BUILD ORDER, "
        "INCREMENTAL v0.1, GAPS. 400-600 words."
    )

    marketeer.ask(
        "MARKETEER analyzing: {plan_name}. Find THE BUYER, THE PAIN, THE SWITCH, FIRST 10 CUSTOMERS, "
        "PRICING MODEL. 400-600 words."
    )

    # Get all analyst results
    strategist_result = strategist.get_last_node()
    skeptic_result = skeptic.get_last_node()
    technologist_result = technologist.get_last_node()
    marketeer_result = marketeer.get_last_node()

    # Synthesize all 4 reports
    council_prompt = g.think(
        "council_prompt",
        "EXPERT COUNCIL CHAIR. Synthesize 4 analyst reports:\n\n"
        "STRATEGIST:\n{strategist_result}\n\nSKEPTIC:\n{skeptic_result}\n\nTECHNOLOGIST:\n{technologist_result}\n\nMARKETEER:\n{marketeer_result}\n\n"
        "Structure:\n## CONVERGENCE\n## DIVERGENCE\n## TENSIONS\n## KEY DECISIONS\n## COUNCIL VERDICT"
    )

    # Council synthesizes
    council.ask("{council_prompt}")
    council_synthesis = council.get_last_node()

    # Adversarial stress test
    council.ask(
        "Stress-test: 1) What single assumption, if wrong, kills everything? "
        "2) What could a large tech co ship in 6 months that makes this irrelevant? "
        "3) Final verdict?"
    )
    adversarial = council.get_last_node()

    # Chair makes final decision
    chair_prompt = g.think(
        "chair_prompt",
        "CHAIR with fresh eyes. Full package:\n\n"
        "STRATEGIST:\n{strategist_result}\n\nSKEPTIC:\n{skeptic_result}\n\nCOUNCIL:\n{council_synthesis}\n\nADVERSARIAL:\n{adversarial}\n\n"
        "Decide:\n## DECISION (GO/NO-GO/PIVOT)\n## THE WEDGE\n## 3 NEXT STEPS\n## THE ONE THING"
    )

    chair.ask("{chair_prompt}")
    chair_decision = chair.get_last_node()

    # Print and return
    output = g.print("=== APXM PLANNER DECISION: {plan_name} ===\n\n{chair_decision}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(planner._graph.to_air())
