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
    """A resolved effectful call: model, tool, capability, agent, or event."""

    node_id: str
    intent_kind: str  # model_invocation | tool_invocation | capability_invocation | agent_creation | agent_invocation | event_wait
    parent_region_id: str
    execution_order: int
    binding_ref: Optional[str]
    operands: tuple[BoundOperand, ...]
    result_value: Optional[str]
    span: Optional[Span]
    receiver_kind: Optional[str] = None  # program_ref | program_instance_ref


@dataclass(frozen=True, slots=True)
class BoundControl:
    """A resolved structural construct: branch, loop, task group, try, yield, return."""

    node_id: str
    control_kind: str  # conditional | switch | loop | task_group | try_catch | throw | yield | return
    parent_region_id: str
    execution_order: int
    body_region_ids: tuple[str, ...]
    predicate: Optional[BoundPredicate]
    operands: tuple[BoundOperand, ...]
    result_value: Optional[str]
    span: Optional[Span]


@dataclass(frozen=True, slots=True)
class BoundRegion:
    """One lexical region owning ordered children."""

    region_id: str
    region_role: str  # function_body | conditional_arm | loop_body | task_scope | task_child | try_body | catch_body
    parent_region_id: Optional[str]
    execution_order: int


@dataclass(frozen=True, slots=True)
class BoundHook:
    """A static before/after Hook binding."""

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


@dataclass(frozen=True, slots=True)
class BoundContextEdge:
    """An explicit typed Context transition between two nodes."""

    from_node: str
    to_node: str
    context_type_ref: str


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
    capability_requirements: tuple[tuple[str, bool], ...] = ()
    model_requirements: tuple[str, ...] = ()
    spans: tuple[tuple[str, Span, str], ...] = field(default=())


BoundNode = Union[BoundCall, BoundControl]
