#!/usr/bin/env python3
"""brief_analyzer.py - Analyze app brief -> structured design manifest

Pipeline: 3 sequential analyst agents -> synthesis -> manifest

Usage: python3 -m examples.python.workflows.designer.brief_analyzer
"""

from apxm.graph import compile, GraphRecorder
from apxm._generated.agents import claude
import os


@compile()
def brief_analyzer(g: GraphRecorder):
    """Analyze an app brief and produce a structured design manifest."""
    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Phase 1: Entity analysis
    entity_analyst = g.spawn(
        "entity_analyst",
        profile=claude,
        cwd=cwd
    )
    entity_analyst.ask(
        "You are an Entity and State Machine Analyst for app design.\n\n"
        "App brief: vintage watch marketplace. Sellers list watches with photos and descriptions. "
        "Buyers browse, filter by brand/price/condition, save favorites, and message sellers. "
        "Sellers have a dashboard showing listings, messages, and sales stats. "
        "Buyers can make offers which sellers accept, reject, or counter.\n\n"
        "Output ONLY a JSON object (no markdown): "
        '{{"entities":[...], "user_roles":[...], "entity_states":{{"EntityName":["state1"]}}, "state_transitions":[...]}}'
    )

    # Phase 2: Screen analysis
    screen_analyst = g.spawn(
        "screen_analyst",
        profile=claude,
        cwd=cwd
    )
    screen_analyst.ask(
        "You are a Screen and Navigation Analyst for app design.\n\n"
        "You have entity analysis data. Now produce a complete screen inventory for a vintage watch marketplace.\n\n"
        "Output ONLY a JSON object: "
        '{{"screens":{{"role/capability/screen":["state1"]}}, "shared_screens":["..."], "screen_count":N}}'
    )

    # Phase 3: Component analysis
    component_analyst = g.spawn(
        "component_analyst",
        profile=claude,
        cwd=cwd
    )
    component_analyst.ask(
        "You are a Component and Design System Analyst for app design.\n\n"
        "You have screen analysis data. Now produce a complete component inventory for a vintage watch marketplace.\n\n"
        "Output ONLY a JSON object: "
        '{{"components":[{{"name":"X", "variants":["a"], "states":["normal"]}}], "patterns":["..."], "component_count":N}}'
    )

    # Phase 4: Synthesize manifest
    manifest = g.ask(
        "manifest",
        template="You are a design manifest synthesizer. Produce a structured design manifest for a vintage watch "
        "marketplace app. The app has: sellers listing watches with photos, buyers browsing and filtering, "
        "messaging, offer negotiation, and seller dashboards.\n\n"
        "Produce ONLY this JSON structure (no markdown, no explanation):\n"
        '{"summary":"one paragraph", "stats":{"entities":N, "screens":N, "screen_states":N, "components":N, '
        '"estimated_html_files":N}, "critical_screens":["top 5"], "verification_checklist":["..."], '
        '"risks":["..."], "go_signal":"GO"}'
    )

    # Print and return
    output = g.print("=== APXM-DESIGNER v0.4 MANIFEST ===\n\n{manifest}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(brief_analyzer._graph.to_air())
