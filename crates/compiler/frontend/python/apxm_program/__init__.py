"""Canonical Agent Program authoring frontend (Python).

Author a program with the five semantic operations and structural regions, then
lower the recorded FrontendGraph to canonical AIR through the in-process native
bridge. No CLI subprocess and no network compile are involved.
"""

from __future__ import annotations

import json
from typing import Any, Optional

from ._native import lower_frontend_graph as _lower_frontend_graph
from ._native import verify_frontend_graph as _verify_frontend_graph

FRONTEND_GRAPH_VERSION = "apxm.frontend-graph.v1"
SOURCE_MAP_VERSION = "apxm.source-map.v1"


class GraphBuilder:
    """Record a language-neutral FrontendGraph."""

    def __init__(self, source_language: str = "python") -> None:
        self._graph: dict[str, Any] = {
            "schema_version": FRONTEND_GRAPH_VERSION,
            "source_language": source_language,
            "program_definitions": [],
            "imported_program_refs": [],
            "semantic_operations": [],
            "structural_regions": [],
            "context_flow": [],
            "hook_bindings": [],
            "capability_requirements": [],
            "model_requirements": [],
            "source_map": {
                "schema_version": SOURCE_MAP_VERSION,
                "source_language": source_language,
                "node_spans": [],
                "region_annotations": [],
            },
        }

    def program(
        self,
        program_id: str,
        entrypoint: str,
        input_type_ref: str,
        output_type_ref: str,
        has_default_context: bool,
        context_type_ref: Optional[str] = None,
    ) -> "GraphBuilder":
        definition: dict[str, Any] = {
            "program_id": program_id,
            "entrypoint": entrypoint,
            "input_type_ref": input_type_ref,
            "output_type_ref": output_type_ref,
            "has_default_context": has_default_context,
        }
        if context_type_ref is not None:
            definition["context_type_ref"] = context_type_ref
        self._graph["program_definitions"].append(definition)
        return self

    def import_program(
        self,
        program_ref: str,
        artifact_digest: str,
        entrypoint: str,
        target_agent_identity_requirement: str,
    ) -> "GraphBuilder":
        self._graph["imported_program_refs"].append(
            {
                "program_ref": program_ref,
                "artifact_digest": artifact_digest,
                "entrypoint": entrypoint,
                "target_agent_identity_requirement": target_agent_identity_requirement,
            }
        )
        return self

    def _op(self, node_id: str, op: str, operands: Optional[dict[str, Any]]) -> "GraphBuilder":
        record: dict[str, Any] = {"node_id": node_id, "op": op}
        if operands is not None:
            record["operands"] = operands
        self._graph["semantic_operations"].append(record)
        return self

    def model_call(self, node_id: str, model_target_ref: Optional[str] = None) -> "GraphBuilder":
        operands = {"model_target_ref": model_target_ref} if model_target_ref is not None else None
        return self._op(node_id, "model.call", operands)

    def capability_invoke(self, node_id: str, capability_ref: Optional[str] = None) -> "GraphBuilder":
        operands = {"capability_ref": capability_ref} if capability_ref is not None else None
        return self._op(node_id, "capability.invoke", operands)

    def external_agent_capability(
        self,
        node_id: str,
        profile_ref: str,
        session_ref: str,
    ) -> "GraphBuilder":
        """Author an External Agent (ACP) capability.

        It lowers to exactly one `capability.invoke` semantic operation — no new
        AIR op and no native `model.call`. The peer's private model/tool cycles
        are recorded as nested attributed evidence under this one NodeExecution.
        The opaque session reference is the only handle retained in Context.
        """
        return self._op(
            node_id,
            "capability.invoke",
            {"capability_ref": f"external-agent:{profile_ref}", "external_agent_session": session_ref},
        )

    def program_new(self, node_id: str) -> "GraphBuilder":
        return self._op(node_id, "program.new", None)

    def program_invoke(self, node_id: str) -> "GraphBuilder":
        return self._op(node_id, "program.invoke", None)

    def await_event(self, node_id: str) -> "GraphBuilder":
        return self._op(node_id, "await.event", None)

    def region(self, region_id: str, kind: str) -> "GraphBuilder":
        self._graph["structural_regions"].append({"region_id": region_id, "kind": kind})
        return self

    def context_edge(self, from_node: str, to_node: str, context_type_ref: str) -> "GraphBuilder":
        self._graph["context_flow"].append(
            {"from_node": from_node, "to_node": to_node, "context_type_ref": context_type_ref}
        )
        return self

    def hook(self, **fields: Any) -> "GraphBuilder":
        self._graph["hook_bindings"].append(dict(fields))
        return self

    def capability_requirement(self, capability_ref: str) -> "GraphBuilder":
        self._graph["capability_requirements"].append({"capability_ref": capability_ref})
        return self

    def model_requirement(self, model_target_ref: str) -> "GraphBuilder":
        self._graph["model_requirements"].append({"model_target_ref": model_target_ref})
        return self

    def build(self) -> dict[str, Any]:
        return json.loads(json.dumps(self._graph))

    def to_canonical_json(self) -> str:
        return json.dumps(self._graph, sort_keys=True, separators=(",", ":"))


def verify(graph: dict[str, Any]) -> Optional[str]:
    """Return None if the graph verifies, else a diagnostic string."""
    return _verify_frontend_graph(json.dumps(graph))


def lower(graph: dict[str, Any]) -> dict[str, Any]:
    """Lower a recorded FrontendGraph to canonical AIR through the native bridge."""
    return json.loads(_lower_frontend_graph(json.dumps(graph)))


def canonical_air_json(graph: dict[str, Any]) -> str:
    """The canonical AIR JSON string exactly as emitted by the native bridge."""
    return _lower_frontend_graph(json.dumps(graph))
