"""In-process compiler bridge.

Submits an emitted FrontendGraph to the native extension for verification,
canonical AIR lowering, and artifact compilation. These are compiler results,
not authoring APIs: authors write declarations and control flow, never a graph
dict.
"""

from __future__ import annotations

import json
from typing import Any, Optional

from ._native import compile_frontend_graph_artifact as _compile_frontend_graph_artifact
from ._native import lower_frontend_graph as _lower_frontend_graph
from ._native import verify_frontend_graph as _verify_frontend_graph

FRONTEND_GRAPH_VERSION = "apxm.frontend-graph.v1"
SOURCE_MAP_VERSION = "apxm.source-map.v1"


def verify_graph(graph: dict[str, Any]) -> Optional[str]:
    """Return ``None`` when the graph verifies, else a diagnostic string."""
    return _verify_frontend_graph(json.dumps(graph))


def lower_graph(graph: dict[str, Any]) -> dict[str, Any]:
    """Lower a FrontendGraph to canonical AIR through the native bridge."""
    return json.loads(_lower_frontend_graph(json.dumps(graph)))


def canonical_air_json(graph: dict[str, Any]) -> str:
    """The canonical AIR JSON string exactly as emitted by the native bridge."""
    return _lower_frontend_graph(json.dumps(graph))


def compile_artifact(graph: dict[str, Any]) -> dict[str, Any]:
    """Compile a FrontendGraph into one complete executable artifact."""
    return json.loads(_compile_frontend_graph_artifact(json.dumps(graph)))
