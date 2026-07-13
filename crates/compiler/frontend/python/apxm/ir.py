from __future__ import annotations

from dataclasses import dataclass, field
import json
import subprocess
from typing import Any

from apxm._generated import operations
from apxm.constants import (
    DEPENDENCY_CONTROL,
    DEPENDENCY_DATA,
    DEPENDENCY_EFFECT,
    normalize_dependency_type,
)


_DEPENDENCY_TYPES = {DEPENDENCY_DATA, DEPENDENCY_EFFECT, DEPENDENCY_CONTROL}


class AirEmissionError(RuntimeError):
    """Raised when the compiler-owned AIR emitter cannot produce AIR."""


@dataclass(frozen=True, slots=True)
class AirEmitterCommand:
    """Subprocess command that emits AIR from the shared frontend graph DTO."""

    argv: tuple[str, ...]

    @classmethod
    def from_environment(cls) -> "AirEmitterCommand":
        # APXM_BIN names the installed compiler CLI; otherwise it must be
        # available as `apxm` on PATH. Lazy import breaks the ir <-> execution
        # module cycle.
        from apxm.execution import _find_apxm_binary

        return cls((_find_apxm_binary(), "emit-air"))

    def emit(self, payload: Any) -> str:
        try:
            result = subprocess.run(
                self.argv,
                input=json.dumps(payload),
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
        except OSError as exc:
            command = " ".join(self.argv)
            raise AirEmissionError(f"{command} could not be started: {exc}") from exc

        if result.returncode != 0:
            command = " ".join(self.argv)
            detail = result.stderr.strip() or f"exit status {result.returncode}"
            raise AirEmissionError(f"{command} failed: {detail}")
        return result.stdout


@dataclass(slots=True)
class GraphNode:
    """One recorded operation in an :class:`ApxmGraph`.

    This is the shared frontend-internal node shape. See
    ``crates/compiler/frontend/python/docs/graph-model.md`` for the formal
    contract every frontend emitter must satisfy.

    Fields:
        id: Positive, unique integer node id within the owning graph. IDs are
            assigned in recording order starting at 1 (see
            ``GraphRecorder._add_node``); they are never reused and do not
            need to be contiguous after graph transforms (e.g. ``merge()``
            remaps them).
        name: The node's stable identifier stem. It must be unique within the
            graph because the Rust AIR builder uses it when assigning SSA names.
        op: The upper-case AIS operation name (e.g. ``"ASK"``,
            ``"SPAWN_AGENT"``, ``"COMMUNICATE"``). Must be a key in the
            generated op catalog (``apxm._generated.operations``).
        attributes: Op-specific, JSON-plain key/value attributes (str, int,
            float, bool, list, dict, or ``None``). Values must already be
            JSON-serializable; ``GraphRecorder`` helpers normalize Python-native
            inputs before storing them here.
    """

    id: int
    name: str
    op: str
    attributes: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        """Serialize to the canonical plain-dict wire shape.

        ``{"id": int, "name": str, "op": str, "attributes": dict}`` — every
        value is JSON-plain (no custom types), so ``json.dumps(node.to_dict())``
        round-trips through :meth:`from_dict` unchanged. This is the exact
        shape a TS frontend's ``GraphNode`` interface
        (``typescript/src/graph.ts``) must structurally match field-for-field
        (``id``, ``name``, ``op``, ``attributes``).
        """
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
    """A directed dependency between two :class:`GraphNode` ids.

    Fields:
        from_id: Source node id (must exist in the owning graph's ``nodes``).
        to_id: Destination node id (must exist in the owning graph's
            ``nodes``).
        dependency: One of ``"Data"``, ``"Control"``, or ``"Effect"`` (see
            ``apxm.constants.DEPENDENCY_DATA/_CONTROL/_EFFECT``). Only
            ``"Data"`` edges become SSA operands in emitted MLIR
            (``to_air()`` reads incoming Data edges as an op's inputs);
            ``"Control"``/``"Effect"`` edges affect topological ordering only
            and are otherwise invisible in the emitted text. Normalized and
            validated in ``__post_init__`` — constructing a ``GraphEdge``
            with an unrecognized dependency string raises ``ValueError``.
    """

    from_id: int
    to_id: int
    dependency: str = DEPENDENCY_DATA

    def __post_init__(self) -> None:
        self.dependency = normalize_dependency_type(self.dependency)
        if self.dependency not in _DEPENDENCY_TYPES:
            allowed = ", ".join(sorted(_DEPENDENCY_TYPES))
            raise ValueError(f"invalid dependency '{self.dependency}', expected one of: {allowed}")

    def to_dict(self) -> dict[str, Any]:
        """Serialize to ``{"from": int, "to": int, "dependency": str}``.

        Field names ``from``/``to`` (not ``from_id``/``to_id``) match the TS
        ``GraphEdge`` interface (``typescript/src/graph.ts``) exactly, so the
        two frontends' edge dicts are structurally identical on the wire.
        """
        return {
            "from": self.from_id,
            "to": self.to_id,
            "dependency": self.dependency,
        }

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "GraphEdge":
        return cls(
            from_id=int(value["from"]),
            to_id=int(value["to"]),
            dependency=normalize_dependency_type(value.get("dependency", DEPENDENCY_DATA)),
        )


@dataclass(slots=True)
class Parameter:
    """A declared compile-time parameter of an :class:`ApxmGraph`.

    Emitted as a `!ais.token` function argument with
    `{ais.param_name, ais.param_type}` metadata. Parameters are referenced in op
    templates as ``{name}``; the compiled artifact substitutes them at
    scheduler init, distinct from ``NodeRef``-bound `{name}` placeholders
    which the runtime template renderer resolves from produced values.

    Fields:
        name: Parameter name; must be a valid Python identifier fragment and
            unique within the graph's ``parameters`` list (``ApxmGraph``
            validation flags duplicates).
        type_name: One of the runtime's accepted parameter type strings
            (``"str"``, ``"int"``, ``"float"``, ``"bool"``, ``"json"`` — see
            ``apxm._generated.constants.VALID_PARAM_TYPES``). Non-standard
            values are accepted but flagged as a warning by
            :func:`validate_against_apxm`.
    """

    name: str
    type_name: str

    def to_dict(self) -> dict[str, Any]:
        """Serialize to ``{"name": str, "type_name": str}`` — matches the TS
        ``Parameter`` interface's ``{name, typeName}`` field-for-field (TS
        uses camelCase ``typeName`` per its own naming convention, but
        serializes to the same ``type_name`` key on the wire via
        ``toDict()``)."""
        return {"name": self.name, "type_name": self.type_name}

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "Parameter":
        return cls(name=str(value["name"]), type_name=str(value["type_name"]))


@dataclass(slots=True)
class ApxmGraph:
    """The shared frontend-internal graph model.

    ``ApxmGraph`` is produced by ``GraphRecorder.to_graph()`` (the recorder
    that backs Python's ``@compile()``/``g.<op>()`` authoring surface) and is
    this frontend's in-memory representation of one compiled flow — a plain
    DAG of :class:`GraphNode`, :class:`GraphEdge`, declared
    :class:`Parameter`, and free-form ``metadata``. It is **not** a server
    wire format: the only artifact that crosses the frontend/runtime boundary
    is the `.air` text produced by :meth:`to_air`. ``ApxmGraph`` (and its
    :meth:`to_dict` plain-dict form) exists so frontend-internal tooling
    (tests, graph transforms like :meth:`merge`, validation) has a typed,
    serializable representation to work against before lowering to MLIR
    text.

    TypeScript's ``@apxm/frontend`` package
    (``crates/compiler/frontend/typescript/src/graph.ts``,
    `ApxmGraphData`/`ApxmGraph`) implements the same contract. See
    ``crates/compiler/frontend/python/docs/graph-model.md`` for the formal
    field-by-field graph/AIR contract both frontends must satisfy.

    Fields:
        name: The flow's name. The Rust AIR builder sanitizes it into an MLIR
            function symbol.
        nodes: All recorded operations, in recording order. Order does not
            determine execution order — :meth:`to_air` topologically sorts
            by :attr:`edges` before emitting.
        edges: All recorded dependencies between node ids. See
            :class:`GraphEdge` for the three dependency kinds and how each
            affects emission.
        parameters: Declared compile-time parameters, in declaration order.
            Becomes the emitted function's argument list, one `!ais.token`
            per parameter in this order.
        metadata: Free-form graph-level flags. The one field the Rust AIR
            builder reads is ``is_entry`` (a Boolean): when true, the emitted
            function carries `attributes {ais.entry}`. Every emitted program
            declares exactly one entry flow explicitly.
            Any other key is preserved by :meth:`to_dict`/:meth:`from_dict`
            but not otherwise interpreted by this class.
    """

    name: str
    nodes: list[GraphNode] = field(default_factory=list)
    edges: list[GraphEdge] = field(default_factory=list)
    parameters: list[Parameter] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        """Serialize to the canonical plain-dict graph shape.

        ::

            {
              "name": str,
              "nodes": [GraphNode.to_dict(), ...],
              "edges": [GraphEdge.to_dict(), ...],
              "parameters": [Parameter.to_dict(), ...],
              "metadata": dict,
            }

        Every value is JSON-plain, so ``json.dumps(graph.to_dict())``
        round-trips through :meth:`from_dict` unchanged. This exact shape
        (same top-level keys, same nested field names) is what TypeScript's
        ``ApxmGraph.toDict()`` (``typescript/src/graph.ts``) must also
        produce for an equivalent graph — the shared contract documented in
        ``crates/compiler/frontend/python/docs/graph-model.md``.
        """
        return {
            "name": self.name,
            "nodes": [node.to_dict() for node in self.nodes],
            "edges": [edge.to_dict() for edge in self.edges],
            "parameters": [parameter.to_dict() for parameter in self.parameters],
            "metadata": dict(self.metadata),
        }

    def to_air(self) -> str:
        """Emit valid MLIR text for this graph through the Rust AIR builder."""
        return AirEmitterCommand.from_environment().emit(self.to_dict())

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
                GraphEdge(from_id=exit_id, to_id=wait_all_id, dependency=DEPENDENCY_CONTROL)
            )

        return cls(
            name=name,
            nodes=merged_nodes,
            edges=merged_edges,
            parameters=merged_params,
            metadata=merged_metadata,
        )


def emit_multi_flow_module(graphs: list["ApxmGraph"]) -> str:
    """Serialize captured graphs into one AIR module through the Rust builder."""
    return AirEmitterCommand.from_environment().emit([graph.to_dict() for graph in graphs])


@dataclass(slots=True)
class ValidationResult:
    """Result of validating an ApxmGraph against the APXM contract."""

    valid: bool
    errors: list[str]
    warnings: list[str]


def validate_against_apxm(graph: ApxmGraph) -> ValidationResult:
    """Validate an ``ApxmGraph`` through the compiler-owned AIR builder."""
    try:
        graph.to_air()
    except Exception as exc:
        return ValidationResult(valid=False, errors=[str(exc)], warnings=[])
    return ValidationResult(valid=True, errors=[], warnings=[])
