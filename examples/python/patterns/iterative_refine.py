#!/usr/bin/env python3
"""iterative_refine.py - 3 rounds of REFLECT -> ASK self-refinement

Usage: python3 -m examples.python.patterns.iterative-refine.iterative_refine
"""

from apxm import compile, GraphRecorder


@compile()
def iterative_refine_unrolled(g: GraphRecorder):
    """Iterative refinement workflow with 3 rounds of critique and improvement."""
    # Initial draft (auto-named "ask")
    initial_draft = g.ask("Write a concise technical explanation of how LLM inference works, targeting a software engineer audience. Be thorough but under 300 words.")

    # Round 1: Reflect and refine (auto-named "think", "ask_1")
    reflect_1 = g.think(
        "Critique this draft. What is unclear, imprecise, or missing? "
        "Be specific and constructive.\n\nDraft:\n{initial_draft}"
    )

    refine_1 = g.ask(
        "Improve this draft based on the critique below.\n\n"
        "Original draft:\n{initial_draft}\n\nCritique:\n{reflect_1}\n\n"
        "Output only the improved draft."
    )

    # Round 2: Reflect and refine (auto-named "think_1", "ask_2")
    reflect_2 = g.think(
        "Critique this revised draft. What still needs improvement? "
        "Look for technical accuracy and clarity.\n\nDraft:\n{refine_1}"
    )

    refine_2 = g.ask(
        "Improve this draft based on the second round of critique.\n\n"
        "Revised draft:\n{refine_1}\n\nCritique:\n{reflect_2}\n\n"
        "Output only the improved draft."
    )

    # Round 3: Final polish (auto-named "think_2", "ask_3")
    reflect_3 = g.think(
        "Final critique before publication. Check for: technical accuracy, "
        "flow, clarity, conciseness.\n\nDraft:\n{refine_2}"
    )

    refine_3 = g.ask(
        "Apply the final polish to this draft.\n\n"
        "Draft:\n{refine_2}\n\nFinal critique:\n{reflect_3}\n\n"
        "Output the publication-ready version."
    )

    # Print final result (auto-named "print")
    output = g.print("=== FINAL REFINED DRAFT ===\n{refine_3}")

    g.done(output)
    


if __name__ == "__main__":
    import json
    print(iterative_refine_unrolled._graph.to_air())
