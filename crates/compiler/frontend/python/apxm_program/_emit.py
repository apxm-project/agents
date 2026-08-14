"""Deterministic traversal from the bound tree to the FrontendGraph contract.

One visitor folds an immutable :class:`BoundProgram` into the language-neutral
``apxm.frontend-graph`` value. Every reachable bound node produces exactly one
graph record and every emitted value carries one typed origin. The visitor reads
the frozen tree only; it never executes authored code.

Per-record key layout is not written here: every ``serialize_*`` call below
projects one contract record through
:mod:`apxm_program._generated.frontend_serializers`, which holds each record's
required keys and then its stated optional ones in the contract's own order.
What stays here is the fold itself plus the two derivations the contract does
not state — :func:`_blocks` and the loop ``region_annotations``.
"""

from __future__ import annotations

from dataclasses import replace
from typing import Any, Optional

from ._bound_tree import BoundOperand, BoundProgram
from ._bridge import FRONTEND_GRAPH_VERSION, SOURCE_MAP_VERSION
from ._generated.frontend_graph import (
    CONTROL_KIND_LOOP,
    SOURCE_LANGUAGE_PYTHON,
    VALUE_ORIGIN_BLOCK_ARGUMENT,
    VALUE_ORIGIN_RESUME_INPUT,
)
from ._generated.frontend_records import (
    Block,
    DataEdge,
    FunctionDef,
    ImportedProgramRef,
    ModelRequirement,
    ProgramDefinition,
)
from ._generated.frontend_serializers import (
    serialize_block,
    serialize_call_intent,
    serialize_capability_requirement,
    serialize_context_edge,
    serialize_control_intent,
    serialize_data_edge,
    serialize_declaration,
    serialize_function_def,
    serialize_hook_binding,
    serialize_imported_program_ref,
    serialize_model_requirement,
    serialize_program_definition,
    serialize_region,
    serialize_skill_requirement,
    serialize_value,
)

#: The one source-map annotation this frontend derives. It is a source-map
#: term, not a FrontendGraph vocabulary member, so no generated set states it.
STRUCTURAL_LOOP = "structural_loop"

#: Every region owns exactly one lexical entry block, named after the region.
ENTRY_BLOCK_SUFFIX = ".block.0"


def emit_frontend_graph(
    program: BoundProgram, source_language: str = SOURCE_LANGUAGE_PYTHON
) -> dict[str, Any]:
    """Fold a bound program into the typed FrontendGraph contract value."""
    # An intent carries its operand value ids; the slot each operand fills is
    # carried by the data edge instead, so both are emitted from one walk.
    data_edges: list[dict[str, Any]] = []

    call_intents = []
    for call in program.calls:
        call_intents.append(
            serialize_call_intent(
                replace(call.contract, operand_values=_operand_values(call.operands))
            )
        )
        data_edges.extend(_data_edges(call.node_id, call.operands))

    control_intents = []
    for control in program.controls:
        control_intents.append(
            serialize_control_intent(
                replace(
                    control.contract,
                    operand_values=_operand_values(control.operands),
                    predicate=control.predicate,
                )
            )
        )
        data_edges.extend(_data_edges(control.node_id, control.operands))

    node_spans = [
        {
            "node_id": node_id,
            "source_file": span.source_file,
            "span": {
                "start_line": span.start_line,
                "start_column": span.start_column,
                "end_line": span.end_line,
                "end_column": span.end_column,
            },
            "semantic_annotation": annotation,
        }
        for node_id, span, annotation in program.spans
    ]
    # Frontend policy, not contract projection: a loop's first body region is
    # the one the compiler reads as the structural loop scope.
    region_annotations = [
        {"region_id": control.body_region_ids[0], "annotation": STRUCTURAL_LOOP}
        for control in program.controls
        if control.control_kind == CONTROL_KIND_LOOP and control.body_region_ids
    ]

    return {
        "schema_version": FRONTEND_GRAPH_VERSION,
        "source_language": source_language,
        "program_definitions": [
            serialize_program_definition(
                ProgramDefinition(
                    program_id=program.program_id,
                    entrypoint=program.entrypoint,
                    input_type_ref=program.input_type_ref,
                    output_type_ref=program.output_type_ref,
                    has_default_context=program.has_default_context,
                    context_type_ref=program.context_type_ref,
                )
            )
        ],
        "imported_program_refs": [
            serialize_imported_program_ref(
                ImportedProgramRef(
                    program_ref=program_ref,
                    artifact_digest=digest,
                    entrypoint=entrypoint,
                    target_agent_identity_requirement=identity,
                )
            )
            for (program_ref, digest, entrypoint, identity) in program.imported_programs
        ],
        "declarations": [
            serialize_declaration(declaration) for declaration in program.declarations
        ],
        "functions": [
            serialize_function_def(
                FunctionDef(
                    function_id=program.entrypoint,
                    parameters=program.parameters,
                    body_region_id=program.body_region_id,
                    is_entrypoint=True,
                    result_type_ref=program.output_type_ref,
                )
            )
        ],
        "values": [serialize_value(value) for value in program.values],
        "blocks": [serialize_block(block) for block in _blocks(program)],
        "regions": [serialize_region(region) for region in program.regions],
        "data_edges": data_edges,
        "call_intents": call_intents,
        "control_intents": control_intents,
        "context_flow": [serialize_context_edge(edge) for edge in program.context_edges],
        "hook_bindings": [serialize_hook_binding(hook) for hook in program.hooks],
        "capability_requirements": [
            serialize_capability_requirement(requirement)
            for requirement in program.capability_requirements
        ],
        "model_requirements": [
            serialize_model_requirement(ModelRequirement(model_target_ref=target_ref))
            for target_ref in program.model_requirements
        ],
        "skill_requirements": [
            serialize_skill_requirement(requirement)
            for requirement in program.skill_requirements
        ],
        "source_map": {
            "schema_version": SOURCE_MAP_VERSION,
            "source_language": source_language,
            "node_spans": node_spans,
            "region_annotations": region_annotations,
        },
    }


def _operand_values(operands: tuple[BoundOperand, ...]) -> Optional[tuple[str, ...]]:
    """The operand value ids an intent carries, or ``None`` when it takes none."""
    return tuple(operand.value_id for operand in operands) or None


def _data_edges(node_id: str, operands: tuple[BoundOperand, ...]) -> list[dict[str, Any]]:
    """One edge per operand, carrying the consumer slot the intent leaves out."""
    return [
        serialize_data_edge(
            DataEdge(
                from_value=operand.value_id,
                to_consumer=node_id,
                consumer_slot=operand.slot,
            )
        )
        for operand in operands
    ]


def _blocks(program: BoundProgram) -> list[Block]:
    """Emit one lexical entry block for every captured region.

    A yielded resume value is introduced at the lexical point where the yield
    resumes. Keeping that value in its enclosing region block makes the
    continuation boundary explicit to Rust lowering without changing the
    value's ``resume_input`` provenance. This is frontend policy: the contract
    states the block record, not which region a resume value belongs to.
    """
    controls = {control.node_id: control for control in program.controls}
    arguments_by_region: dict[str, list[str]] = {
        region.region_id: [] for region in program.regions
    }
    for value in program.values:
        if value.origin == VALUE_ORIGIN_RESUME_INPUT and value.origin_id is not None:
            control = controls.get(value.origin_id)
            if control is not None:
                arguments_by_region[control.parent_region_id].append(value.value_id)
        elif value.origin == VALUE_ORIGIN_BLOCK_ARGUMENT and value.origin_id is not None:
            region_id = value.origin_id.removesuffix(ENTRY_BLOCK_SUFFIX)
            if region_id in arguments_by_region:
                arguments_by_region[region_id].append(value.value_id)

    return [
        Block(
            block_id=f"{region.region_id}{ENTRY_BLOCK_SUFFIX}",
            region_id=region.region_id,
            block_arguments=tuple(arguments_by_region[region.region_id]),
            execution_order=0,
        )
        for region in program.regions
    ]
