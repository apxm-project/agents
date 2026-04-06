from __future__ import annotations

from dataclasses import dataclass, field
import json
from typing import Any

from apxm._generated import constants as c
from apxm._generated import operations


_DEPENDENCY_TYPES = {"Data", "Effect", "Control"}


@dataclass(slots=True)
class GraphNode:
    id: int
    name: str
    op: str
    attributes: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "name": self.name,
            "op": self.op,
            "attributes": dict(self.attributes),
        }

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "GraphNode":
        return cls(
            id=int(value["id"]),
            name=str(value["name"]),
            op=str(value["op"]),
            attributes=dict(value.get("attributes", {})),
        )


@dataclass(slots=True)
class GraphEdge:
    from_id: int
    to_id: int
    dependency: str = "Data"

    def __post_init__(self) -> None:
        if self.dependency not in _DEPENDENCY_TYPES:
            allowed = ", ".join(sorted(_DEPENDENCY_TYPES))
            raise ValueError(f"invalid dependency '{self.dependency}', expected one of: {allowed}")

    def to_dict(self) -> dict[str, Any]:
        return {
            "from": self.from_id,
            "to": self.to_id,
            "dependency": self.dependency,
        }

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "GraphEdge":
        return cls(
            from_id=int(value.get("from", value.get("from_id"))),
            to_id=int(value.get("to", value.get("to_id"))),
            dependency=str(value.get("dependency", "Data")),
        )


@dataclass(slots=True)
class Parameter:
    name: str
    type_name: str

    def to_dict(self) -> dict[str, Any]:
        return {"name": self.name, "type_name": self.type_name}

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "Parameter":
        return cls(name=str(value["name"]), type_name=str(value["type_name"]))


@dataclass(slots=True)
class ApxmGraph:
    name: str
    nodes: list[GraphNode] = field(default_factory=list)
    edges: list[GraphEdge] = field(default_factory=list)
    parameters: list[Parameter] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "nodes": [node.to_dict() for node in self.nodes],
            "edges": [edge.to_dict() for edge in self.edges],
            "parameters": [parameter.to_dict() for parameter in self.parameters],
            "metadata": dict(self.metadata),
        }

    def to_json(self, *, indent: int = 2, sort_keys: bool = False) -> str:
        return json.dumps(self.to_dict(), indent=indent, sort_keys=sort_keys)

    def to_air(self) -> str:
        """Emit canonical .air text IR for this graph.

        The .air format is the Agent Intermediate Representation:
        human-readable, SSA-style, diffable — analogous to LLVM's .ll format.

        All supported authoring frontends (Python and JSON-backed tooling) emit .air as
        the canonical IR.
        The APXM compiler consumes .air and produces .apxmobj artifacts.

        Example output:
            ; Agent IR (.air) - canonical intermediate representation
            ; graph: research_pipeline
            ; is_entry: true

            ; params:
            ;   %topic: str

              %research = ais.ask {template_str = "Research: {0}"}
              %critique = ais.ask {template_str = "Critique: {0}"}
              %sync = ais.wait_all

              ; edges:
              ;   %research -> %sync (Control)
              ;   %critique -> %sync (Control)
        """
        lines: list[str] = []

        # Header
        lines.append("; Agent IR (.air) - canonical intermediate representation")
        lines.append(f"; graph: {self.name}")
        for k, v in self.metadata.items():
            lines.append(f"; {k}: {v}")

        # Parameters
        if self.parameters:
            lines.append("")
            lines.append("; params:")
            for p in self.parameters:
                lines.append(f";   %{p.name}: {p.type_name}")

        lines.append("")

        # Build node id -> name map for edge rendering
        id_to_name: dict[int, str] = {n.id: n.name for n in self.nodes}

        # Nodes as SSA ops
        for node in self.nodes:
            op = node.op.lower()
            # Build attribute string, skip internal attrs starting with _
            attrs = {k: v for k, v in node.attributes.items() if not k.startswith("_")}
            if attrs:
                attr_parts = []
                for k, v in attrs.items():
                    if isinstance(v, str):
                        # Truncate long strings for readability
                        display = v[:60] + "..." if len(v) > 60 else v
                        attr_parts.append(f'{k} = "{display}"')
                    else:
                        attr_parts.append(f"{k} = {v}")
                attr_str = f" {{{', '.join(attr_parts)}}}"
            else:
                attr_str = ""
            lines.append(f"  %{node.name} = ais.{op}{attr_str}")

        # Edges as comments
        if self.edges:
            lines.append("")
            lines.append("  ; edges:")
            for edge in self.edges:
                from_name = id_to_name.get(edge.from_id, f"?{edge.from_id}")
                to_name = id_to_name.get(edge.to_id, f"?{edge.to_id}")
                lines.append(f"  ;   %{from_name} -> %{to_name} ({edge.dependency})")

        lines.append("")
        return "\n".join(lines)

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "ApxmGraph":
        return cls(
            name=str(value["name"]),
            nodes=[GraphNode.from_dict(node) for node in value.get("nodes", [])],
            edges=[GraphEdge.from_dict(edge) for edge in value.get("edges", [])],
            parameters=[
                Parameter.from_dict(parameter) for parameter in value.get("parameters", [])
            ],
            metadata=dict(value.get("metadata", {})),
        )

    @classmethod
    def from_json(cls, payload: str | bytes | bytearray) -> "ApxmGraph":
        if isinstance(payload, (bytes, bytearray)):
            payload = payload.decode("utf-8")
        return cls.from_dict(json.loads(payload))

    @classmethod
    def merge(cls, name: str, graphs: list["ApxmGraph"]) -> "ApxmGraph":
        """Merge multiple sub-graphs, remapping node IDs to avoid collisions.

        Adds a WAIT_ALL synchronization node collecting all sub-graph exit nodes.
        Parameters are deduplicated by name (first wins).
        """
        if not graphs:
            return cls(name=name)

        merged_nodes: list[GraphNode] = []
        merged_edges: list[GraphEdge] = []
        merged_params: list[Parameter] = []
        merged_metadata: dict[str, Any] = {}
        seen_param_names: set[str] = set()

        id_offset = 0

        for graph in graphs:
            # Build remap table
            remap: dict[int, int] = {node.id: node.id + id_offset for node in graph.nodes}

            # Add remapped nodes
            for node in graph.nodes:
                merged_nodes.append(
                    GraphNode(
                        id=remap[node.id],
                        name=node.name,
                        op=node.op,
                        attributes=dict(node.attributes),
                    )
                )

            # Add remapped edges
            for edge in graph.edges:
                merged_edges.append(
                    GraphEdge(
                        from_id=remap[edge.from_id],
                        to_id=remap[edge.to_id],
                        dependency=edge.dependency,
                    )
                )

            # Deduplicate parameters (first wins)
            for param in graph.parameters:
                if param.name not in seen_param_names:
                    seen_param_names.add(param.name)
                    merged_params.append(Parameter(name=param.name, type_name=param.type_name))

            # Merge metadata (first wins)
            for k, v in graph.metadata.items():
                if k not in merged_metadata:
                    merged_metadata[k] = v

            # Advance offset past max ID in this sub-graph
            if graph.nodes:
                id_offset += max(node.id for node in graph.nodes)

        # Find exit nodes (no outgoing edges)
        sources = {edge.from_id for edge in merged_edges}
        exit_ids = [node.id for node in merged_nodes if node.id not in sources]

        # Create WAIT_ALL sync node
        wait_all_id = max(node.id for node in merged_nodes) + 1
        merged_nodes.append(
            GraphNode(id=wait_all_id, name=f"{name}_sync", op=operations.WAIT_ALL.op)
        )

        # Control edges from exit nodes to sync
        for exit_id in exit_ids:
            merged_edges.append(
                GraphEdge(from_id=exit_id, to_id=wait_all_id, dependency="Control")
            )

        return cls(
            name=name,
            nodes=merged_nodes,
            edges=merged_edges,
            parameters=merged_params,
            metadata=merged_metadata,
        )


# ============================================================================
# APXM contract validation
# ============================================================================

# Required attributes per operation, derived from Rust AIS_OPERATIONS when
# the native module is available.  Falls back to a hardcoded dict aligned
# with the JSON wire-format keys from apxm-core/src/constants.rs.
# Note: some OperationSpec field names in definitions.rs differ from the
# wire-format keys (e.g. "label_true" vs "true_label") because the MLIR
# compiler performs attribute-name translation at emission time.  Ops whose
# required inputs come solely from graph edges (MERGE, WAIT_ALL, RETURN)
# have empty sets here — the compiler resolves them from edge topology.
try:
    from apxm._native import get_ais_operations as _get_ais_operations

    _REQUIRED_ATTRS: dict[str, set[str]] = {
        spec["op"]: set(spec["required_fields"])
        for spec in _get_ais_operations()
    }
except ImportError:
    # Fallback: use generated operation specs
    _REQUIRED_ATTRS: dict[str, set[str]] = {
        spec.op: set(spec.required_fields)
        for spec in operations.ALL_OPERATIONS
    }

# Valid parameter type_name values accepted by the APXM runtime.
_VALID_PARAM_TYPES: set[str] = {"str", "int", "float", "bool", "json"}


@dataclass(slots=True)
class ValidationResult:
    """Result of validating an ApxmGraph against the APXM contract."""

    valid: bool
    errors: list[str]
    warnings: list[str]


def validate_against_apxm(graph: ApxmGraph) -> ValidationResult:
    """Validate an ``ApxmGraph`` against the APXM compiler contract.

    Checks performed:

    * All operation names are valid AIS ops.
    * Required attributes present for each op type (warning, not error,
      because the Python SDK often sets slightly different key names).
    * All node IDs are unique and positive.
    * Edges reference existing node IDs.
    * Parameter ``type_name`` values are valid.
    * Graph name is non-empty.
    * No duplicate parameter names.
    * Graph edges form a DAG (no cycles).
    """
    errors: list[str] = []
    warnings: list[str] = []

    # --- graph-level checks ------------------------------------------------
    if not graph.name:
        errors.append("graph name must not be empty")

    if not graph.nodes:
        errors.append("graph must contain at least one node")

    # --- node checks -------------------------------------------------------
    node_ids: set[int] = set()
    for node in graph.nodes:
        # unique positive IDs
        if node.id <= 0:
            errors.append(f"node '{node.name}' has non-positive id {node.id}")
        if node.id in node_ids:
            errors.append(f"duplicate node id {node.id}")
        node_ids.add(node.id)

        # empty name
        if not node.name:
            errors.append(f"node id={node.id} has empty name")

        # valid op
        if node.op not in _REQUIRED_ATTRS:
            errors.append(
                f"node '{node.name}' (id={node.id}) has unknown op '{node.op}'"
            )
        else:
            # check required attributes (as warnings -- SDK may use aliases)
            required = _REQUIRED_ATTRS[node.op]
            present = set(node.attributes.keys())
            missing = required - present
            if missing:
                warnings.append(
                    f"node '{node.name}' ({node.op}): missing recommended "
                    f"attributes {sorted(missing)}"
                )

    # --- edge checks -------------------------------------------------------
    for edge in graph.edges:
        if edge.from_id not in node_ids:
            errors.append(
                f"edge references non-existent from_id {edge.from_id}"
            )
        if edge.to_id not in node_ids:
            errors.append(
                f"edge references non-existent to_id {edge.to_id}"
            )

    # --- parameter checks --------------------------------------------------
    param_names: set[str] = set()
    for param in graph.parameters:
        if not param.name:
            errors.append("parameter with empty name")
        if param.name in param_names:
            errors.append(f"duplicate parameter name '{param.name}'")
        param_names.add(param.name)
        if param.type_name not in _VALID_PARAM_TYPES:
            warnings.append(
                f"parameter '{param.name}' has non-standard type_name "
                f"'{param.type_name}'"
            )

    # --- DAG cycle check (Kahn's algorithm) --------------------------------
    if graph.nodes and graph.edges:
        from .utils import detect_cycle

        edges = [
            (edge.from_id, edge.to_id)
            for edge in graph.edges
            if edge.from_id in node_ids and edge.to_id in node_ids
        ]
        cycle_count = detect_cycle(node_ids, edges)
        if cycle_count:
            errors.append(
                f"graph contains a cycle ({cycle_count} nodes involved)"
            )

    return ValidationResult(
        valid=len(errors) == 0,
        errors=errors,
        warnings=warnings,
    )
