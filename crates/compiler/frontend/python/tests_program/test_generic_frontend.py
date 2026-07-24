"""Source-first Python authoring produces the typed FrontendGraph."""

from __future__ import annotations

import json

from apxm_program import Agent, Capability, Context, Event, Model, Tool
from apxm_program._generated.runtime_evidence import (
    LoopIterationCompletedFact,
    decode_fact,
)

SummarizerModel = Model[object, object]("summarizer.model.v1")


@Agent(input="SummaryRequest", output="Summary")
async def Summarizer(agent, request):
    return await SummarizerModel(request)


@Context
class Conversation:
    messages: tuple = ()


SearchWeb = Tool[object, object]("search.web.capability.v1")
SupportModel = Model[object, object]("support.model.v1")


@Agent(input="ConversationInput", output="ConversationOutput", context=Conversation)
async def Support(agent, incoming):
    while True:
        research = None
        if incoming is not None:
            research = await SearchWeb(incoming)
        response = await SupportModel(incoming)
        agent.context = Conversation()
        return response


def test_minimal_one_shot_agent_binds_model_and_verifies() -> None:
    graph = Summarizer.frontend_graph()
    assert graph["schema_version"] == "apxm.frontend-graph.v1"
    assert graph["program_definitions"][0]["program_id"] == "Summarizer"
    assert [d["decl_kind"] for d in graph["declarations"]] == ["model_binding"]
    assert [c["intent_kind"] for c in graph["call_intents"]] == ["model_invocation"]
    assert graph["model_requirements"] == [{"model_target_ref": "summarizer.model.v1"}]
    assert Summarizer.diagnostics() is None


def test_minimal_agent_lowers_to_registered_model_call() -> None:
    air = json.loads(Summarizer.canonical_air())
    assert [op["op"] for op in air["semantic_operations"]] == ["model.call"]
    request_slots = [
        operand["slot"] for operand in air["semantic_operations"][0]["operands"]
    ]
    assert "request" in request_slots


def test_contextual_agent_binds_context_tool_and_loop() -> None:
    graph = Support.frontend_graph()
    assert {d["decl_kind"] for d in graph["declarations"]} == {
        "context",
        "tool_binding",
        "model_binding",
    }
    assert [c["intent_kind"] for c in graph["call_intents"]] == [
        "tool_invocation",
        "model_invocation",
    ]
    assert {c["control_kind"] for c in graph["control_intents"]} == {
        "loop",
        "conditional",
        "return",
    }
    assert graph["capability_requirements"] == [
        {"capability_ref": "search.web.capability.v1", "tool_schema_present": True}
    ]
    assert Support.diagnostics() is None


def test_contextual_agent_tool_lowers_to_capability_invoke_and_ais_loop() -> None:
    air = json.loads(Support.canonical_air())
    ops = [op["op"] for op in air["semantic_operations"]]
    assert "capability.invoke" in ops
    assert "model.call" in ops
    structural = [node["kind"] for node in air["structural_ir"]]
    assert "ais.loop" in structural
    assert "branch" in structural


def test_authoring_surface_exposes_no_recorder_or_operation_constants() -> None:
    import apxm_program

    exported = set(apxm_program.__all__)
    assert exported == {
        "Agent",
        "Capability",
        "Context",
        "Event",
        "Hook",
        "Model",
        "TaskGroup",
        "Tool",
    }
    for forbidden in (
        "AgentProgram",
        "AgentFacade",
        "FIVE_OPS",
        "OP_MODEL_CALL",
        "canonical_air_json",
        "lower",
        "verify",
    ):
        assert not hasattr(apxm_program, forbidden), forbidden


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
