"""Native Python frontend DTO and Rust AIR-emission parity."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Callable

import pytest

from apxm.constants import PromptInputRole
from apxm.ir import (
    AirEmissionError,
    AirEmitterCommand,
    ApxmGraph,
    emit_multi_flow_module,
)
from apxm.proxy import GraphRecorder, prompt_input

_FIXTURES_DIR = (
    Path(__file__).resolve().parents[4] / "tools" / "cli" / "tests" / "fixtures" / "frontend_graph_parity"
)

def _load_fixture(name: str) -> dict:
    return json.loads((_FIXTURES_DIR / name).read_text())


def _load_golden(name: str) -> str:
    return (_FIXTURES_DIR / name).read_text()


def _ask_flow() -> ApxmGraph:
    recorder = GraphRecorder("ask_flow", metadata={"is_entry": True})
    recorder.param("name", "str")
    answer = recorder.ask(name="ask", prompt="Say hi to {name}")
    recorder.done(answer, name="out")
    return recorder.to_graph()


def _prompt_role_flow() -> ApxmGraph:
    recorder = GraphRecorder("prompt_role_flow", metadata={"is_entry": True})
    question = recorder.ask(name="question", prompt="Question")
    policy = recorder.ask(name="policy", prompt="System instructions")
    dependency = recorder.ask(name="dependency", prompt="Dependency state")
    tool_result = recorder.ask(name="tool_result", prompt="Tool result")
    guard = recorder.ask(name="guard", prompt="Control state")
    answer = recorder.ask(
        name="answer",
        prompt="Answer {question}",
        prompt_inputs={
            "policy": prompt_input(policy, PromptInputRole.SYSTEM),
            "dependency": prompt_input(dependency, PromptInputRole.DEPENDENCY_ONLY),
            "tool_result": prompt_input(tool_result, PromptInputRole.TOOL_CONTEXT),
            "guard": prompt_input(guard, PromptInputRole.CONTROL),
        },
    )
    recorder.done(answer, name="out")
    return recorder.to_graph()


def _parametrized_flow() -> ApxmGraph:
    recorder = GraphRecorder("parametrized_flow", metadata={"is_entry": True})
    recorder.param("topic", "str")
    recorder.param("style", "str")
    answer = recorder.ask(
        name="ask",
        prompt="Research {topic} in the style of {style}",
        token_budget=256,
    )
    recorder.done(answer, name="out")
    return recorder.to_graph()


def _profiled_agent_flow() -> ApxmGraph:
    recorder = GraphRecorder("agent_flow", metadata={"is_entry": True})
    agent = recorder.agent(
        name="coder",
        profile="codex",
        prompt="Fix it",
        cwd="/tmp/work",
    )
    recorder.done(agent, name="out")
    return recorder.to_graph()


def _multi_flow_conversational() -> list[ApxmGraph]:
    main = GraphRecorder("main", metadata={"is_entry": True})
    run_turn = main.flow_call(
        name="run_turn",
        agent_name="conversation",
        flow_name="turn",
    )
    main.done(run_turn, name="return_turn")

    turn = GraphRecorder("conversation.turn", metadata={"is_entry": False})
    turn.param("user_message", "str")
    answer = turn.ask(name="ask", prompt="Reply to: {user_message}")
    turn.done(answer, name="out")
    return [main.to_graph(), turn.to_graph()]


def _multi_flow_conversational_air() -> str:
    return emit_multi_flow_module(_multi_flow_conversational())


def _reasoning_flow() -> ApxmGraph:
    recorder = GraphRecorder("reasoning_flow", metadata={"is_entry": True})
    recorder.plan(name="plan", goal="Create a complete implementation plan")
    recorder.reflect(name="reflect", trace_query="most_recent_execution")
    verification = recorder.verify(
        name="verify",
        claim="The implementation plan is complete.",
        evidence="The review lists every required frontend contract.",
    )
    recorder.done(verification, name="out")
    return recorder.to_graph()


def _control_flow() -> ApxmGraph:
    recorder = GraphRecorder("control_flow", metadata={"is_entry": True})
    classified = recorder.ask(name="classify", prompt="Classify the request")
    recorder.branch(
        name="branch",
        condition_node=classified,
        value="approved",
        true_label="approved_path",
        false_label="review_path",
    )
    routed = recorder.switch_(
        name="route",
        discriminant="classification",
        cases=["approved", "review"],
    )
    recorder.add_edge(classified, routed)
    recorder.try_catch(name="recover", try_label="route", catch_label="fallback")
    recorder.done(routed, name="out")
    return recorder.to_graph()


def _synchronization_flow() -> ApxmGraph:
    recorder = GraphRecorder("synchronization_flow", metadata={"is_entry": True})
    answer = recorder.ask(name="answer", prompt="Prepare the durable result")
    checkpoint = recorder.checkpoint("after_answer", name="checkpoint")
    recorder.add_edge(answer, checkpoint)
    fence = recorder.fence(name="fence", ordering="serial")
    merged = recorder.merge("merged", checkpoint, fence)
    recorder.done(merged, name="out")
    return recorder.to_graph()


def _coordination_flow() -> ApxmGraph:
    recorder = GraphRecorder("coordination_flow", metadata={"is_entry": True})
    worker = recorder.spawn_agent(name="worker", agent_name="worker", mode="collaborative")
    delegated = recorder.delegate(
        name="delegate",
        task_spec="Review the frontend contract",
        target_agent="worker",
    )
    recorder.add_edge(worker, delegated)
    transferred = recorder.handoff(
        name="handoff",
        handoff_from="orchestrator",
        handoff_to="worker",
        payload="Begin the assigned review.",
        transfer_state=True,
    )
    recorder.add_edge(delegated, transferred)
    recorder.done(transferred, name="out")
    return recorder.to_graph()


def test_native_authoring_matches_shared_dto_vectors():
    assert _ask_flow().to_dict() == _load_fixture("ask_flow.json")
    assert _prompt_role_flow().to_dict() == _load_fixture("prompt_role_flow.json")
    assert _parametrized_flow().to_dict() == _load_fixture("parametrized_flow.json")
    assert _profiled_agent_flow().to_dict() == _load_fixture("profiled_agent_flow.json")
    assert [graph.to_dict() for graph in _multi_flow_conversational()] == _load_fixture(
        "multi_flow_conversational.json"
    )
    assert _reasoning_flow().to_dict() == _load_fixture("reasoning_flow.json")
    assert _control_flow().to_dict() == _load_fixture("control_flow.json")
    assert _synchronization_flow().to_dict() == _load_fixture("synchronization_flow.json")
    assert _coordination_flow().to_dict() == _load_fixture("coordination_flow.json")


@pytest.mark.parametrize(
    ("golden_name", "emit"),
    [
        ("ask_flow.golden.air", lambda: _ask_flow().to_air()),
        ("prompt_role_flow.golden.air", lambda: _prompt_role_flow().to_air()),
        ("parametrized_flow.golden.air", lambda: _parametrized_flow().to_air()),
        ("profiled_agent_flow.golden.air", lambda: _profiled_agent_flow().to_air()),
        ("multi_flow_conversational.golden.air", _multi_flow_conversational_air),
        ("reasoning_flow.golden.air", lambda: _reasoning_flow().to_air()),
        ("control_flow.golden.air", lambda: _control_flow().to_air()),
        ("synchronization_flow.golden.air", lambda: _synchronization_flow().to_air()),
        ("coordination_flow.golden.air", lambda: _coordination_flow().to_air()),
    ],
)
def test_live_air_matches_golden_vectors(golden_name: str, emit: Callable[[], str]):
    assert emit() == _load_golden(golden_name)


def test_air_emitter_command_raises_air_emission_error_when_apxm_cannot_be_resolved():
    bogus_binary = "/nonexistent/path/apxm-does-not-exist-w3-1"
    command = AirEmitterCommand(argv=(bogus_binary, "emit-air"))

    with pytest.raises(AirEmissionError) as excinfo:
        command.emit({"name": "unreachable", "nodes": [], "edges": [], "parameters": [], "metadata": {}})

    message = str(excinfo.value)
    assert bogus_binary in message
    assert "could not be started" in message
