"""A stand-in authoring frontend that records the wrong source language.

The port declares where each authoring frontend package lives, so what stands at
a declared root is not something the port can assume. This package occupies the
Python root and hands back a structurally valid `apxm.frontend-graph.v1` that
claims TypeScript authored it, which is exactly the graph a caller must never
receive back as the compilation of Python source.

It is a test double for the source-port conformance tests. It is not an
authoring frontend: it captures nothing, reads no source, and is reachable only
from the fixture root under this crate's tests.
"""

from __future__ import annotations

from typing import Any

#: The source language this stand-in records, which is not the language of the
#: source the port submits to it.
MISLABELED_SOURCE_LANGUAGE = "typescript"


class _MislabeledProgram:
    """Hands back a well-formed graph attributed to the wrong language."""

    def frontend_graph(self) -> dict[str, Any]:
        return {
            "schema_version": "apxm.frontend-graph.v1",
            "source_language": MISLABELED_SOURCE_LANGUAGE,
            "program_definitions": [],
            "imported_program_refs": [],
            "declarations": [],
            "functions": [],
            "values": [],
            "blocks": [],
            "regions": [],
            "data_edges": [],
            "call_intents": [],
            "control_intents": [],
            "context_flow": [],
            "hook_bindings": [],
            "capability_requirements": [],
            "model_requirements": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": MISLABELED_SOURCE_LANGUAGE,
                "node_spans": [],
                "region_annotations": [],
            },
        }


def mislabeled_program() -> _MislabeledProgram:
    """The stand-in program the fixture source binds as its entrypoint."""
    return _MislabeledProgram()
