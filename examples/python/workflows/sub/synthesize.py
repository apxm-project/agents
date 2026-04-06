#!/usr/bin/env python3
"""synthesize.py - Synthesis workflow combining multiple perspectives

Usage: python3 -m examples.python.workflows.sub.synthesize
"""

from apxm.graph import compile, GraphRecorder


@compile()
def synthesize(g: GraphRecorder, arch: str, adver: str, impl: str):
    """Synthesize architecture, adversary, and implementation perspectives.

    Parameters
    ----------
    arch : str
        Architecture perspective.
    adver : str
        Adversary perspective (risks and concerns).
    impl : str
        Implementation perspective.
    """
    synthesize_think = g.think(
        "synthesize_think",
        "You are a synthesis expert. Given these three perspectives:\n\n"
        "ARCHITECTURE:\n{arch}\n\n"
        "RISKS & CONCERNS:\n{adver}\n\n"
        "IMPLEMENTATION:\n{impl}\n\n"
        "Synthesize into a comprehensive plan:\n"
        "1. Best architectural ideas\n"
        "2. How risks are mitigated\n"
        "3. Clear implementation guidance\n\n"
        "Provide a unified, actionable plan."
    )

    g.return_("result", source=synthesize_think)
    


if __name__ == "__main__":
    import json
    print(json.dumps(synthesize._graph.to_dict(), indent=2))
