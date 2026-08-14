"""Immutable typed source tree for authored Agent programs.

The tree is the frontend's semantic representation between the native Python AST
and the language-neutral FrontendGraph. It retains lexical constructs, resolved
declarations, inferred type references, and source spans. It carries no AIR or
AIS operation name, no runtime value, and no mutable recorder state; it is frozen
after construction so traversal cannot depend on marker execution order.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Optional, Union

from ._generated.frontend_records import CallIntent, ControlIntent
from ._generated.permissions import Permission


@dataclass(frozen=True, slots=True)
class Span:
    """A source location range for one construct."""

    source_file: str
    start_line: int
    start_column: int
    end_line: int
    end_column: int


@dataclass(frozen=True, slots=True)
class BoundParameter:
    """One typed callback parameter with its resolved role."""

    value_id: str
    type_ref: str
    role: str  # agent_facade | input | context | ordinary


@dataclass(frozen=True, slots=True)
class BoundDeclaration:
    """A resolved Context, Model, Tool, Capability, or Event declaration."""

    decl_id: str
    decl_kind: str  # context | model_binding | tool_binding | capability_binding | event_type
    input_type_ref: str
    output_type_ref: str
    target_ref: Optional[str] = None
    context_default_present: Optional[bool] = None


@dataclass(frozen=True, slots=True)
class BoundValue:
    """One typed value with exactly one origin."""

    value_id: str
    type_ref: str
    origin: str  # parameter | call_result | block_argument | context_value | literal | resume_input
    origin_id: Optional[str] = None
    expression: Optional[dict[str, object]] = None


@dataclass(frozen=True, slots=True)
class BoundOperand:
    """A typed use of a value at a named consumer slot."""

    value_id: str
    slot: str


@dataclass(frozen=True, slots=True)
class BoundPredicate:
    """A closed structural predicate over one prior typed value."""

    root_value_id: str
    property_path: tuple[str, ...]
    comparator: str  # truthy | equals | not_equals
    literal: Optional[dict[str, object]] = None


@dataclass(frozen=True, slots=True)
class BoundCall:
    """A resolved effectful call: model, tool, capability, agent, or event.

    Carries a generated ``CallIntent`` contract record plus the authoring-time
    extras the contract does not state: a source ``span`` and typed
    ``operands`` with consumer-slot identity (the contract instead carries
    ``operand_values`` and leaves slot identity to ``data_edges``). The flat
    properties below read through to ``contract`` so every other reader of a
    ``BoundCall`` is unaffected by this split.
    """

    contract: CallIntent
    span: Optional[Span]
    operands: tuple[BoundOperand, ...]

    @property
    def node_id(self) -> str:
        return self.contract.node_id

    @property
    def intent_kind(self) -> str:
        return self.contract.intent_kind

    @property
    def parent_region_id(self) -> str:
        return self.contract.parent_region_id

    @property
    def execution_order(self) -> int:
        return self.contract.execution_order

    @property
    def binding_ref(self) -> Optional[str]:
        return self.contract.binding_ref

    @property
    def receiver_kind(self) -> Optional[str]:
        return self.contract.receiver_kind

    @property
    def result_value(self) -> Optional[str]:
        return self.contract.result_value


@dataclass(frozen=True, slots=True)
class BoundControl:
    """A resolved structural construct: branch, loop, task group, try, yield, return.

    Carries a generated ``ControlIntent`` contract record plus the
    authoring-time extras the contract does not state: a source ``span`` and
    typed ``operands`` with consumer-slot identity. ``predicate`` is kept as
    its own field rather than populated on ``contract``: the generated
    ``ControlIntent.predicate`` is typed as the generated ``ControlPredicate``
    union, whose ``EqualsPredicate``/``NotEqualsPredicate`` branches carry a
    generated ``PredicateLiteral``, and threading capture's predicate
    construction through those generated types is out of this change's scope
    (see the frontend-vocabulary-generation design note, §4 item 2). The flat
    properties below read through to ``contract`` so every other reader of a
    ``BoundControl`` is unaffected by this split.
    """

    contract: ControlIntent
    span: Optional[Span]
    operands: tuple[BoundOperand, ...]
    predicate: Optional[BoundPredicate]

    @property
    def node_id(self) -> str:
        return self.contract.node_id

    @property
    def control_kind(self) -> str:
        return self.contract.control_kind

    @property
    def parent_region_id(self) -> str:
        return self.contract.parent_region_id

    @property
    def execution_order(self) -> int:
        return self.contract.execution_order

    @property
    def body_region_ids(self) -> tuple[str, ...]:
        return self.contract.body_region_ids or ()

    @property
    def result_value(self) -> Optional[str]:
        return self.contract.result_value


@dataclass(frozen=True, slots=True)
class BoundRegion:
    """One lexical region owning ordered children."""

    region_id: str
    region_role: str  # one of RegionRole in _generated.frontend_graph
    parent_region_id: Optional[str]
    execution_order: int


@dataclass(frozen=True, slots=True)
class BoundHook:
    """A static before/after Hook binding and its captured body region."""

    hook_id: str
    scope: str
    phase: str
    target_selector: str
    declaration_order: int
    handler_ref: str
    handler_digest: str
    input_type_ref: str
    output_type_ref: str
    return_mode: str
    body_region_id: str
    assigned_context_value_id: Optional[str] = None


@dataclass(frozen=True, slots=True)
class BoundContextEdge:
    """An explicit typed Context transition between two nodes."""

    from_node: str
    to_node: str
    context_type_ref: str
    value_id: str


@dataclass(frozen=True, slots=True)
class BoundCapabilityRequirement:
    """One authored Capability declaration.

    Declarations are held one per authored binding, never one per
    ``capability_ref``: the same capability declared as both a Tool and a plain
    Capability is two distinct requirements and both reach the FrontendGraph.
    """

    capability_ref: str
    tool_schema_present: bool
    requested_permission: Optional[Permission] = None


@dataclass(frozen=True, slots=True)
class BoundProgram:
    """The immutable bound tree for one authored Agent program."""

    program_id: str
    entrypoint: str
    input_type_ref: str
    output_type_ref: str
    has_default_context: bool
    context_type_ref: Optional[str]
    parameters: tuple[BoundParameter, ...]
    body_region_id: str
    declarations: tuple[BoundDeclaration, ...] = ()
    values: tuple[BoundValue, ...] = ()
    regions: tuple[BoundRegion, ...] = ()
    calls: tuple[BoundCall, ...] = ()
    controls: tuple[BoundControl, ...] = ()
    context_edges: tuple[BoundContextEdge, ...] = ()
    hooks: tuple[BoundHook, ...] = ()
    imported_programs: tuple[tuple[str, str, str, str], ...] = ()
    capability_requirements: tuple[BoundCapabilityRequirement, ...] = ()
    model_requirements: tuple[str, ...] = ()
    spans: tuple[tuple[str, Span, str], ...] = field(default=())


BoundNode = Union[BoundCall, BoundControl]
