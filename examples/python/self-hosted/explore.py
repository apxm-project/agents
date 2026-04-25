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

    # All 5 agents work in parallel with different perspectives
    arch_result = architect.ask(prompt="""You are the APXM systems architect. Answer this question from a systems design perspective:

Question: {question}

Consider:
- How does this fit into the overall APXM architecture (compiler, runtime, AIS, frontend)?
- What are the architectural implications?
- How does it interact with existing components (AAM, scheduler, MLIR backend)?
- What's the cleanest way to implement this in the current design?

Be specific and reference actual APXM components. Keep under 300 words.
""")

    adv_result = adversary.ask(prompt="""You are the adversary. Your job is to find problems with the proposed idea:

Question: {question}

Challenge it:
- What breaks if we do this?
- What's over-engineered?
- What assumptions are we making that might be wrong?
- What are the maintenance costs?
- Is this solving a real problem or imaginary one?
- What's the simplest alternative that achieves 80% of the value?

Be brutally honest. If it's a bad idea, say so. Keep under 300 words.
""")

    impl_result = implementer.ask(prompt="""You are the implementer. Answer this question with concrete Rust code:

Question: {question}

Show:
- What crates would be modified
- What new types/traits would be needed
- Sketch key function signatures
- Show a minimal working example (pseudocode is fine)

Focus on *how* it would actually be built in Rust. Keep under 300 words.
""")

    res_result = researcher.ask(prompt="""You are the researcher. Answer this question based on what the industry and literature say:

Question: {question}

Research:
- How do similar systems solve this? (LLVM, Dask, Ray, etc.)
- What do the papers say? (cite if you know specific ones)
- What are the standard approaches?
- What have others tried that failed?
- What's the state of the art?

Cite examples from real systems. Keep under 300 words.
""")

    user_result = user_advocate.ask(prompt="""You are the user advocate. Answer this question from the user's perspective:

Question: {question}

Consider:
- What do APXM users actually need?
- Is this solving their pain points?
- How does this affect the user experience (CLI, Python API, workflows)?
- What would users have to learn or change?
- Is there a simpler way to give users what they want?

Think about real-world workflow authors using APXM. Keep under 300 words.
""")

    # Print each perspective
    print_arch = g.print(message="=== ARCHITECT ===\n{arch_result}")
    print_adv = g.print(message="=== ADVERSARY ===\n{adv_result}")
    print_impl = g.print(message="=== IMPLEMENTER ===\n{impl_result}")
    print_res = g.print(message="=== RESEARCHER ===\n{res_result}")
    print_user = g.print(message="=== USER ADVOCATE ===\n{user_result}")

    # Wait for all to complete
    wait = g.wait_all(
        "wait_all_perspectives",
        arch_result,
        adv_result,
        impl_result,
        res_result,
        user_result
    )
    g.add_edge(print_arch, wait, dependency="Control")
    g.add_edge(print_adv, wait, dependency="Control")
    g.add_edge(print_impl, wait, dependency="Control")
    g.add_edge(print_res, wait, dependency="Control")
    g.add_edge(print_user, wait, dependency="Control")

    # Synthesis: merge all 5 perspectives, adversary wins on scope
    synthesis = g.think(
        name="synthesis",
        prompt="""Synthesize the 5 perspectives on this question:

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
    g.add_edge(wait, synthesis, dependency="Control")

    print_synth = g.print(message="=== SYNTHESIS ===\n{synthesis}")

    # Action plan: extract concrete next steps
    action_plan = g.think(
        name="action_plan",
        prompt="""Based on the synthesis, extract concrete next steps:

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
    g.add_edge(print_synth, action_plan, dependency="Control")

    print_plan = g.print(message="=== ACTION PLAN ===\n{action_plan}")

    g.done(print_plan)


if __name__ == "__main__":
    import apxm

    result = apxm.run(explore_workflow("How does the scheduler work?"))
    print(result.content)
