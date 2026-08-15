"""A reference Agent Program that declares Agent Skills and loads them.

The two `Skill` declarations differ only in where their instructions live, and
that difference is the route an edit takes to the artifact digest: the file one
is hashed into this package's integrity chain, and the written one is inside the
source bundle the artifact digest already covers. Loading either is the same
ordinary `capability.invoke`, so the artifact declares the authority to read
instructions the way it declares every other capability.
"""

from __future__ import annotations

import json
import sys
from typing import TypedDict

from apxm_program import Agent, Skill


class ReviewRequest(TypedDict):
    change: str


class ReviewGuidance(TypedDict):
    instructions: str


# Carried as a package file the folder contract recognizes and the integrity
# chain hashes.
ReviewSkill = Skill("review", entry="skills/review/SKILL.md")

# Written here, so editing it changes the artifact digest through the source
# bundle rather than through a package file digest.
ToneSkill = Skill(
    "tone",
    text=(
        "# Tone\n\n"
        "Answer in the register the author wrote in. Prefer one concrete\n"
        "sentence to three hedged ones.\n"
    ),
)


@Agent(input=ReviewRequest, output=ReviewGuidance)
async def SkilledExample(agent, request):
    """Load both declared skills and return the instructions they carry."""
    await ToneSkill.load()
    instructions = await ReviewSkill.load()
    return {"instructions": instructions}


def main() -> None:
    """Print the example graph, canonical AIR, artifact, or diagnostics."""
    if "--air" in sys.argv[1:]:
        print(SkilledExample.canonical_air())
    elif "--artifact" in sys.argv[1:]:
        print(json.dumps(SkilledExample.artifact(), sort_keys=True, separators=(",", ":")))
    elif "--diagnostics" in sys.argv[1:]:
        print(json.dumps(SkilledExample.diagnostics()))
    else:
        print(
            json.dumps(
                SkilledExample.frontend_graph(),
                sort_keys=True,
                separators=(",", ":"),
            )
        )


if __name__ == "__main__":
    main()
