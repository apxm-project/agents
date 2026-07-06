from __future__ import annotations

from dataclasses import dataclass, field
import json
import subprocess
from typing import Any

from apxm._generated import constants as c
from apxm._generated import operations
from apxm.constants import (
    DEPENDENCY_CONTROL,
    DEPENDENCY_DATA,
    DEPENDENCY_EFFECT,
    normalize_dependency_type,
)
from apxm.hooks import GATE_LIFECYCLE_EVENTS, HookMode, LIFECYCLE_EVENTS
from apxm._generated.operations import ASK, COMMUNICATE, REASON, SPAWN_AGENT, THINK


_DEPENDENCY_TYPES = {DEPENDENCY_DATA, DEPENDENCY_EFFECT, DEPENDENCY_CONTROL}
_LLM_TURN_OPS = frozenset({ASK.op, THINK.op, REASON.op})


class AirEmissionError(RuntimeError):
    """Raised when the compiler-owned AIR emitter cannot produce AIR."""


@dataclass(frozen=True, slots=True)
class AirEmitterCommand:
    """Subprocess command that emits AIR from the shared frontend graph DTO."""

    argv: tuple[str, ...]

    @classmethod
    def from_environment(cls) -> "AirEmitterCommand":
        # One resolver for both execution and AIR emission: APXM_BIN, then `apxm`
        # on PATH, then the repo-local build. The single Rust printer is reached
        # via `<apxm> emit-air` — no separate dekk subcommand path. Lazy import
        # breaks the ir <-> execution module cycle.
        from apxm.execution import _build_cli_base_command, _find_apxm_binary

        base = _build_cli_base_command(_find_apxm_binary())
        return cls((*base, "emit-air"))

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
            from_id=int(value.get("from", value.get("from_id"))),
            to_id=int(value.get("to", value.get("to_id"))),
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
            builder reads is ``is_entry`` (bool or a truthy string —
            ``"true"``/``"1"``/``"yes"``, case-insensitive): when true (the
            default), the emitted function carries `attributes {ais.entry}`.
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

    def to_func_air(self) -> str:
        """Emit just the `func.func @<name>(...) { ... }` block (no module wrapper).

        Used by the multi-flow module emitter so several graphs can share one
        `module { ... }` — a `ConversationalAgent` lowers to one entry loop flow,
        one turn flow, and one flow per sub-agent, all inside a single module.
        """
        air = self.to_air()
        body = air.split("\n")
        if body and body[0].strip() == "module {":
            body = body[1:]
        if body and body[-1].strip() == "}":
            body = body[:-1]
        return "\n".join(body)

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


# ============================================================================
# APXM contract validation
# ============================================================================

# Required attributes per operation, derived from Rust AIS_OPERATIONS when
# the native module is available. Falls back to generated operation specs.
# Attribute names are canonicalized in the shared apxm-core contract. Ops whose
# required inputs come solely from graph edges (MERGE, WAIT_ALL, RETURN) have
# empty sets here — the compiler resolves them from edge topology.
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

    errors.extend(_validate_spawn_communicate_dependencies(graph))
    errors.extend(_validate_llm_operation_attributes(graph))
    errors.extend(_validate_register_hook(graph))

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


def _validate_register_hook(graph: ApxmGraph) -> list[str]:
    """AIR validation for REGISTER_HOOK nodes.

    Flags: unknown ``hook_event``; ``gate`` mode on a non-pre event; missing
    ``python_hook_handler_id``.
    """
    errors: list[str] = []
    for node in graph.nodes:
        if node.op != "REGISTER_HOOK":
            continue
        event = node.attributes.get(c.HOOK_EVENT)
        if not isinstance(event, str) or event not in LIFECYCLE_EVENTS:
            valid = ", ".join(sorted(LIFECYCLE_EVENTS))
            errors.append(
                f"node '{node.name}' (REGISTER_HOOK) has unknown hook_event "
                f"{event!r}; expected one of {valid}"
            )
            continue
        mode = node.attributes.get(c.HOOK_MODE, HookMode.OBSERVE.value)
        if mode == HookMode.GATE.value and event not in GATE_LIFECYCLE_EVENTS:
            errors.append(
                f"node '{node.name}' (REGISTER_HOOK) uses gate mode on non-pre "
                f"event {event!r}; gate is only valid on pre-execution events"
            )
        handler = node.attributes.get(c.PYTHON_HOOK_HANDLER_ID)
        if not isinstance(handler, str) or not handler:
            errors.append(
                f"node '{node.name}' (REGISTER_HOOK) is missing "
                f"python_hook_handler_id"
            )
    return errors


def _validate_llm_operation_attributes(graph: ApxmGraph) -> list[str]:
    errors: list[str] = []
    for node in graph.nodes:
        if c.LLM_OPERATION not in node.attributes:
            continue
        value = node.attributes[c.LLM_OPERATION]
        if not isinstance(value, str):
            errors.append(
                f"node '{node.name}' ({node.op}) has non-string "
                f"{c.LLM_OPERATION!r} attribute"
            )
            continue
        if value not in _LLM_TURN_OPS:
            valid = ", ".join(sorted(_LLM_TURN_OPS))
            errors.append(
                f"node '{node.name}' ({node.op}) has invalid "
                f"{c.LLM_OPERATION!r} value {value!r}; expected one of {valid}"
            )
    return errors


def _validate_spawn_communicate_dependencies(graph: ApxmGraph) -> list[str]:
    spawned: dict[str, int] = {}
    errors: list[str] = []

    for node in graph.nodes:
        if node.op != SPAWN_AGENT.op:
            continue
        agent_name = node.attributes.get(c.AGENT_NAME)
        if isinstance(agent_name, str):
            spawned[agent_name] = node.id

    data_edges = [
        (edge.from_id, edge.to_id)
        for edge in graph.edges
        if edge.dependency == DEPENDENCY_DATA
    ]
    control_edges = {
        (edge.from_id, edge.to_id)
        for edge in graph.edges
        if edge.dependency == DEPENDENCY_CONTROL
    }

    for node in graph.nodes:
        if node.op != COMMUNICATE.op:
            continue
        recipient = node.attributes.get(c.RECIPIENT)
        if not isinstance(recipient, str) or recipient not in spawned:
            continue
        spawn_id = spawned[recipient]
        message_input_count = _input_names_count(node.attributes.get(c.INPUT_NAMES))
        data_sources = [
            from_id for from_id, to_id in data_edges if to_id == node.id
        ]
        structural_sources = data_sources[message_input_count:]
        if any(_has_data_path(spawn_id, source_id, data_edges) for source_id in structural_sources):
            continue
        if (spawn_id, node.id) in control_edges:
            hint = "Control edges do not carry the spawn token into AIR operands"
        else:
            hint = "no Data dependency path from the matching SPAWN_AGENT was found"
        errors.append(
            f"node '{node.name}' ({node.op}) targets spawned agent '{recipient}' "
            f"but does not depend on its SPAWN_AGENT token: {hint}"
        )

    return errors


def _has_data_path(
    source_id: int,
    target_id: int,
    data_edges: list[tuple[int, int]],
) -> bool:
    adjacency: dict[int, list[int]] = {}
    for from_id, to_id in data_edges:
        adjacency.setdefault(from_id, []).append(to_id)

    visited: set[int] = set()
    pending = [source_id]
    while pending:
        current = pending.pop()
        if current in visited:
            continue
        visited.add(current)
        for next_id in adjacency.get(current, []):
            if next_id == target_id:
                return True
            pending.append(next_id)
    return False


def _input_names_count(value: Any) -> int:
    if isinstance(value, list):
        return len(value)
    if isinstance(value, str):
        return 1
    return 0
