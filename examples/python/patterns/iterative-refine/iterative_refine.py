#!/usr/bin/env python3
"""iterative_refine.py - 3 rounds of REFLECT -> ASK self-refinement

Usage: python3 -m examples.python.patterns.iterative-refine.iterative_refine
"""

from apxm.graph import compile, GraphRecorder


@compile()
def iterative_refine_unrolled(g: GraphRecorder):
    """Iterative refinement workflow with 3 rounds of critique and improvement."""
    # Initial draft
    initial_draft = g.ask(
        "initial_draft",
        "Write a concise technical explanation of how LLM inference works, "
        "targeting a software engineer audience. Be thorough but under 300 words."
    )

    # Round 1: Reflect and refine
    reflect_1 = g.think(
        "reflect_1",
        "Critique this draft. What is unclear, imprecise, or missing? "
        "Be specific and constructive.\n\nDraft:\n{0}"
    )
    initial_draft | reflect_1

    refine_1 = g.ask(
        "refine_1",
        "Improve this draft based on the critique below.\n\n"
        "Original draft:\n{0}\n\nCritique:\n{1}\n\n"
        "Output only the improved draft."
    )
    initial_draft | refine_1
    reflect_1 | refine_1

    # Round 2: Reflect and refine
    reflect_2 = g.think(
        "reflect_2",
        "Critique this revised draft. What still needs improvement? "
        "Look for technical accuracy and clarity.\n\nDraft:\n{0}"
    )
    refine_1 | reflect_2

    refine_2 = g.ask(
        "refine_2",
        "Improve this draft based on the second round of critique.\n\n"
        "Revised draft:\n{0}\n\nCritique:\n{1}\n\n"
        "Output only the improved draft."
    )
    refine_1 | refine_2
    reflect_2 | refine_2

    # Round 3: Final polish
    reflect_3 = g.think(
        "reflect_3",
        "Final critique before publication. Check for: technical accuracy, "
        "flow, clarity, conciseness.\n\nDraft:\n{0}"
    )
    refine_2 | reflect_3

    refine_3 = g.ask(
        "refine_3",
        "Apply the final polish to this draft.\n\n"
        "Draft:\n{0}\n\nFinal critique:\n{1}\n\n"
        "Output the publication-ready version."
    )
    refine_2 | refine_3
    reflect_3 | refine_3

    # Print final result
    output = g.print_("final_output", message="=== FINAL REFINED DRAFT ===\n{0}")
    refine_3 | output

    g.return_("result", source=output)
    


if __name__ == "__main__":
    import json
    print(iterative_refine_unrolled._graph.to_air())
