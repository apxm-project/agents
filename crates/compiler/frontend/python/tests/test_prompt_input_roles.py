"""Role-bearing LLM inputs serialize to the shared frontend graph DTO."""

from __future__ import annotations

import pytest

from apxm.constants import INPUT_NAMES, INPUT_ROLES, PromptInputRole, SYSTEM_PROMPT_INPUT
from apxm.proxy import GraphRecorder, prompt_input


def _node_attributes(graph: GraphRecorder, node_id: int) -> dict[str, object]:
    return graph.to_graph().nodes[node_id - 1].attributes


def test_ask_serializes_every_prompt_input_role_in_input_order():
    graph = GraphRecorder("prompt_roles")
    question = graph.print(name="question", message="question")
    policy = graph.print(name="policy", message="policy")
    dependency = graph.print(name="dependency", message="dependency")
    tool_result = graph.print(name="tool_result", message="tool_result")
    guard = graph.print(name="guard", message="guard")

    ask = graph.ask(
        prompt="Answer {question}",
        prompt_inputs={
            "policy": prompt_input(policy, PromptInputRole.SYSTEM),
            "dependency": prompt_input(dependency, PromptInputRole.DEPENDENCY_ONLY),
            "tool_result": prompt_input(tool_result, PromptInputRole.TOOL_CONTEXT),
            "guard": prompt_input(guard, PromptInputRole.CONTROL),
        },
    )

    attrs = _node_attributes(graph, ask._node_id)
    assert attrs[INPUT_NAMES] == ["question", "policy", "dependency", "tool_result", "guard"]
    assert attrs[INPUT_ROLES] == [
        "user",
        "system",
        "dependency_only",
        "tool_context",
        "control",
    ]
    answer_edges = [edge for edge in graph.to_graph().edges if edge.to_id == ask._node_id]
    assert [edge.from_id for edge in answer_edges] == [
        question._node_id,
        policy._node_id,
        dependency._node_id,
        tool_result._node_id,
        guard._node_id,
    ]


@pytest.mark.parametrize("method_name", ["ask", "think", "reason"])
def test_llm_builders_share_role_binding_api(method_name: str):
    graph = GraphRecorder(f"{method_name}_roles")
    question = graph.print(name="question", message="question")
    policy = graph.print(name="policy", message="policy")

    node = getattr(graph, method_name)(
        prompt="Answer {question}",
        prompt_inputs={"policy": prompt_input(policy, PromptInputRole.SYSTEM)},
    )

    attrs = _node_attributes(graph, node._node_id)
    assert attrs[INPUT_NAMES] == ["question", "policy"]
    assert attrs[INPUT_ROLES] == ["user", "system"]


def test_legacy_system_prompt_input_emits_the_system_role():
    graph = GraphRecorder("legacy_system")
    policy = graph.print(name="policy", message="policy")

    ask = graph.ask(prompt="Answer", system_prompt_input=policy)

    attrs = _node_attributes(graph, ask._node_id)
    assert attrs[INPUT_NAMES] == [SYSTEM_PROMPT_INPUT]
    assert attrs[INPUT_ROLES] == ["system"]


def test_prompt_input_metadata_cannot_bypass_typed_bindings():
    graph = GraphRecorder("typed_roles")

    with pytest.raises(ValueError, match="prompt_inputs"):
        graph.ask(prompt="Answer", input_names=["question"])
