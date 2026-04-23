from __future__ import annotations

from dataclasses import dataclass, field
import json
import os
import re
from pathlib import Path
from typing import Any

from apxm._generated import constants as c
from apxm._generated import operations
from apxm._generated.emission import EMITTERS, TEMPLATE_ATTRS, VOID_OPS


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
        """Emit valid MLIR text for this graph.

        The .air format now produces VALID MLIR that can be parsed directly by
        MLIR's Module::parse(). No custom parser needed.

        Example output:
            module {
              func.func @research_pipeline(%arg0: !ais.token {ais.param_name = "topic", ais.param_type = "str"}) -> !ais.token attributes {ais.entry} {
                %research = ais.ask "Research: {0}" [%arg0 : !ais.token] : !ais.token
                %critique = ais.ask "Critique: {0}" [%arg0 : !ais.token] : !ais.token
                %sync = ais.wait_all %research, %critique : !ais.token, !ais.token -> !ais.token
                func.return %sync : !ais.token
              }
            }
        """
        from .utils import topological_sort

        # Topological sort
        node_ids = [n.id for n in self.nodes]
        edges = [(e.from_id, e.to_id) for e in self.edges]
        try:
            order = topological_sort(node_ids, edges)
        except ValueError as e:
            # Fallback: use node order as-is if cycle detection fails
            order = node_ids

        # Build adjacency structures
        incoming: dict[int, list[int]] = {nid: [] for nid in node_ids}
        outgoing: dict[int, list[int]] = {nid: [] for nid in node_ids}
        for edge in self.edges:
            if edge.from_id in incoming and edge.to_id in incoming:
                # Only include Data edges as inputs (Control/Effect are for sequencing only)
                if edge.dependency == "Data":
                    incoming[edge.to_id].append(edge.from_id)
                outgoing[edge.from_id].append(edge.to_id)

        # Map node_id -> node
        nodes_by_id = {n.id: n for n in self.nodes}

        # Track produced SSA values
        produced: dict[int, str] = {}

        # Function arguments (parameters) — referenced by templates as `{name}`,
        # rewritten below to `{{name}}` so the runtime substitutes them at
        # scheduler init. Compile params are not wired as Data inputs.
        param_names = {p.name for p in self.parameters}

        lines: list[str] = []

        # Emit nodes in topological order
        for node_id in order:
            node = nodes_by_id[node_id]
            ssa_name = f"%{node.name}"

            # Get inputs from incoming edges
            inputs = [produced[src_id] for src_id in incoming[node_id] if src_id in produced]

            # Rewrite `{param_name}` → `{{param_name}}` in template attrs so
            # the scheduler substitutes compile params at init (state.rs:154).
            # NodeRef-bound names (in `input_names`) are left as `{name}` for
            # the runtime template renderer.
            if param_names:
                input_names_attr = node.attributes.get(c.INPUT_NAMES) or []
                input_name_set = set(input_names_attr)
                for attr_name in TEMPLATE_ATTRS:
                    if attr_name in node.attributes:
                        text = node.attributes[attr_name]
                        if isinstance(text, str):
                            node.attributes[attr_name] = re.sub(
                                r'\{(\w+)\}',
                                lambda m: (
                                    f'{{{{{m.group(1)}}}}}'
                                    if m.group(1) in param_names
                                    and m.group(1) not in input_name_set
                                    else m.group(0)
                                ),
                                text,
                            )

            # RETURN nodes are handled by the func.return at the end
            if node.op.upper() == "RETURN":
                if inputs:
                    produced[node_id] = inputs[0]
                continue

            # Emit the operation
            mlir_line = self._emit_mlir_op(node, ssa_name, inputs)
            if mlir_line:
                lines.append(f"    {mlir_line}")

            # Track produced value
            if not self._is_void_op(node.op):
                produced[node_id] = ssa_name
            elif inputs:
                produced[node_id] = inputs[0]

        # Find exit nodes (no outgoing edges) and emit func.return
        exit_nodes = [nid for nid in node_ids if not outgoing[nid]]

        # Emit func.return
        return_vals = [produced[nid] for nid in exit_nodes if nid in produced]
        if not return_vals:
            # No exit nodes with values, find last produced value
            for nid in reversed(order):
                if nid in produced:
                    return_vals = [produced[nid]]
                    break

        if return_vals:
            if len(return_vals) == 1:
                lines.append(f"    func.return {return_vals[0]} : !ais.token")
            else:
                # Merge multiple return values
                merged = "%ret_merge"
                operands = ", ".join(return_vals)
                types = ", ".join(["!ais.token"] * len(return_vals))
                lines.append(f"    {merged} = ais.merge {operands} : {types} -> !ais.token")
                lines.append(f"    func.return {merged} : !ais.token")
        else:
            # No values produced, create a const token
            lines.append("    %result = ais.const_str \"result\" : !ais.token")
            lines.append("    func.return %result : !ais.token")

        # Build function signature
        func_name = self._sanitize_name(self.name)
        args = []
        for i, param in enumerate(self.parameters):
            args.append(
                f"%arg{i}: !ais.token {{ais.param_name = \"{param.name}\", ais.param_type = \"{param.type_name}\"}}"
            )
        args_str = ", ".join(args)

        # Check if this is an entry flow
        is_entry = self.metadata.get("is_entry", True)
        if isinstance(is_entry, bool):
            pass
        elif isinstance(is_entry, str):
            is_entry = is_entry.lower() in ("true", "1", "yes")
        else:
            is_entry = bool(is_entry)

        attrs_str = " attributes {ais.entry}" if is_entry else ""

        # Assemble module
        mlir_lines = ["module {"]
        mlir_lines.append(f"  func.func @{func_name}({args_str}) -> !ais.token{attrs_str} {{")
        mlir_lines.extend(lines)
        mlir_lines.append("  }")
        mlir_lines.append("}")

        return "\n".join(mlir_lines)

    def _sanitize_name(self, name: str) -> str:
        """Sanitize graph name for use as MLIR function name."""
        # Replace spaces and special chars with underscores
        import re
        sanitized = re.sub(r'[^a-zA-Z0-9_]', '_', name)
        # Ensure it doesn't start with a digit
        if sanitized and sanitized[0].isdigit():
            sanitized = f"flow_{sanitized}"
        return sanitized or "unnamed_flow"

    def _is_void_op(self, op: str) -> bool:
        """Check if an operation produces no result (void)."""
        return op.upper() in VOID_OPS

    def _emit_op(self, op: str, ssa_name: str, attrs: dict[str, Any], inputs: list[str]) -> str:
        """Dispatch to auto-generated emitters, with a minimal generic fallback."""
        fn = EMITTERS.get(op)
        if fn is not None:
            return fn(ssa_name, attrs, inputs)
        # Unknown op — generic fallback
        ctx = f" [{', '.join(inputs)} : {', '.join(['!ais.token'] * len(inputs))}]" if inputs else ""
        return f"{ssa_name} = ais.{op.lower()}{ctx} : !ais.token"

    def _emit_mlir_op(self, node: GraphNode, ssa_name: str, inputs: list[str]) -> str:
        """Emit MLIR assembly for a single operation."""
        return self._emit_op(node.op.upper(), ssa_name, node.attributes, inputs)

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


def load_graph(path: str | os.PathLike[str]) -> ApxmGraph:
    """Load a graph from a JSON file.

    Args:
        path: Path to a .json graph file
    """
    p = Path(path)
    text = p.read_text(encoding="utf-8")
    return ApxmGraph.from_json(text)


# ============================================================================
# APXM contract validation
# ============================================================================

# Required attributes per operation, derived from Rust AIS_OPERATIONS when
# the native module is available. Falls back to generated operation specs.
# Attribute names are canonicalized in the shared apxm-core contract.
# Ops whose
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
        spec.op: {f.name for f in spec.fields if f.required}
        for spec in operations.ALL_OPERATIONS
    }

# Valid parameter type_name values accepted by the APXM runtime.
try:
    from apxm._generated.constants import VALID_PARAM_TYPES as _VALID_PARAM_TYPES
except ImportError:
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
