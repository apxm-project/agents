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
    dekk agents execute examples/python/self-hosted/plan_feature.py \
      "Add streaming support to LLM backends"
"""

from apxm import DependencyType, GraphRecorder, agent_cwd, compile
from apxm._generated.agents import claude, codex


@compile()
def plan_feature_workflow(g: GraphRecorder):
    """Generate detailed implementation plan for a feature request.

    Parameters:
        feature (str): Feature description
    """
    g.param("feature", "str")

    cwd = agent_cwd()

    # Spawn architect
    architect = g.spawn("architect", profile=claude, cwd=cwd)

    # Step 1: Architect produces initial plan
    initial_plan = architect.ask(prompt="""You are the APXM architect. Create an implementation plan for this feature:

Feature: {feature}

Read the APXM project structure:
- CLAUDE.md for architecture overview
- Cargo.toml for workspace crates
- docs/README.md and docs/compiler/pipeline.md for current design notes
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
""")

    print1 = g.print(message="=== INITIAL PLAN ===\n{initial_plan}")

    # Step 2: Gap analysis — what's missing vs what exists
    gap_analysis = g.think(
        name="gap_analysis",
        prompt="""Analyze what's missing vs what exists:

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
    g.add_edge(print1, gap_analysis, dependency=DependencyType.CONTROL)

    print2 = g.print(message="=== GAP ANALYSIS ===\n{gap_analysis}")

    # Step 3: Risk analysis — what could go wrong
    risk_analysis = g.think(
        name="risk_analysis",
        prompt="""Identify risks and mitigation strategies:

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
    g.add_edge(print1, risk_analysis, dependency=DependencyType.CONTROL)

    print3 = g.print(message="=== RISK ANALYSIS ===\n{risk_analysis}")

    # Step 4: Crate ordering — bottom-up dependency order
    crate_ordering = g.think(
        name="crate_ordering",
        prompt="""Determine the order to modify crates based on dependencies:

Initial plan: {initial_plan}

APXM crate dependency order (bottom-up):
1. apxm-core and apxm-program own contracts and canonical FrontendGraph/AIR types
2. apxm-inference and apxm-kernel define replaceable runtime ports
3. apxm-execution executes canonical AIR through injected ports
4. apxm-composition wires exact admitted adapters at the composition root
5. apxm-cli and service clients call the canonical compiler/execution surfaces

Output the implementation order:
- Which crates to modify in which order
- Why this order (dependency reasons)
- Which crates can be done in parallel
- Estimated effort per crate (hours)
"""
    )
    g.add_edge(print1, crate_ordering, dependency=DependencyType.CONTROL)

    print4 = g.print(message="=== CRATE ORDERING ===\n{crate_ordering}")

    # Step 5: Merge all analyses
    merge = g.merge(
        "merge_analyses",
        initial_plan,
        gap_analysis,
        risk_analysis,
        crate_ordering
    )
    g.add_edge(print2, merge, dependency=DependencyType.CONTROL)
    g.add_edge(print3, merge, dependency=DependencyType.CONTROL)
    g.add_edge(print4, merge, dependency=DependencyType.CONTROL)

    # Step 6: Final plan with everything integrated
    final_plan = g.think(
        name="final_plan",
        prompt="""Create the final implementation plan:

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
    g.add_edge(merge, final_plan, dependency=DependencyType.CONTROL)

    print5 = g.print(message="=== FINAL PLAN ===\n{final_plan}")

    g.done(print5)


if __name__ == "__main__":
    import apxm

    result = apxm.run(plan_feature_workflow("Add retry logic to LLM calls"))
    print(result.content)
