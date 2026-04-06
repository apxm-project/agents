"""Test graph construction API."""

import json
import pytest


def test_simple_graph():
    """Test basic graph construction."""
    from apxm import GraphRecorder

    g = GraphRecorder("simple_test")
    ask_node = g.ask("query", "What is {0}?")

    graph = g.to_graph()

    assert graph.name == "simple_test"
    assert len(graph.nodes) == 1
    assert graph.nodes[0].name == "query"
    assert graph.nodes[0].op == "ASK"
    assert graph.nodes[0].attributes["template_str"] == "What is {0}?"


def test_graph_with_params():
    """Test graph with explicit parameters."""
    from apxm import GraphRecorder

    g = GraphRecorder("with_params")
    g.param("topic", "str")
    g.param("depth", "int")
    ask_node = g.ask("query", "Research {0} with depth {1}")

    graph = g.to_graph()

    assert len(graph.parameters) == 2
    assert graph.parameters[0].name == "topic"
    assert graph.parameters[0].type_name == "str"
    assert graph.parameters[1].name == "depth"
    assert graph.parameters[1].type_name == "int"


def test_graph_edges():
    """Test graph edges with >> and | operators."""
    from apxm import GraphRecorder

    g = GraphRecorder("edges_test")
    a = g.ask("step1", "Do A")
    b = g.ask("step2", "Do B")
    c = g.ask("step3", "Do C")

    # Control edge
    a >> b
    # Data edge
    b | c

    graph = g.to_graph()

    assert len(graph.edges) == 2
    control_edge = [e for e in graph.edges if e.dependency == "Control"][0]
    data_edge = [e for e in graph.edges if e.dependency == "Data"][0]

    assert control_edge.from_id == a._node_id
    assert control_edge.to_id == b._node_id
    assert data_edge.from_id == b._node_id
    assert data_edge.to_id == c._node_id


def test_spawn_and_communicate():
    """Test spawn_agent and communicate nodes."""
    from apxm import GraphRecorder
    from apxm._generated.constants import OP_SPAWN_AGENT, OP_COMMUNICATE

    g = GraphRecorder("spawn_test")
    spawn = g.spawn_agent("alice_spawn", agent_name="alice", profile="claude")
    comm = g.communicate("alice_msg", target_agent="alice", message="Hello")
    spawn >> comm

    graph = g.to_graph()

    assert len(graph.nodes) == 2
    assert graph.nodes[0].op == OP_SPAWN_AGENT
    assert graph.nodes[1].op == OP_COMMUNICATE


def test_team_sugar():
    """Test Team sugar for spawning multiple agents."""
    from apxm import GraphRecorder

    g = GraphRecorder("team_test")
    team = g.team("research_team")

    alice = team.add("alice", profile="claude")
    bob = team.add("bob", profile="codex")

    alice.ask("Research X")
    bob.ask("Research Y")

    sync = team.wait_all()

    graph = g.to_graph()

    # Should have: 2 spawns + 2 communicates + 1 wait_all
    assert len(graph.nodes) >= 5
    spawn_nodes = [n for n in graph.nodes if n.op == "SPAWN_AGENT"]
    comm_nodes = [n for n in graph.nodes if n.op == "COMMUNICATE"]
    wait_nodes = [n for n in graph.nodes if n.op == "WAIT_ALL"]

    assert len(spawn_nodes) == 2
    assert len(comm_nodes) == 2
    assert len(wait_nodes) == 1


def test_agent_handle_chaining():
    """Test AgentHandle method chaining."""
    from apxm import GraphRecorder

    g = GraphRecorder("handle_test")
    handle = g.spawn("alice", profile="claude")
    handle.ask("Do task 1").ask("Do task 2").ask("Do task 3")

    graph = g.to_graph()

    # 1 spawn + 3 communicates
    assert len(graph.nodes) == 4
    comm_nodes = [n for n in graph.nodes if n.op == "COMMUNICATE"]
    assert len(comm_nodes) == 3


def test_graph_to_json():
    """Test graph serialization to JSON."""
    from apxm import GraphRecorder

    g = GraphRecorder("json_test")
    g.param("input", "str")
    g.ask("process", "Process {0}")

    graph = g.to_graph()
    json_str = graph.to_json()

    data = json.loads(json_str)

    assert data["name"] == "json_test"
    assert len(data["nodes"]) == 1
    assert len(data["parameters"]) == 1
    assert data["parameters"][0]["name"] == "input"


def test_graph_validation():
    """Test graph validation."""
    from apxm import GraphRecorder, validate_graph

    g = GraphRecorder("valid_graph")
    g.ask("test", "Test query")

    graph = g.to_graph()
    errors = validate_graph(graph)

    assert len(errors) == 0
