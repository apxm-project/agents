"""Authoring tests for CALL_SKILL, SPAWN_TEAM, and the bounded LOOP region.

These exercise the GraphRecorder DSL surfaces added so a conversational
multi-agent agent can be authored entirely from the Python frontend:
``g.call_skill``, ``g.spawn_team``, the ``g.loop`` context-manager, the
``loop_start`` ``max_iterations`` fix, and ``AgentHandle.chat``.
"""

from __future__ import annotations

from apxm import GraphRecorder, Loop
from apxm import constants as g


def _by_op(graph, op):
    return [n for n in graph.nodes if n.op == op]


def test_call_skill_emits_node_with_args_and_edges():
    rec = GraphRecorder("skill_flow")
    src = rec.ask(name="q", prompt="hi")
    cs = rec.call_skill("summarize", args={"text": src, "tone": "concise"}, version="1.2")

    graph = rec.to_graph()
    nodes = _by_op(graph, g.OP_CALL_SKILL)
    assert len(nodes) == 1
    node = nodes[0]
    assert node.attributes[g.SKILL_ID] == "summarize@1.2"
    # NodeRef arg auto-wires as {text} placeholder + input_names + Data edge.
    assert node.attributes[g.ARGS]["text"] == "{text}"
    assert node.attributes[g.ARGS]["tone"] == "concise"
    assert node.attributes[g.INPUT_NAMES] == ["text"]
    assert any(
        e.to_id == cs._node_id and e.from_id == src._node_id for e in graph.edges
    )


def test_call_skill_requires_skill_id():
    rec = GraphRecorder("skill_flow")
    try:
        rec.call_skill("")
    except ValueError as exc:
        assert "skill_id" in str(exc)
    else:
        raise AssertionError("expected ValueError for empty skill_id")


def test_spawn_team_emits_node():
    rec = GraphRecorder("team_flow")
    rec.spawn_team(team_name="research", cwd="/tmp/work")

    graph = rec.to_graph()
    nodes = _by_op(graph, g.OP_SPAWN_TEAM)
    assert len(nodes) == 1
    assert nodes[0].attributes[g.TEAM_NAME] == "research"
    assert nodes[0].attributes[g.CWD] == "/tmp/work"
    # No member edges: the runtime resolves the roster from teams.toml.
    assert not graph.edges


def test_spawn_team_requires_team_name():
    rec = GraphRecorder("team_flow")
    try:
        rec.spawn_team()
    except ValueError as exc:
        assert "team_name" in str(exc)
    else:
        raise AssertionError("expected ValueError for missing team_name")


def test_loop_start_writes_max_iterations_not_count():
    rec = GraphRecorder("loop_flow")
    rec.loop_start(count=5, label="refine")

    graph = rec.to_graph()
    node = _by_op(graph, g.OP_LOOP_START)[0]
    # The bug fix: the bound reaches the runtime as max_iterations.
    assert node.attributes[g.MAX_ITERATIONS] == 5
    assert node.attributes[g.COUNT_TOKEN] == "5"
    assert node.attributes[g.LABEL] == "refine"
    # The old (silently-dropped) 'count' attr must not be the carrier.
    assert g.COUNT not in node.attributes


def test_loop_context_manager_wires_region():
    rec = GraphRecorder("loop_flow")
    with rec.loop(count=3, label="refine") as lp:
        assert isinstance(lp, Loop)
        body = lp.step(rec.ask(name="step1", prompt="improve"))

    graph = rec.to_graph()
    start_node = _by_op(graph, g.OP_LOOP_START)[0]
    assert lp.end is not None
    # Counter Data edge LOOP_START -> LOOP_END.
    assert any(
        e.from_id == lp.start._node_id
        and e.to_id == lp.end._node_id
        and e.dependency == g.DEPENDENCY_DATA
        for e in graph.edges
    )
    # Control edge from body into LOOP_END.
    assert any(
        e.from_id == body._node_id
        and e.to_id == lp.end._node_id
        and e.dependency == g.DEPENDENCY_CONTROL
        for e in graph.edges
    )


def test_agent_handle_chat_chains_turns():
    rec = GraphRecorder("chat_flow")
    agent = rec.spawn("assistant")
    last = agent.chat(["hello", "follow up", "and again"])

    graph = rec.to_graph()
    comms = _by_op(graph, g.OP_COMMUNICATE)
    assert len(comms) == 3
    assert last is not None
    # last NodeRef is the final COMMUNICATE turn.
    assert any(n.id == last._node_id and n.op == g.OP_COMMUNICATE for n in graph.nodes)
    # Consecutive turns are joined by Data dependencies (2 for 3 turns).
    data_edges = [e for e in graph.edges if e.dependency == g.DEPENDENCY_DATA]
    assert len(data_edges) >= 2


def test_conversational_agent_program_builds_and_emits_air():
    rec = GraphRecorder("assistant")
    rec.reason(name="plan", prompt="Plan a response to: {conversation}")
    answer = rec.ask(
        name="answer",
        prompt="Answer using tools: {conversation}",
        tool_groups=["web"],
    )
    rec.call_skill("summarize", args={"text": answer})
    rec.spawn_team(team_name="reviewers")
    rec.done(source=answer)

    air = rec.to_graph().to_air()
    assert "ais.call_skill" in air
    assert "ais.spawn_team" in air
    assert "ais.reason" in air
