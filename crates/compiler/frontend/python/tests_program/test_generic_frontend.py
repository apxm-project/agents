"""Generic Python AgentProgram recording without example-owned abstractions."""

from __future__ import annotations

from apxm_program import AgentProgram
from apxm_program._generated.runtime_evidence import (
    LoopIterationCompletedFact,
    decode_fact,
)


def test_agent_program_records_generic_effects_and_structured_loop() -> None:
    program = AgentProgram(
        program_id="generic",
        input_type_ref="Input",
        output_type_ref="Output",
        context_type_ref="Context",
    )
    program.loop("region.loop", lambda body: body.model_call("node.model", "model.default"))
    program.capability_invoke("node.capability", "cap.search")
    program.await_event("node.await", "event.input")
    program.return_region("region.return")

    graph = program.build_graph()
    assert [operation["op"] for operation in graph["semantic_operations"]] == [
        "model.call",
        "capability.invoke",
        "await.event",
    ]
    assert graph["semantic_operations"][0]["parent_region_id"] == "region.loop"
    assert graph["semantic_operations"][0]["execution_order"] == 0
    assert graph["semantic_operations"][1]["parent_region_id"] == "region.generic.body"
    assert graph["semantic_operations"][1]["execution_order"] == 1
    assert graph["structural_regions"] == [
        {
            "region_id": "region.generic.body",
            "kind": "region",
            "execution_order": 0,
        },
        {
            "region_id": "region.loop",
            "kind": "ais.loop",
            "parent_region_id": "region.generic.body",
            "execution_order": 0,
        },
        {
            "region_id": "region.return",
            "kind": "return",
            "parent_region_id": "region.generic.body",
            "execution_order": 3,
        },
    ]
    assert graph["source_map"]["region_annotations"] == [
        {"region_id": "region.loop", "annotation": "structural_loop"}
    ]


def test_agent_program_exposes_every_author_facing_structural_kind() -> None:
    program = AgentProgram(
        program_id="structured",
        input_type_ref="Input",
        output_type_ref="Output",
    )
    empty = lambda _body: None

    program.branch("region.branch", empty, empty)
    program.switch("region.switch", (empty, empty))
    program.loop("region.loop", empty)
    program.parallel("region.parallel", empty)
    program.try_catch("region.try", "region.catch", empty, empty)
    program.throw_region("region.throw")
    program.return_region("region.return")
    program.yield_region("region.yield")

    assert program.build_graph()["structural_regions"] == [
        {
            "region_id": "region.structured.body",
            "kind": "region",
            "execution_order": 0,
        },
        *[
            {
                "region_id": region_id,
                "kind": kind,
                "parent_region_id": "region.structured.body",
                "execution_order": execution_order,
            }
            for execution_order, (region_id, kind) in enumerate(
                [
                    ("region.branch", "branch"),
                    ("region.switch", "switch"),
                    ("region.loop", "ais.loop"),
                    ("region.parallel", "parallel_join"),
                    ("region.try", "try"),
                    ("region.catch", "catch"),
                    ("region.throw", "throw"),
                    ("region.return", "return"),
                    ("region.yield", "yield"),
                ]
            )
        ],
    ]
    assert not hasattr(program, "region")


def test_agent_program_records_nested_and_sibling_loop_containment() -> None:
    program = AgentProgram(
        program_id="nested",
        input_type_ref="Input",
        output_type_ref="Output",
    )

    def outer(body: AgentProgram) -> None:
        body.model_call("node.outer.before", "model.default")
        body.loop(
            "loop.inner",
            lambda inner: inner.capability_invoke("node.inner", "cap.inner"),
        )
        body.model_call("node.outer.after", "model.default")

    program.loop("loop.outer", outer)
    program.loop(
        "loop.sibling",
        lambda sibling: sibling.capability_invoke("node.sibling", "cap.sibling"),
    )

    graph = program.build_graph()
    by_region = {
        region["region_id"]: region for region in graph["structural_regions"]
    }
    by_node = {
        operation["node_id"]: operation
        for operation in graph["semantic_operations"]
    }
    assert by_region["loop.outer"]["parent_region_id"] == "region.nested.body"
    assert by_region["loop.outer"]["execution_order"] == 0
    assert by_region["loop.inner"]["parent_region_id"] == "loop.outer"
    assert by_region["loop.inner"]["execution_order"] == 1
    assert by_region["loop.sibling"]["parent_region_id"] == "region.nested.body"
    assert by_region["loop.sibling"]["execution_order"] == 1
    assert by_node["node.outer.before"]["parent_region_id"] == "loop.outer"
    assert by_node["node.outer.before"]["execution_order"] == 0
    assert by_node["node.inner"]["parent_region_id"] == "loop.inner"
    assert by_node["node.outer.after"]["execution_order"] == 2
    assert by_node["node.sibling"]["parent_region_id"] == "loop.sibling"
    assert {
        annotation["annotation"]
        for annotation in graph["source_map"]["region_annotations"]
    } == {"structural_loop"}


def test_generated_runtime_evidence_binding_is_closed() -> None:
    fact = decode_fact(
        {
            "fact_id": "loop.1",
            "event_sequence": 1,
            "fact_kind": "LoopIterationCompleted",
            "static_loop_id": "loop.main",
            "loop_occurrence_id": "occurrence.1",
            "iteration_index": 0,
            "program_invocation_id": "invocation.1",
            "causal_node_execution_ids": ["node-execution.1"],
        }
    )
    assert isinstance(fact, LoopIterationCompletedFact)
    try:
        decode_fact(
            {
                "fact_id": "bad.1",
                "event_sequence": 1,
                "fact_kind": "invented.fact",
            }
        )
    except ValueError:
        pass
    else:
        raise AssertionError("unknown fact kind must fail closed")

    runtime_input = {
        "fact_id": "runtime.1",
        "event_sequence": 2,
        "fact_kind": "invocation.committed",
        "commit_sequence": 1,
        "invocation_state": "committed_return",
        "context_before_ref": {
            "ref_type": "ContextRef",
            "ref": "context.before",
            "digest": f"sha256:{'a' * 64}",
        },
        "typed_error": {
            "error_id": "error.1",
            "category": "validation",
            "code_ref": "InvalidInput",
            "message": "invalid input",
        },
    }
    runtime = decode_fact(runtime_input)
    assert runtime.to_dict() == runtime_input

    malformed = [
        {**runtime_input, "event_sequence": "2"},
        {**runtime_input, "invocation_state": "invented"},
        {**runtime_input, "context_before_ref": {"ref_type": "ContextRef"}},
        {
            **runtime_input,
            "context_before_ref": {
                "ref_type": "ContextRef",
                "ref": "context.before",
                "extra": True,
            },
        },
        {
            **runtime_input,
            "context_before_ref": {
                "ref_type": "bad space",
                "ref": "context.before",
            },
        },
        {**runtime_input, "typed_error": {"error_id": "error.1"}},
        {
            **runtime_input,
            "typed_error": {
                "error_id": "error.1",
                "category": "invented",
                "code_ref": "InvalidInput",
                "message": "invalid",
            },
        },
        {
            **runtime_input,
            "typed_error": {
                "error_id": "error.1",
                "category": "validation",
                "code_ref": "InvalidInput",
                "message": "invalid",
                "extra": True,
            },
        },
        {
            "fact_id": "loop.bad",
            "event_sequence": 1,
            "fact_kind": "LoopIterationCompleted",
            "static_loop_id": "loop.main",
            "loop_occurrence_id": "occurrence.1",
            "iteration_index": 0,
            "program_invocation_id": "invocation.1",
            "causal_node_execution_ids": ["node.1", "node.1"],
        },
        {
            "fact_id": "node.bad",
            "event_sequence": 1,
            "fact_kind": "node_execution.recorded",
            "node_execution_id": "node.1",
            "air_node_id": "air.1",
            "execution_scope": {
                "scope_kind": "loop",
                "region_occurrence_id": "occurrence.1",
                "static_region_id": "loop.main",
                "loop_memberships": [{"static_loop_id": "loop.main"}],
            },
        },
        {**runtime_input, "unknown": True},
    ]
    for value in malformed:
        try:
            decode_fact(value)
        except ValueError:
            continue
        raise AssertionError(f"malformed fact accepted: {value}")
