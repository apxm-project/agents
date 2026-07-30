"""Source-first Python authoring produces the typed FrontendGraph."""

from __future__ import annotations

import json

from apxm_program import Agent, Capability, Context, Event, Hook, Model, TaskGroup, Tool
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


@Context
class ReviewContext:
    completed: bool = False


ReviewModel = Model[object, object]("review.model.v1")
Approval = Event[object]("approval.event.v1")


@Agent(input="ReviewRequest", output="Review", context=ReviewContext)
async def Specialist(agent, request):
    return await ReviewModel(request)


@Hook.before(target="ReviewModel", scope="model")
async def RecordModelStart(agent) -> None:
    return None


@Agent(input="ReviewRequest", output="Review", context=ReviewContext)
async def Coordinator(agent, request):
    while True:
        specialist = Specialist.new(context=ReviewContext())
        review = await specialist.invoke(request)
        approved = await Approval.wait()
        async with TaskGroup():
            result = await ReviewModel(review)
        agent.context = ReviewContext(completed=True)
        request = await agent.yield_(result)


def test_minimal_one_shot_agent_binds_model_and_verifies() -> None:
    graph = Summarizer.frontend_graph()
    assert graph["schema_version"] == "apxm.frontend-graph.v1"
    assert graph["program_definitions"][0]["program_id"] == "Summarizer"
    assert [d["decl_kind"] for d in graph["declarations"]] == ["model_binding"]
    assert [c["intent_kind"] for c in graph["call_intents"]] == ["model_invocation"]
    assert graph["model_requirements"] == [{"model_target_ref": "summarizer.model.v1"}]
    assert all(
        not span["source_file"].startswith("/")
        and "/home/" not in span["source_file"]
        for span in graph["source_map"]["node_spans"]
    )
    assert Summarizer.diagnostics() is None


def test_minimal_agent_lowers_to_registered_model_call() -> None:
    air = json.loads(Summarizer.canonical_air())
    assert [op["op"] for op in air["semantic_operations"]] == ["model.call"]
    request_slots = [
        operand["slot"] for operand in air["semantic_operations"][0]["operands"]
    ]
    assert "request" in request_slots
    assert {
        operand["slot"] for operand in air["semantic_operations"][0]["operands"]
    } >= {"model_ref", "request"}
    assert next(
        operand["value_id"]
        for operand in air["semantic_operations"][0]["operands"]
        if operand["slot"] == "model_ref"
    ) == "summarizer.model.v1"


def test_static_bindings_reject_display_names_callables_and_bare_event_factories() -> None:
    for marker in (Model, Tool, Capability, Event):
        for invalid in ("", "default", "model.default", "support", "search-web"):
            try:
                marker(invalid)
            except ValueError:
                pass
            else:
                raise AssertionError(f"{marker!r} accepted forbidden reference {invalid!r}")

    for marker in (Tool, Capability):
        try:
            marker(lambda: None)
        except ValueError:
            pass
        else:
            raise AssertionError(f"{marker!r} accepted a package-local handler")

    bare_typed_factory = Event[object]
    assert not hasattr(bare_typed_factory, "wait")
    event = bare_typed_factory("event.session.input.v1")
    assert event.target_ref == "event.session.input.v1"


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
        "AgentDefinition",
        "ProgramInstance",
        "FIVE_OPS",
        "OP_MODEL_CALL",
        "canonical_air_json",
        "lower",
        "verify",
    ):
        assert not hasattr(apxm_program, forbidden), forbidden


def test_advanced_constructs_bind_without_executing_the_agent_body() -> None:
    graph = Coordinator.frontend_graph()

    assert graph["imported_program_refs"] == [
        {
            "program_ref": "Specialist",
            "artifact_digest": Specialist._artifact_digest,
            "entrypoint": "Specialist",
            "target_agent_identity_requirement": "Specialist.identity",
        }
    ]
    assert {call["intent_kind"] for call in graph["call_intents"]} == {
        "agent_creation",
        "agent_invocation",
        "event_wait",
        "model_invocation",
    }
    assert {
        control["control_kind"] for control in graph["control_intents"]
    } >= {"loop", "task_group", "yield"}
    assert any(region["region_role"] == "task_scope" for region in graph["regions"])

    event_wait = next(
        call for call in graph["call_intents"] if call["intent_kind"] == "event_wait"
    )
    event_declaration = next(
        declaration
        for declaration in graph["declarations"]
        if declaration["decl_kind"] == "event_type"
    )
    yield_control = next(
        control for control in graph["control_intents"] if control["control_kind"] == "yield"
    )
    task_group = next(
        control for control in graph["control_intents"] if control["control_kind"] == "task_group"
    )
    assert _value(graph, event_wait["result_value"])["origin"] == "call_result"
    assert _value(graph, yield_control["result_value"])["origin"] == "resume_input"
    assert event_declaration["target_ref"] == "approval.event.v1"
    assert graph["context_flow"] == [
        {
            "from_node": task_group["node_id"],
            "to_node": yield_control["node_id"],
            "context_type_ref": "ReviewContext",
        }
    ]
    assert {block["region_id"] for block in graph["blocks"]} == {
        region["region_id"] for region in graph["regions"]
    }
    loop_region = next(
        region["region_id"]
        for region in graph["regions"]
        if region["region_role"] == "loop_body"
    )
    resume_block = next(
        block for block in graph["blocks"] if block["region_id"] == loop_region
    )
    assert resume_block["block_arguments"] == [yield_control["result_value"]]
    assert _value(graph, yield_control["result_value"])["origin"] == "resume_input"

    air = json.loads(Coordinator.canonical_air())
    resume_definitions = [
        argument["value_id"]
        for structural in air["structural_ir"]
        for argument in structural.get("block_arguments", [])
        if argument["value_id"] == yield_control["result_value"]
    ]
    assert resume_definitions == [yield_control["result_value"]]


def test_hook_decorator_resolves_a_static_call_target() -> None:
    graph = Coordinator.frontend_graph()
    hook = graph["hook_bindings"][0]
    model_call = next(
        call for call in graph["call_intents"] if call["intent_kind"] == "model_invocation"
    )

    assert hook["target_selector"] == model_call["node_id"]
    assert hook["handler_ref"] == "RecordModelStart"
    assert hook["handler_digest"].startswith("sha256:")
    assert hook["output_type_ref"] == "Unit"
    assert Coordinator.diagnostics() is None


def test_value_position_rejects_an_unresolved_or_effectful_call() -> None:
    ValuePositionModel = Model[object, object]("value.position.model.v1")

    # A returned call the frontend cannot resolve was previously dropped, so
    # the graph silently lost the author's whole result expression.
    try:

        @Agent(input="Input", output="Output")
        async def ReturnsUnresolved(agent, request):
            return Unresolved(request)  # noqa: F821 - unresolved on purpose

    except ValueError as error:
        assert "call target 'Unresolved' is not a bound Context schema" in str(error)
    else:
        raise AssertionError("an unresolved returned call must not capture")

    # An unresolved call nested in an operand was previously folded into an
    # untyped literal operand, hiding it from lowering and from admission.
    try:

        @Agent(input="Input", output="Output")
        async def OperandUnresolved(agent, request):
            return await ValuePositionModel(Unresolved(request))  # noqa: F821

    except ValueError as error:
        assert "call target 'Unresolved' is not a bound Context schema" in str(error)
    else:
        raise AssertionError("an unresolved operand call must not capture")

    # A typed effect is an effect wherever it is written: reaching it without
    # await must not degrade it to data.
    try:

        @Agent(input="Input", output="Output")
        async def EffectAsValue(agent, request):
            return ValuePositionModel(request)

    except ValueError as error:
        assert "is a typed effect and is called with await" in str(error)
    else:
        raise AssertionError("an un-awaited typed effect must not capture as data")


def _value(graph: dict, value_id: str) -> dict:
    """Find one emitted FrontendGraph value by its stable identifier."""
    return next(value for value in graph["values"] if value["value_id"] == value_id)


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
