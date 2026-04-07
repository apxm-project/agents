#!/usr/bin/env python3
"""plan_feature.py - Generate Implementation Plan

Takes a feature request and produces a detailed implementation plan with
gap analysis, risk analysis, crate ordering, and concrete action items.

Graph structure:
- spawn architect (claude) — reads project structure, produces plan
- think: gap_analysis — what's missing vs what exists
- think: risk_analysis — what could go wrong
- think: crate_ordering — which crates to modify in what order
- merge: combine all analyses
- think: final_plan — structured plan with file paths and test strategy

Usage:
    PYTHONPATH=crates/apxm-frontend/python python3 examples/python/self-hosted/plan_feature.py > /tmp/plan.air
    dekk apxm compile /tmp/plan.air -o /tmp/plan.apxmobj
    dekk apxm execute /tmp/plan.air "Add streaming support to LLM backends"
"""

import os
from apxm.graph import compile, GraphRecorder


@compile()
def plan_feature_workflow(g: GraphRecorder):
    """Generate detailed implementation plan for a feature request.

    Parameters:
        feature (str): Feature description
    """
    g.param("feature", "str")

    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn architect
    architect = g.spawn("architect", profile="claude", cwd=cwd)

    # Step 1: Architect produces initial plan
    architect_task = g.const_(
        "architect_task",
        value="""You are the APXM architect. Create an implementation plan for this feature:

Feature: {0}

Read the APXM project structure:
- CLAUDE.md for architecture overview
- Cargo.toml for workspace crates
- docs/implementation/architecture.md for design principles
- Existing code in relevant crates

Produce a plan with:
1. Feature breakdown (3-5 sub-tasks)
2. Affected crates (which crates need changes)
3. Public API changes (what new functions/types are exposed)
4. Internal implementation changes (what changes inside each crate)
5. Dependencies (what new dependencies might be needed)
6. Testing strategy (unit tests, integration tests, examples)

Be specific — include function names, file paths, and type signatures where possible.
Keep under 500 words.
"""
    )

    architect.ask("{0}")
    architect_task | architect.get_last_node()

    print1 = g.print_("print_initial_plan", message="=== INITIAL PLAN ===\n{0}")
    architect.get_last_node() | print1

    # Step 2: Gap analysis — what's missing vs what exists
    gap_analysis = g.think(
        "gap_analysis",
        template="""Analyze what's missing vs what exists:

Feature: {0}
Initial plan: {1}

Gap analysis:
1. What components already exist that we can reuse?
   - Look for similar functionality in existing crates
   - What traits/types can be extended vs created new

2. What's completely new that needs to be built?
   - New crates needed?
   - New abstractions needed?

3. What's the gap between current state and desired state?
   - What's the minimal set of changes needed?
   - What can be deferred to later phases?

Output a structured gap analysis with:
- Existing components to reuse
- New components to build
- Missing pieces (critical vs nice-to-have)
"""
    )
    g.const_("feature_for_gap", value="{0}") | gap_analysis
    architect.get_last_node() | gap_analysis
    print1 >> gap_analysis

    print2 = g.print_("print_gap_analysis", message="=== GAP ANALYSIS ===\n{0}")
    gap_analysis | print2

    # Step 3: Risk analysis — what could go wrong
    risk_analysis = g.think(
        "risk_analysis",
        template="""Identify risks and mitigation strategies:

Feature: {0}
Initial plan: {1}

Risk analysis:
1. Technical risks:
   - What could break?
   - What's the complexity (low/medium/high)?
   - Where might we hit performance issues?

2. Design risks:
   - What assumptions are we making?
   - What if requirements change?
   - Is the API future-proof?

3. Integration risks:
   - What existing code might this affect?
   - Breaking changes to public APIs?
   - Migration path for existing users?

For each risk, provide:
- Risk level (low/medium/high)
- Impact if it happens
- Mitigation strategy
"""
    )
    g.const_("feature_for_risk", value="{0}") | risk_analysis
    architect.get_last_node() | risk_analysis
    print1 >> risk_analysis

    print3 = g.print_("print_risk_analysis", message="=== RISK ANALYSIS ===\n{0}")
    risk_analysis | print3

    # Step 4: Crate ordering — bottom-up dependency order
    crate_ordering = g.think(
        "crate_ordering",
        template="""Determine the order to modify crates based on dependencies:

Initial plan: {0}

APXM crate dependency order (bottom-up):
1. apxm-core (no dependencies on other APXM crates)
2. apxm-events, apxm-ais, apxm-tools, apxm-sandbox
3. apxm-credentials, apxm-backends
4. apxm-graph, apxm-artifact
5. apxm-compiler, apxm-acp
6. apxm-runtime
7. apxm-driver
8. apxm-cli, apxm-server

Output the implementation order:
- Which crates to modify in which order
- Why this order (dependency reasons)
- Which crates can be done in parallel
- Estimated effort per crate (hours)
"""
    )
    architect.get_last_node() | crate_ordering
    print1 >> crate_ordering

    print4 = g.print_("print_crate_ordering", message="=== CRATE ORDERING ===\n{0}")
    crate_ordering | print4

    # Step 5: Merge all analyses
    merge = g.merge(
        "merge_analyses",
        architect.get_last_node(),
        gap_analysis,
        risk_analysis,
        crate_ordering
    )
    print2 >> merge
    print3 >> merge
    print4 >> merge

    # Step 6: Final plan with everything integrated
    final_plan = g.think(
        "final_plan",
        template="""Create the final implementation plan:

Initial plan: {0}
Gap analysis: {1}
Risk analysis: {2}
Crate ordering: {3}

Final plan structure:

# Feature: <feature name>

## Summary
<1-2 sentence summary>

## Implementation Order
<numbered list of crates in dependency order>

## Detailed Steps
For each crate:
1. File: <path>
   Changes: <what to add/modify/delete>
   Types: <new types/traits>
   Functions: <new functions with signatures>

## Public API Changes
<list of breaking vs non-breaking changes>

## Test Strategy
- Unit tests: <which files>
- Integration tests: <which scenarios>
- Examples: <which examples to add/update>

## Risks & Mitigation
<top 3 risks with mitigation>

## Estimated Effort
- Development: <hours/days>
- Testing: <hours/days>
- Documentation: <hours/days>
- Total: <hours/days>

Keep the plan actionable and specific. Include file paths and function names.
"""
    )
    architect.get_last_node() | final_plan
    gap_analysis | final_plan
    risk_analysis | final_plan
    crate_ordering | final_plan
    merge >> final_plan

    print5 = g.print_("print_final_plan", message="=== FINAL PLAN ===\n{0}")
    final_plan | print5

    g.return_("result", source=print5)


if __name__ == "__main__":
    print(plan_feature_workflow._graph.to_air())
