"""Deterministic traversal from the bound tree to the FrontendGraph contract.

One visitor folds an immutable :class:`BoundProgram` into the language-neutral
``apxm.frontend-graph.v1`` value. Every reachable bound node produces exactly one
graph record and every emitted value carries one typed origin. The visitor reads
the frozen tree only; it never executes authored code.
"""

from __future__ import annotations

from typing import Any

from ._bound_tree import BoundProgram
from ._bridge import FRONTEND_GRAPH_VERSION, SOURCE_MAP_VERSION


def emit_frontend_graph(program: BoundProgram, source_language: str = "python") -> dict[str, Any]:
    """Fold a bound program into the typed FrontendGraph contract value."""
    definition: dict[str, Any] = {
        "program_id": program.program_id,
        "entrypoint": program.entrypoint,
        "input_type_ref": program.input_type_ref,
        "output_type_ref": program.output_type_ref,
        "has_default_context": program.has_default_context,
    }
    if program.context_type_ref is not None:
        definition["context_type_ref"] = program.context_type_ref

    functions = [
        {
            "function_id": program.entrypoint,
            "parameters": [
                {"value_id": p.value_id, "type_ref": p.type_ref, "role": p.role}
                for p in program.parameters
            ],
            "result_type_ref": program.output_type_ref,
            "body_region_id": program.body_region_id,
            "is_entrypoint": True,
        }
    ]

    declarations = [_declaration(d) for d in program.declarations]
    values = [_value(v) for v in program.values]
    regions = [_region(r) for r in program.regions]
    blocks = _blocks(program)

    call_intents = []
    data_edges: list[dict[str, Any]] = []
    for call in program.calls:
        record: dict[str, Any] = {
            "node_id": call.node_id,
            "intent_kind": call.intent_kind,
            "parent_region_id": call.parent_region_id,
            "execution_order": call.execution_order,
        }
        if call.binding_ref is not None:
            record["binding_ref"] = call.binding_ref
        if call.receiver_kind is not None:
            record["receiver_kind"] = call.receiver_kind
        if call.operands:
            record["operand_values"] = [o.value_id for o in call.operands]
            for operand in call.operands:
                data_edges.append(
                    {
                        "from_value": operand.value_id,
                        "to_consumer": call.node_id,
                        "consumer_slot": operand.slot,
                    }
                )
        if call.result_value is not None:
            record["result_value"] = call.result_value
        call_intents.append(record)

    control_intents = []
    for control in program.controls:
        record = {
            "node_id": control.node_id,
            "control_kind": control.control_kind,
            "parent_region_id": control.parent_region_id,
            "execution_order": control.execution_order,
        }
        if control.body_region_ids:
            record["body_region_ids"] = list(control.body_region_ids)
        if control.operands:
            record["operand_values"] = [o.value_id for o in control.operands]
            for operand in control.operands:
                data_edges.append(
                    {
                        "from_value": operand.value_id,
                        "to_consumer": control.node_id,
                        "consumer_slot": operand.slot,
                    }
                )
        if control.result_value is not None:
            record["result_value"] = control.result_value
        control_intents.append(record)

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
    region_annotations = [
        {"region_id": control.body_region_ids[0], "annotation": "structural_loop"}
        for control in program.controls
        if control.control_kind == "loop" and control.body_region_ids
    ]

    return {
        "schema_version": FRONTEND_GRAPH_VERSION,
        "source_language": source_language,
        "program_definitions": [definition],
        "imported_program_refs": [
            {
                "program_ref": ref,
                "artifact_digest": digest,
                "entrypoint": entry,
                "target_agent_identity_requirement": identity,
            }
            for (ref, digest, entry, identity) in program.imported_programs
        ],
        "declarations": declarations,
        "functions": functions,
        "values": values,
        "blocks": blocks,
        "regions": regions,
        "data_edges": data_edges,
        "call_intents": call_intents,
        "control_intents": control_intents,
        "context_flow": [
            {
                "from_node": edge.from_node,
                "to_node": edge.to_node,
                "context_type_ref": edge.context_type_ref,
            }
            for edge in program.context_edges
        ],
        "hook_bindings": [_hook(h) for h in program.hooks],
        "capability_requirements": [
            {"capability_ref": ref, "tool_schema_present": tool}
            for (ref, tool) in program.capability_requirements
        ],
        "model_requirements": [
            {"model_target_ref": ref} for ref in program.model_requirements
        ],
        "source_map": {
            "schema_version": SOURCE_MAP_VERSION,
            "source_language": source_language,
            "node_spans": node_spans,
            "region_annotations": region_annotations,
        },
    }


def _declaration(decl: Any) -> dict[str, Any]:
    record: dict[str, Any] = {
        "decl_id": decl.decl_id,
        "decl_kind": decl.decl_kind,
        "input_type_ref": decl.input_type_ref,
        "output_type_ref": decl.output_type_ref,
    }
    if decl.target_ref is not None:
        record["target_ref"] = decl.target_ref
    if decl.context_default_present is not None:
        record["context_default_present"] = decl.context_default_present
    return record


def _value(value: Any) -> dict[str, Any]:
    record: dict[str, Any] = {
        "value_id": value.value_id,
        "type_ref": value.type_ref,
        "origin": value.origin,
    }
    if value.origin_id is not None:
        record["origin_id"] = value.origin_id
    return record


def _region(region: Any) -> dict[str, Any]:
    record: dict[str, Any] = {
        "region_id": region.region_id,
        "region_role": region.region_role,
        "execution_order": region.execution_order,
    }
    if region.parent_region_id is not None:
        record["parent_region_id"] = region.parent_region_id
    return record


def _blocks(program: BoundProgram) -> list[dict[str, Any]]:
    """Emit one lexical entry block for every captured region.

    A yielded resume value is introduced at the lexical point where the yield
    resumes. Keeping that value in its enclosing region block makes the
    continuation boundary explicit to Rust lowering without changing the
    value's ``resume_input`` provenance.
    """
    controls = {control.node_id: control for control in program.controls}
    arguments_by_region: dict[str, list[str]] = {
        region.region_id: [] for region in program.regions
    }
    for value in program.values:
        if value.origin != "resume_input" or value.origin_id is None:
            continue
        control = controls.get(value.origin_id)
        if control is not None:
            arguments_by_region[control.parent_region_id].append(value.value_id)

    return [
        {
            "block_id": f"{region.region_id}.block.0",
            "region_id": region.region_id,
            "block_arguments": arguments_by_region[region.region_id],
            "execution_order": 0,
        }
        for region in program.regions
    ]


def _hook(hook: Any) -> dict[str, Any]:
    return {
        "hook_id": hook.hook_id,
        "scope": hook.scope,
        "phase": hook.phase,
        "target_selector": hook.target_selector,
        "declaration_order": hook.declaration_order,
        "handler_ref": hook.handler_ref,
        "handler_digest": hook.handler_digest,
        "input_type_ref": hook.input_type_ref,
        "output_type_ref": hook.output_type_ref,
        "return_mode": hook.return_mode,
    }
