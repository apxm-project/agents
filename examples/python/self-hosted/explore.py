#!/usr/bin/env python3
"""explore.py - AI Council / Explore Solution

5-way parallel exploration of a technical question with adversarial synthesis.

Graph structure:
- 5 parallel agents with different perspectives:
  * architect (systems design)
  * adversary (what breaks, what's over-engineered)
  * implementer (concrete Rust implementation)
  * researcher (literature/industry perspective)
  * user_advocate (user needs)
- think: synthesize — merge all perspectives, adversary wins on scope
- think: action_plan — extract concrete next steps

Usage:
    PYTHONPATH=crates/apxm-frontend/python python3 examples/python/self-hosted/explore.py > /tmp/explore.air
    dekk apxm compile /tmp/explore.air -o /tmp/explore.apxmobj
    dekk apxm execute /tmp/explore.air "Should we add distributed execution to APXM?"
"""

import os
from apxm.graph import compile, GraphRecorder


@compile()
def explore_workflow(g: GraphRecorder):
    """AI council: 5-way parallel exploration with adversarial synthesis.

    Parameters:
        question (str): Technical question to explore
    """
    g.param("question", "str")

    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn 5 agents with different personas
    architect = g.spawn("architect", profile="claude", cwd=cwd)
    adversary = g.spawn("adversary", profile="claude", cwd=cwd)
    implementer = g.spawn("implementer", profile="codex", cwd=cwd)
    researcher = g.spawn("researcher", profile="claude", cwd=cwd)
    user_advocate = g.spawn("user_advocate", profile="claude", cwd=cwd)

    # All 5 get the same question, different perspectives
    architect_prompt = g.const_(
        "architect_prompt",
        value="""You are the APXM systems architect. Answer this question from a systems design perspective:

Question: {0}

Consider:
- How does this fit into the overall APXM architecture (compiler, runtime, AIS, frontend)?
- What are the architectural implications?
- How does it interact with existing components (AAM, scheduler, MLIR backend)?
- What's the cleanest way to implement this in the current design?

Be specific and reference actual APXM components. Keep under 300 words.
"""
    )

    adversary_prompt = g.const_(
        "adversary_prompt",
        value="""You are the adversary. Your job is to find problems with the proposed idea:

Question: {0}

Challenge it:
- What breaks if we do this?
- What's over-engineered?
- What assumptions are we making that might be wrong?
- What are the maintenance costs?
- Is this solving a real problem or imaginary one?
- What's the simplest alternative that achieves 80% of the value?

Be brutally honest. If it's a bad idea, say so. Keep under 300 words.
"""
    )

    implementer_prompt = g.const_(
        "implementer_prompt",
        value="""You are the implementer. Answer this question with concrete Rust code:

Question: {0}

Show:
- What crates would be modified
- What new types/traits would be needed
- Sketch key function signatures
- Show a minimal working example (pseudocode is fine)

Focus on *how* it would actually be built in Rust. Keep under 300 words.
"""
    )

    researcher_prompt = g.const_(
        "researcher_prompt",
        value="""You are the researcher. Answer this question based on what the industry and literature say:

Question: {0}

Research:
- How do similar systems solve this? (LLVM, Dask, Ray, etc.)
- What do the papers say? (cite if you know specific ones)
- What are the standard approaches?
- What have others tried that failed?
- What's the state of the art?

Cite examples from real systems. Keep under 300 words.
"""
    )

    user_advocate_prompt = g.const_(
        "user_advocate_prompt",
        value="""You are the user advocate. Answer this question from the user's perspective:

Question: {0}

Consider:
- What do APXM users actually need?
- Is this solving their pain points?
- How does this affect the user experience (CLI, Python API, workflows)?
- What would users have to learn or change?
- Is there a simpler way to give users what they want?

Think about real-world workflow authors using APXM. Keep under 300 words.
"""
    )

    # All 5 agents work in parallel
    architect.ask("{0}")
    architect_prompt | architect.get_last_node()

    adversary.ask("{0}")
    adversary_prompt | adversary.get_last_node()

    implementer.ask("{0}")
    implementer_prompt | implementer.get_last_node()

    researcher.ask("{0}")
    researcher_prompt | researcher.get_last_node()

    user_advocate.ask("{0}")
    user_advocate_prompt | user_advocate.get_last_node()

    # Print each perspective
    print_arch = g.print_("print_architect", message="=== ARCHITECT ===\n{0}")
    architect.get_last_node() | print_arch

    print_adv = g.print_("print_adversary", message="=== ADVERSARY ===\n{0}")
    adversary.get_last_node() | print_adv

    print_impl = g.print_("print_implementer", message="=== IMPLEMENTER ===\n{0}")
    implementer.get_last_node() | print_impl

    print_res = g.print_("print_researcher", message="=== RESEARCHER ===\n{0}")
    researcher.get_last_node() | print_res

    print_user = g.print_("print_user_advocate", message="=== USER ADVOCATE ===\n{0}")
    user_advocate.get_last_node() | print_user

    # Wait for all to complete
    wait = g.wait_all(
        "wait_all_perspectives",
        architect.get_last_node(),
        adversary.get_last_node(),
        implementer.get_last_node(),
        researcher.get_last_node(),
        user_advocate.get_last_node()
    )
    print_arch >> wait
    print_adv >> wait
    print_impl >> wait
    print_res >> wait
    print_user >> wait

    # Synthesis: merge all 5 perspectives, adversary wins on scope
    synthesis = g.think(
        "synthesis",
        template="""Synthesize the 5 perspectives on this question:

Question: {0}

Architect: {1}
Adversary: {2}
Implementer: {3}
Researcher: {4}
User advocate: {5}

Synthesize:
1. What do all perspectives agree on?
2. Where do they conflict?
3. What did the adversary catch that others missed?
4. What's the balanced view?
5. Recommendation: should we do this? (yes/no/maybe/different-approach)

Let the adversary win on scope reduction — if they identified over-engineering, take it seriously.
Give a clear, definitive answer. Keep under 400 words.
"""
    )
    # Wire all 6 inputs (question + 5 perspectives)
    g.const_("question_for_synthesis", value="{0}") | synthesis
    architect.get_last_node() | synthesis
    adversary.get_last_node() | synthesis
    implementer.get_last_node() | synthesis
    researcher.get_last_node() | synthesis
    user_advocate.get_last_node() | synthesis
    wait >> synthesis

    print_synth = g.print_("print_synthesis", message="=== SYNTHESIS ===\n{0}")
    synthesis | print_synth

    # Action plan: extract concrete next steps
    action_plan = g.think(
        "action_plan",
        template="""Based on the synthesis, extract concrete next steps:

Synthesis: {0}

If the recommendation is YES or MAYBE:
- List 3-5 concrete action items with file paths
- Assign priority (P0/P1/P2)
- Estimate effort (small/medium/large)
- Identify risks

If the recommendation is NO or DIFFERENT-APPROACH:
- Explain why not
- Suggest the alternative
- List what to do instead (if anything)

Output a structured action plan ready for execution. Keep under 300 words.
"""
    )
    synthesis | action_plan
    print_synth >> action_plan

    print_plan = g.print_("print_action_plan", message="=== ACTION PLAN ===\n{0}")
    action_plan | print_plan

    g.return_("result", source=print_plan)


if __name__ == "__main__":
    print(explore_workflow._graph.to_air())
