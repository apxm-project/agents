"""An Agent Program that invokes the Capability this package ships.

Every argument is a literal, so executing it exercises exactly one thing:
whether a `capability.invoke` for a Capability this package supplies in Python
reaches the Python handler this package ships.

The handler is loaded by path rather than imported. The package folder contract
has no Python package structure — `capabilities/<id>/handler.py` is a recognized
path, not a module in an importable tree — so this is how a program names the
declaration that implements the capability it references.
"""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from typing import TypedDict

from apxm_program import Agent, Capability

# Loading by path must not leave build litter in the package directory.
sys.dont_write_bytecode = True

_HANDLER = Path(__file__).resolve().parent.parent / "capabilities/normalize/handler.py"
_SPEC = importlib.util.spec_from_file_location("capabilities/normalize/handler", _HANDLER)
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)


class NormalizeRequest(TypedDict):
    label: str


class NormalizeResult(TypedDict):
    label: str
    words: int
    mutates: bool


class NoInput(TypedDict):
    pass


Normalize = Capability[NormalizeRequest, NormalizeResult](_MODULE.normalize)


@Agent(input=NoInput, output=NormalizeResult)
async def PythonHandlerFixture(agent, request):
    """Invoke the shipped Capability with an authored literal."""
    return await Normalize({"label": "  Shipped   BY   Python  "})


def main() -> None:
    """Print the fixture's canonical AIR, artifact, or diagnostics."""
    if "--air" in sys.argv[1:]:
        print(PythonHandlerFixture.canonical_air())
    elif "--diagnostics" in sys.argv[1:]:
        print(json.dumps(PythonHandlerFixture.diagnostics()))
    else:
        print(
            json.dumps(
                PythonHandlerFixture.frontend_graph(),
                sort_keys=True,
                separators=(",", ":"),
            )
        )


if __name__ == "__main__":
    main()
