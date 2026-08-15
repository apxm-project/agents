"""The Capability this package supplies, declared and implemented in one place.

Its directory name is the capability id, exactly as `capabilities/<id>/handler.ts`
is for the other frontend, so shipping this file is the whole declaration that
the package can supply `normalize`.
"""

from __future__ import annotations

from typing import Any, Mapping

from apxm_program.handlers import answer, capability, schema, text


def run_normalize(args: Mapping[str, Any]) -> dict[str, Any]:
    """Collapse a label's whitespace and case, without touching anything else."""
    label = " ".join(str(args["label"]).split())
    if not label:
        raise ValueError("normalize requires a non-empty label")
    return answer({"label": label.lower(), "words": len(label.split()), "mutates": False})


normalize = capability(
    {
        "name": "normalize",
        "description": "Return a whitespace- and case-normalized label.",
        # The handler states this about itself; nothing else in the package does.
        "read_only": True,
        "input": schema(
            additional_properties=False,
            properties={"label": text(required=True, min_length=1)},
        ),
        "run": run_normalize,
    }
)
