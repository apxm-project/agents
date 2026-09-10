"""The Python frontend facts the shared conformance corpus cannot state.

Everything about authoring a program — what an Workflow binds, what the
FrontendGraph says, what the canonical AIR lowers to, and what authoring is
rejected — is stated once in `contracts/vectors/apxm.frontend-conformance.json`
and run from the generated harness in `test_frontend_conformance.py`. A fact
that only exists in this language stays here, because a corpus vector projected
into both languages could not state it:

* A bare typed marker factory (``Model[object]``) is a Python subscript
  expression that is not yet a declaration. TypeScript has no such intermediate
  value — `Model<Input, Output>` is not a value at all — so the fact has no
  TypeScript projection.
* The generated runtime-evidence binding decodes facts. It is a generated
  contract binding rather than an authored Workflow, so it has no source fixture to
  capture and no FrontendGraph to compare.
* A ``Skill`` stating both an entry and inline text is a rejection the corpus's
  ``declaration_rejections`` cannot express: that vocabulary states one
  reference per marker and calls the marker with it, so it can state the skill
  that names neither source — and does — but not the one that names two. The
  TypeScript frontend carries the same test for the same reason.
* A Hook whose target names nothing, and a Hook that names its target
  declaration rather than the selector for it. A Python Hook binds through the
  module globals its Workflow resolves, so both are facts about a whole module, not
  about one authored program, and a corpus vector authors its programs inside
  one shared module. The TypeScript frontend carries the same tests for the same
  reason.
"""

from __future__ import annotations

import pytest
from typing import Any, Awaitable, Literal, NotRequired, Required, TypedDict, get_args, get_origin, get_overloads, get_type_hints

from apxm_program import Agent, Workflow, Program, Capability, Context, Event, EventRef, Hook, Model, Skill, Tool
from apxm_program._workflow import InputT, OutputT, ContextT
from apxm_program._capture import CaptureError
from apxm_program._generated.capabilities import READ
from apxm_program._generated.diagnostics import (
    HOOK_DYNAMIC_REGISTRATION,
    HOOK_SCOPE_UNRESOLVED,
    HOOK_TARGET_UNRESOLVED,
    SKILL_ENTRY_PATH_NOT_CANONICAL,
    SKILL_SOURCE_AMBIGUOUS,
)
from apxm_program._generated.permissions import Permission
from apxm_program._generated.runtime_evidence import (
    LoopIterationCompletedFact,
    ModelAttemptRecordedFact,
    decode_fact,
)


def test_workflow_returns_one_sealed_typed_program() -> None:
    class Input(TypedDict):
        reference: str

    class Output(TypedDict):
        accepted: bool

    @Workflow(input=Input, output=Output)
    async def TypedWorkflow(agent, input):
        return {"accepted": True}

    assert get_type_hints(Program.invoke)["_input"] is InputT
    assert get_type_hints(Program.invoke)["return"] == Awaitable[OutputT]
    assert Program.__parameters__ == (InputT, OutputT, ContextT)
    declarations = get_overloads(Workflow)
    assert len(declarations) == 2
    default_handle = get_args(get_type_hints(declarations[0])["return"])[1]
    assert get_origin(default_handle) is Program
    assert get_args(default_handle) == (InputT, OutputT, type(None))
    assert get_type_hints(type(TypedWorkflow).invoke)["return"] == Awaitable[OutputT]
    with pytest.raises(AttributeError):
        type(TypedWorkflow).invoke = lambda *_: None
    with pytest.raises(AttributeError):
        TypedWorkflow._program_id = "changed"
    with pytest.raises(TypeError):
        Program()
    graph = TypedWorkflow.frontend_graph()
    graph["program_definitions"][0]["input_type_ref"] = "changed"
    assert TypedWorkflow.frontend_graph()["program_definitions"][0]["input_type_ref"] == "Input"


def test_context_marker_preserves_its_python_type_without_mutable_state() -> None:
    class State:
        count: int = 0

    declared = Context(State)
    assert declared.schema_type is State
    assert declared.type_ref == "State"
    assert declared.default_present is True
    with pytest.raises(AttributeError):
        declared.type_ref = "Changed"


def test_agent_requires_an_exact_model_that_its_body_invokes() -> None:
    primary = Model[object, object]("test.primary")
    other = Model[object, object]("test.other")

    with pytest.raises(TypeError, match="model"):
        Agent(input=object, output=object)
    with pytest.raises(TypeError, match="typed Model"):
        Agent(input=object, output=object, model="test.primary")
    with pytest.raises(TypeError, match="must invoke"):
        @Agent(input=object, output=object, model=primary)
        async def Unused(agent, input):
            return await other(input)

    @Agent(input=object, output=object, model=primary)
    async def ModelBacked(agent, input):
        return await primary(input)

    assert ModelBacked.frontend_graph()["program_definitions"][0]["authoring"] == {
        "kind": "agent", "primary_model_ref": "test.primary",
    }
    assert ModelBacked.diagnostics() is None
    assert get_type_hints(type(ModelBacked.new()).invoke)["return"] == Awaitable[OutputT]


def test_generated_union_branches_keep_exact_literal_discriminators() -> None:
    from apxm_program._generated.frontend_records import AgentAuthoring, WorkflowAuthoring, SsaExpression, TruthyPredicate

    assert get_type_hints(WorkflowAuthoring)["kind"] == Literal["workflow"]
    assert get_type_hints(AgentAuthoring)["kind"] == Literal["agent"]
    assert get_type_hints(SsaExpression)["kind"] == Literal["ssa"]
    assert get_type_hints(TruthyPredicate)["comparator"] == Literal["truthy"]


def test_a_bare_typed_marker_factory_is_not_yet_a_declaration() -> None:
    bare_typed_factory = Event[object]
    assert not hasattr(bare_typed_factory, "wait")

    event = bare_typed_factory("event.session.input")
    assert event.target_ref == "event.session.input"


class EventPayload(TypedDict):
    reference: str
    approved: bool


class EventInput(TypedDict):
    event: EventRef[EventPayload]


class WrongEventInput(TypedDict):
    event: EventRef[str]


class EquivalentPayload(TypedDict):
    reference: str
    approved: bool


class EquivalentEventInput(TypedDict):
    event: EventRef[EquivalentPayload]


SubmittedEvent = Event[EventPayload]("event.submitted")
EventRead = Capability[EventPayload, EventPayload](READ)


def test_event_reference_and_payload_schema_are_typed_without_source_authority() -> None:
    @Workflow(input=EventInput, output=EventPayload)
    async def EventWorkflow(agent, input):
        result = await SubmittedEvent.wait(input["event"])
        return await EventRead(result)

    graph = EventWorkflow.frontend_graph()
    declaration = next(value for value in graph["declarations"] if value["decl_kind"] == "event_type")
    assert declaration["payload_schema"] == {
        "type": "object", "properties": {"reference": {"type": "string"}, "approved": {"type": "boolean"}},
        "required": ["reference", "approved"], "additionalProperties": False,
    }
    assert graph["program_definitions"][0]["input_schema"]["properties"]["event"] == {
        "type": "object", "properties": {"event_id": {"type": "string"}, "generation": {"type": "integer"}},
        "required": ["event_id", "generation"], "additionalProperties": False,
    }
    wait = next(call for call in graph["call_intents"] if call["intent_kind"] == "event_wait")
    assert len(wait["operand_values"]) == 1
    assert next(value for value in graph["values"] if value["value_id"] == wait["operand_values"][0])["type_ref"] == "EventRef"
    with pytest.raises(TypeError, match="admitted input"):
        EventRef()


def test_event_wait_refuses_missing_literal_and_wrong_payload_references() -> None:
    with pytest.raises(CaptureError, match="EventRef"):
        @Workflow(input=EventInput, output=EventPayload)
        async def Missing(agent, input):
            return await SubmittedEvent.wait()
    with pytest.raises(CaptureError, match="EventRef"):
        @Workflow(input=EventInput, output=EventPayload)
        async def Literal(agent, input):
            return await SubmittedEvent.wait("event.submitted")
    with pytest.raises(CaptureError, match="EventRef"):
        @Workflow(input=WrongEventInput, output=EventPayload)
        async def Wrong(agent, input):
            return await SubmittedEvent.wait(input["event"])


def test_event_payload_names_are_nominal_but_closed_schemas_must_agree() -> None:
    @Workflow(input=EquivalentEventInput, output=EventPayload)
    async def Equivalent(agent, input):
        return await SubmittedEvent.wait(input["event"])

    assert Equivalent.diagnostics() is None


def test_event_payload_requires_supported_finite_json_type() -> None:
    UnsupportedEvent = Event[object]("event.unsupported")
    with pytest.raises(CaptureError, match="finite JSON type"):
        @Workflow(input=EventInput, output=EventPayload)
        async def Unsupported(agent, input):
            return await UnsupportedEvent.wait(input["event"])


class _Payload:
    """A typed declaration this module's fixtures state an interface with."""


class _OptionalDetails(TypedDict, total=False):
    note: str
    enabled: bool


class _JsonInput(TypedDict):
    reference: str
    count: float
    retries: int
    accepted: bool
    absent: None
    labels: list[str]
    details: NotRequired[_OptionalDetails]


class _RecursiveInput(TypedDict):
    child: NotRequired[_RecursiveInput]


class _RequiredOverride(TypedDict, total=False):
    reference: Required[str]


class _NullableInput(TypedDict):
    reference: str | None


def test_typed_json_input_reaches_the_closed_frontend_schema() -> None:
    @Workflow(input=_JsonInput, output=_JsonInput)
    async def JsonInput(agent, input):
        return input

    assert JsonInput.frontend_graph()["program_definitions"][0]["input_schema"] == {
        "type": "object", "additionalProperties": False,
        "required": ["reference", "count", "retries", "accepted", "absent", "labels"],
        "properties": {
            "reference": {"type": "string"}, "count": {"type": "number"},
            "retries": {"type": "integer"}, "accepted": {"type": "boolean"},
            "absent": {"type": "null"}, "labels": {"type": "array", "items": {"type": "string"}},
            "details": {"type": "object", "additionalProperties": False, "required": [],
                        "properties": {"note": {"type": "string"}, "enabled": {"type": "boolean"}}},
        },
    }


def test_required_field_override_remains_required() -> None:
    @Workflow(input=_RequiredOverride, output=_RequiredOverride)
    async def RequiredOverride(agent, input):
        return input

    schema = RequiredOverride.frontend_graph()["program_definitions"][0]["input_schema"]
    assert schema["required"] == ["reference"]


@pytest.mark.parametrize("input_type", [Any, object, _Payload, _RecursiveInput, dict[str, str], tuple[str], Literal["fixed"], _NullableInput])
def test_unsupported_python_input_has_no_widened_schema(input_type) -> None:
    @Workflow(input=input_type, output=_Payload)
    async def UnsupportedInput(agent, input):
        return input

    assert "input_schema" not in UnsupportedInput.frontend_graph()["program_definitions"][0]


_StrayModel = Model[_Payload, _Payload]("stray.hook.model")


async def _stray_handler(agent) -> None:
    return None


def test_a_hook_target_no_declaration_names_is_refused() -> None:
    """A typo'd Hook target is refused rather than dropped.

    The Hook has to live in module globals for the capture to resolve it at all,
    and it is removed again afterwards: a Hook left in scope binds into every
    later capture in this module, which is what the generated corpus harness
    does for the same reason.
    """
    global _StrayHook
    _StrayHook = Hook.before(target="NoDeclarationNamesThis", scope="model")(
        _stray_handler
    )
    try:
        with pytest.raises(CaptureError, match=HOOK_TARGET_UNRESOLVED) as error:

            @Workflow(input=_Payload, output=_Payload)
            async def StrayHookTarget(agent, input):
                return await _StrayModel(input)

        assert error.value.code == HOOK_TARGET_UNRESOLVED

    finally:
        del _StrayHook


def test_a_hook_names_its_target_declaration_or_the_selector_for_it() -> None:
    """``target=_StrayModel`` and ``target="_StrayModel"`` are one statement.

    TypeScript has always taken the declaration; Python took only the selector,
    so the same Hook had to be written two ways. Both resolve to the same bound
    target here, which is what makes them one authoring shape rather than two.
    """
    global _StrayHook
    captured = []
    for target in (_StrayModel, "_StrayModel"):
        _StrayHook = Hook.before(target=target, scope="model")(_stray_handler)
        try:

            @Workflow(input=_Payload, output=_Payload)
            async def HookTargetShape(agent, input):
                return await _StrayModel(input)

            captured.append(
                HookTargetShape.frontend_graph()["hook_bindings"][0]["target_selector"]
            )
        finally:
            del _StrayHook
    assert captured[0] == captured[1]


def test_a_hook_target_that_is_neither_declaration_nor_selector_is_refused() -> None:
    with pytest.raises(TypeError, match=HOOK_DYNAMIC_REGISTRATION):
        Hook.before(target=object(), scope="model")


def test_a_hook_scope_outside_the_vocabulary_is_refused() -> None:
    with pytest.raises(ValueError, match=HOOK_SCOPE_UNRESOLVED):
        Hook.before(target=_StrayModel, scope="capabilities")


def test_a_skill_states_one_instruction_source() -> None:
    with pytest.raises(ValueError, match=SKILL_SOURCE_AMBIGUOUS):
        Skill("review", entry="skills/review/SKILL.md", text="Review carefully.")


def test_a_file_carried_skill_names_the_path_its_id_resolves_to() -> None:
    with pytest.raises(ValueError, match=SKILL_ENTRY_PATH_NOT_CANONICAL):
        Skill("review", entry="prompts/review.md")


def test_a_capability_permission_is_closed_before_graph_capture() -> None:
    with pytest.raises(TypeError, match="generated Allow, Ask, or Deny"):
        Tool[object, object](READ, permission="allow")

    with pytest.raises(ValueError, match="decision must be one of"):
        Tool[object, object](READ, permission=Permission("request", None))

    with pytest.raises(ValueError, match="reason must be a non-empty string"):
        Tool[object, object](READ, permission=Permission("ask", ""))


def test_only_the_declared_host_capabilities_are_minted() -> None:
    """The manifest is not in this process, so the trusted harness declares it.

    The declaration is a private module rather than an authoring import: a
    program that could declare its own host capabilities would be minting its
    own authority.
    """
    from apxm_program import _host_capabilities

    _host_capabilities.set_declared_host_capabilities([])
    try:
        with pytest.raises(ValueError, match="This package declares none"):
            Capability[object, object]("host:notes.search")

        _host_capabilities.set_declared_host_capabilities(["notes.search"])
        binding = Capability[object, object]("host:notes.search")
        assert binding.target_ref == "host:notes.search"

        with pytest.raises(ValueError, match="host:notes.search"):
            Capability[object, object]("host:notes.append")
    finally:
        _host_capabilities.set_declared_host_capabilities([])


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


def test_generated_runtime_evidence_preserves_specialized_attempt_identity() -> None:
    fact = decode_fact(
        {
            "fact_id": "attempt.1",
            "event_sequence": 2,
            "fact_kind": "attempt.recorded",
            "program_invocation_id": "invocation.1",
            "node_execution_id": "node-execution.1",
            "air_node_id": "node.model",
            "attempt_id": "attempt.1",
            "attempt_index": 0,
            "model_effect_id": "effect.1",
            "request_digest": "sha256:" + "1" * 64,
            "model_target_ref": "model-target.1",
            "model_target_digest": "sha256:" + "2" * 64,
            "model_deployment_ref": "deployment.1",
            "exact_port_binding_digest": "sha256:" + "3" * 64,
            "target_commitment_digest": "sha256:" + "4" * 64,
            "generation_cohort_digest": "sha256:" + "5" * 64,
            "target_generation": 1,
            "target_port_contract_digest": "sha256:" + "6" * 64,
            "target_composition_digest": "sha256:" + "7" * 64,
            "native_input_tokens": 3,
            "native_output_tokens": 5,
        }
    )
    assert isinstance(fact, ModelAttemptRecordedFact)
    assert fact.attempt_id == "attempt.1"
    assert fact.native_output_tokens == 5

    with pytest.raises(ValueError, match="expected exactly one oneOf branch"):
        decode_fact(
            {
                "fact_id": "attempt.2",
                "event_sequence": 3,
                "fact_kind": "attempt.recorded",
            }
        )
