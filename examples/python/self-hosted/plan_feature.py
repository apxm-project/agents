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
from apxm._generated.agents import claude, codex


@compile()
def plan_feature_workflow(g: GraphRecorder):
    """Generate detailed implementation plan for a feature request.

    Parameters:
        feature (str): Feature description
    """
    g.param("feature", "str")

    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn architect
    architect = g.spawn("architect", profile=claude, cwd=cwd)

    # Step 1: Architect produces initial plan
    architect_task = g.text(value="""You are the APXM architect. Create an implementation plan for this feature:

Feature: {feature}

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

    architect.ask("{architect_task}")
    initial_plan = architect.get_last_node()

    print1 = g.print("=== INITIAL PLAN ===\n{initial_plan}")

    # Step 2: Gap analysis — what's missing vs what exists
    gap_analysis = g.think(
        "gap_analysis",
        """Analyze what's missing vs what exists:

Feature: {feature}
Initial plan: {initial_plan}

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
    print1 >> gap_analysis

    print2 = g.print("=== GAP ANALYSIS ===\n{gap_analysis}")

    # Step 3: Risk analysis — what could go wrong
    risk_analysis = g.think(
        "risk_analysis",
        """Identify risks and mitigation strategies:

Feature: {feature}
Initial plan: {initial_plan}

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
    print1 >> risk_analysis

    print3 = g.print("=== RISK ANALYSIS ===\n{risk_analysis}")

    # Step 4: Crate ordering — bottom-up dependency order
    crate_ordering = g.think(
        "crate_ordering",
        """Determine the order to modify crates based on dependencies:

Initial plan: {initial_plan}

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
    print1 >> crate_ordering

    print4 = g.print("=== CRATE ORDERING ===\n{crate_ordering}")

    # Step 5: Merge all analyses
    merge = g.merge(
        "merge_analyses",
        initial_plan,
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
        """Create the final implementation plan:

Initial plan: {initial_plan}
Gap analysis: {gap_analysis}
Risk analysis: {risk_analysis}
Crate ordering: {crate_ordering}

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
    merge >> final_plan

    print5 = g.print("=== FINAL PLAN ===\n{final_plan}")

    g.done(print5)


if __name__ == "__main__":
    print(plan_feature_workflow._graph.to_air())
