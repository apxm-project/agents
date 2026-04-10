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
    PYTHONPATH=crates/compiler/apxm-frontend/python python3 examples/python/self-hosted/explore.py > /tmp/explore.air
    dekk apxm compile /tmp/explore.air -o /tmp/explore.apxmobj
    dekk apxm execute /tmp/explore.air "Should we add distributed execution to APXM?"
"""

import os
from apxm import compile, GraphRecorder
from apxm._generated.agents import claude, codex


@compile()
def explore_workflow(g: GraphRecorder):
    """AI council: 5-way parallel exploration with adversarial synthesis.

    Parameters:
        question (str): Technical question to explore
    """
    g.param("question", "str")

    cwd = os.environ.get("APXM_HOME", os.getcwd())

    # Spawn 5 agents with different personas
    architect = g.spawn("architect", profile=claude, cwd=cwd)
    adversary = g.spawn("adversary", profile=claude, cwd=cwd)
    implementer = g.spawn("implementer", profile=codex, cwd=cwd)
    researcher = g.spawn("researcher", profile=claude, cwd=cwd)
    user_advocate = g.spawn("user_advocate", profile=claude, cwd=cwd)

    # All 5 get the same question, different perspectives
    architect_prompt = g.text(value="""You are the APXM systems architect. Answer this question from a systems design perspective:

Question: {question}

Consider:
- How does this fit into the overall APXM architecture (compiler, runtime, AIS, frontend)?
- What are the architectural implications?
- How does it interact with existing components (AAM, scheduler, MLIR backend)?
- What's the cleanest way to implement this in the current design?

Be specific and reference actual APXM components. Keep under 300 words.
"""
    )

    adversary_prompt = g.text(value="""You are the adversary. Your job is to find problems with the proposed idea:

Question: {question}

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

    implementer_prompt = g.text(value="""You are the implementer. Answer this question with concrete Rust code:

Question: {question}

Show:
- What crates would be modified
- What new types/traits would be needed
- Sketch key function signatures
- Show a minimal working example (pseudocode is fine)

Focus on *how* it would actually be built in Rust. Keep under 300 words.
"""
    )

    researcher_prompt = g.text(value="""You are the researcher. Answer this question based on what the industry and literature say:

Question: {question}

Research:
- How do similar systems solve this? (LLVM, Dask, Ray, etc.)
- What do the papers say? (cite if you know specific ones)
- What are the standard approaches?
- What have others tried that failed?
- What's the state of the art?

Cite examples from real systems. Keep under 300 words.
"""
    )

    user_advocate_prompt = g.text(value="""You are the user advocate. Answer this question from the user's perspective:

Question: {question}

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
    architect.ask("{architect_prompt}")
    arch_result = architect.get_last_node()

    adversary.ask("{adversary_prompt}")
    adv_result = adversary.get_last_node()

    implementer.ask("{implementer_prompt}")
    impl_result = implementer.get_last_node()

    researcher.ask("{researcher_prompt}")
    res_result = researcher.get_last_node()

    user_advocate.ask("{user_advocate_prompt}")
    user_result = user_advocate.get_last_node()

    # Print each perspective
    print_arch = g.print("=== ARCHITECT ===\n{arch_result}")
    print_adv = g.print("=== ADVERSARY ===\n{adv_result}")
    print_impl = g.print("=== IMPLEMENTER ===\n{impl_result}")
    print_res = g.print("=== RESEARCHER ===\n{res_result}")
    print_user = g.print("=== USER ADVOCATE ===\n{user_result}")

    # Wait for all to complete
    wait = g.wait_all(
        "wait_all_perspectives",
        arch_result,
        adv_result,
        impl_result,
        res_result,
        user_result
    )
    print_arch >> wait
    print_adv >> wait
    print_impl >> wait
    print_res >> wait
    print_user >> wait

    # Synthesis: merge all 5 perspectives, adversary wins on scope
    synthesis = g.think(
        "synthesis",
        """Synthesize the 5 perspectives on this question:

Question: {question}

Architect: {arch_result}
Adversary: {adv_result}
Implementer: {impl_result}
Researcher: {res_result}
User advocate: {user_result}

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
    wait >> synthesis

    print_synth = g.print("=== SYNTHESIS ===\n{synthesis}")

    # Action plan: extract concrete next steps
    action_plan = g.think(
        "action_plan",
        """Based on the synthesis, extract concrete next steps:

Synthesis: {synthesis}

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
    print_synth >> action_plan

    print_plan = g.print("=== ACTION PLAN ===\n{action_plan}")

    g.done(print_plan)


if __name__ == "__main__":
    print(explore_workflow._graph.to_air())
