"""Test graph construction API."""

import json
import pytest

WEB_TOOL_GROUP = "web"
FILE_READ_TOOL_GROUP = "file:read"
MOCK_AGENT_PROFILE = "mock-agent-profile"
MOCK_AGENT_PROFILE_ALT = "mock-agent-profile-alt"


def test_simple_graph():
    """Test basic graph construction."""
    from apxm import GraphRecorder

    g = GraphRecorder("simple_test")
    g.param("topic", "str")
    ask_node = g.ask(name="query", prompt="What is {topic}?")

    graph = g.to_graph()

    assert graph.name == "simple_test"
    assert len(graph.nodes) == 1
    assert graph.nodes[0].name == "query"
    assert graph.nodes[0].op == "ASK"
    assert graph.nodes[0].attributes["template_str"] == "What is {topic}?"


def test_agent_config_emits_tool_groups_and_enables_grouped_tools():
    from apxm import AgentConfig

    agent = AgentConfig(name="researcher", tool_groups=[WEB_TOOL_GROUP, FILE_READ_TOOL_GROUP])
    attrs = agent.to_node_attributes()

    assert attrs["tool_groups"] == [WEB_TOOL_GROUP, FILE_READ_TOOL_GROUP]
    assert attrs["tools_enabled"] is True


def test_agent_config_explicit_tools_enabled_overrides_tool_group_default():
    from apxm import AgentConfig

    agent = AgentConfig(name="researcher", tool_groups=[WEB_TOOL_GROUP], tools_enabled=False)
    attrs = agent.to_node_attributes()

    assert attrs["tool_groups"] == [WEB_TOOL_GROUP]
    assert attrs["tools_enabled"] is False


def test_graph_policy_applies_default_node_attrs():
    from apxm import GraphRecorder, NodePolicy

    g = GraphRecorder(
        "policy_defaults",
        policy=NodePolicy(tool_groups=[WEB_TOOL_GROUP], token_budget=128, timeout_ms=2500),
    )
    g.ask(name="research", prompt="Find sources")

    graph = g.to_graph()
    attrs = graph.nodes[0].attributes
    assert attrs["tool_groups"] == [WEB_TOOL_GROUP]
    assert attrs["tools_enabled"] is True
    assert attrs["token_budget"] == 128
    assert attrs["timeout_ms"] == 2500


def test_node_policy_overrides_graph_policy():
    from apxm import GraphRecorder, NodePolicy

    g = GraphRecorder("policy_override", policy=NodePolicy(tool_groups=[WEB_TOOL_GROUP], token_budget=64))
    g.ask(
        name="research",
        prompt="Find sources",
        policy=NodePolicy(
            tool_groups=[FILE_READ_TOOL_GROUP],
            token_budget=256,
            tools_enabled=False,
        ),
    )

    graph = g.to_graph()
    attrs = graph.nodes[0].attributes
    assert attrs["tool_groups"] == [FILE_READ_TOOL_GROUP]
    assert attrs["token_budget"] == 256
    assert attrs["tools_enabled"] is False


def test_graph_with_params():
    """Test graph with explicit parameters."""
    from apxm import GraphRecorder

    g = GraphRecorder("with_params")
    g.param("topic", "str")
    g.param("depth", "int")
    ask_node = g.ask(name="query", prompt="Research {topic} with depth {depth}")

    graph = g.to_graph()

    assert len(graph.parameters) == 2
    assert graph.parameters[0].name == "topic"
    assert graph.parameters[0].type_name == "str"
    assert graph.parameters[1].name == "depth"
    assert graph.parameters[1].type_name == "int"


def test_graph_edges():
    """Test graph edges with add_edge()."""
    from apxm import GraphRecorder

    g = GraphRecorder("edges_test")
    a = g.ask(name="step1", prompt="Do A")
    b = g.ask(name="step2", prompt="Do B")
    c = g.ask(name="step3", prompt="Do C")

    # Control edge
    g.add_edge(a, b, dependency="Control")
    # Data edge
    g.add_edge(b, c, dependency="Data")

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
    from apxm.constants import OP_SPAWN_AGENT, OP_COMMUNICATE

    g = GraphRecorder("spawn_test")
    spawn = g.spawn_agent("alice_spawn", agent_name="alice", profile=MOCK_AGENT_PROFILE)
    comm = g.communicate(name="alice_msg", target_agent="alice", message="Hello")
    g.add_edge(spawn, comm, dependency="Control")

    graph = g.to_graph()

    assert len(graph.nodes) == 2
    assert graph.nodes[0].op == OP_SPAWN_AGENT
    assert graph.nodes[1].op == OP_COMMUNICATE


def test_team_sugar():
    """Test Team sugar for spawning multiple agents."""
    from apxm import GraphRecorder

    g = GraphRecorder("team_test")
    team = g.team("research_team")

    alice = team.add("alice", profile=MOCK_AGENT_PROFILE)
    bob = team.add("bob", profile=MOCK_AGENT_PROFILE_ALT)

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
    handle = g.spawn("alice", profile=MOCK_AGENT_PROFILE)
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
    g.ask(name="process", prompt="Process {input}")

    graph = g.to_graph()
    json_str = graph.to_json()

    data = json.loads(json_str)

    assert data["name"] == "json_test"
    assert len(data["nodes"]) == 1
    assert len(data["parameters"]) == 1
    assert data["parameters"][0]["name"] == "input"


def test_graph_to_air_preserves_full_literals():
    """Test .air serialization emits valid MLIR with proper string escaping."""
    from apxm import GraphRecorder

    long_prompt = (
        'Line 1 says "hello".\n'
        "Line 2 keeps going past sixty characters to verify the emitter never truncates values."
    )

    g = GraphRecorder("air_test")
    g.ask(name="emit", prompt=long_prompt)
    g.spawn_agent("alice", agent_name="alice", profile=MOCK_AGENT_PROFILE, mode="auto")

    air = g.to_air()

    # Check new MLIR format where template is a positional arg
    assert 'ais.ask "Line 1 says \\"hello\\".\\nLine 2 keeps going past sixty characters to verify the emitter never truncates values."' in air

    # Check that spawn_agent emits known attributes
    assert 'ais.spawn_agent "alice"' in air
    assert f'profile = "{MOCK_AGENT_PROFILE}"' in air
    assert 'mode = "auto"' in air

    # Verify it's valid MLIR structure
    assert "module {" in air
    assert "func.func @air_test" in air
    assert "func.return" in air


def test_graph_validation():
    """Test graph validation."""
    from apxm import GraphRecorder, validate_graph

    g = GraphRecorder("valid_graph")
    g.ask(name="test", prompt="Test query")

    graph = g.to_graph()
    errors = validate_graph(graph)

    assert len(errors) == 0


def test_call_compiled_flow():
    """Test g.call() with a @compile-decorated function."""
    from apxm import GraphRecorder, compile

    @compile()
    def helper(g: GraphRecorder, topic: str):
        result = g.ask(name="research", prompt=f"Research {{topic}}")
        g.done(result)

    g = GraphRecorder("main_flow")
    step1 = g.ask(name="get_topic", prompt="What topic?")
    step2 = g.call(helper, topic=step1)
    g.done(step2)

    graph = g.to_graph()

    # Should have: get_topic (ASK) + call_helper (FLOW_CALL) + return (RETURN)
    assert len(graph.nodes) == 3
    flow_call_nodes = [n for n in graph.nodes if n.op == "FLOW_CALL"]
    assert len(flow_call_nodes) == 1
    assert flow_call_nodes[0].attributes["agent_name"] == "helper"
    assert flow_call_nodes[0].attributes["flow_name"] == "main"
    assert flow_call_nodes[0].attributes["input_names"] == ["topic"]
    assert flow_call_nodes[0].attributes["args"]["topic"] == "{topic}"

    # Should have data edge from step1 -> flow_call
    data_edges = [e for e in graph.edges if e.dependency == "Data"]
    assert any(e.from_id == step1._node_id and e.to_id == step2._node_id for e in data_edges)


def test_call_with_literal_args():
    """Test g.call() with literal (non-NodeRef) arguments."""
    from apxm import GraphRecorder, compile

    @compile()
    def helper(g: GraphRecorder, topic: str):
        g.ask(name="research", prompt=f"Research {{topic}}")

    g = GraphRecorder("main_flow")
    result = g.call(helper, topic="AI safety")

    graph = g.to_graph()

    flow_call_node = [n for n in graph.nodes if n.op == "FLOW_CALL"][0]
    # Literal args should be serialized in the args attribute
    assert "AI safety" in str(flow_call_node.attributes.get("args", ""))
    assert flow_call_node.attributes["args"]["topic"] == "AI safety"


def test_flow_call_auto_wires_node_ref_args():
    """Test direct flow_call() auto-wires NodeRef argument values."""
    from apxm import GraphRecorder

    g = GraphRecorder("main_flow")
    step1 = g.ask(name="get_topic", prompt="What topic?")
    step2 = g.flow_call(
        agent_name="researcher",
        flow_name="main",
        args={"topic": step1, "audience": "engineers"},
    )
    g.done(step2)

    graph = g.to_graph()
    flow_call_node = [n for n in graph.nodes if n.op == "FLOW_CALL"][0]
    assert flow_call_node.attributes["input_names"] == ["topic"]
    assert flow_call_node.attributes["args"]["topic"] == "{topic}"
    assert flow_call_node.attributes["args"]["audience"] == "engineers"

    data_edges = [e for e in graph.edges if e.dependency == "Data"]
    assert any(e.from_id == step1._node_id and e.to_id == step2._node_id for e in data_edges)


def test_workflow_spawn_auto_wires_node_ref_args():
    from apxm import GraphRecorder, WorkflowTargetKind

    g = GraphRecorder("main_flow")
    step1 = g.ask(name="get_topic", prompt="What topic?")
    step2 = g.workflow_spawn(
        target_kind=WorkflowTargetKind.WORKFLOW_PATH,
        target="workflows/review.apxmw",
        args={"topic": step1, "audience": "engineers"},
    )
    g.done(step2)

    graph = g.to_graph()
    spawn_node = [n for n in graph.nodes if n.op == "WORKFLOW_SPAWN"][0]
    assert spawn_node.attributes["target_kind"] == WorkflowTargetKind.WORKFLOW_PATH.value
    assert spawn_node.attributes["target"] == "workflows/review.apxmw"
    assert spawn_node.attributes["await_result"] is True
    assert spawn_node.attributes["input_names"] == ["topic"]
    assert spawn_node.attributes["args"]["topic"] == "{topic}"
    assert spawn_node.attributes["args"]["audience"] == "engineers"

    data_edges = [e for e in graph.edges if e.dependency == "Data"]
    assert any(e.from_id == step1._node_id and e.to_id == step2._node_id for e in data_edges)


def test_workflow_spawn_applies_node_policy_and_session_root():
    from apxm import GraphRecorder, NodePolicy, WorkflowTargetKind

    g = GraphRecorder("main_flow")
    node = g.workflow_spawn(
        target_kind=WorkflowTargetKind.GRAPH_PATH,
        target="graphs/review.air",
        session_root=".apxm/custom",
        node_policy=NodePolicy(timeout_ms=2_500, token_budget=64),
    )

    graph = g.to_graph()
    spawn_node = [n for n in graph.nodes if n.id == node._node_id][0]
    assert spawn_node.attributes["session_root"] == ".apxm/custom"
    assert spawn_node.attributes["timeout_ms"] == 2500
    assert spawn_node.attributes["token_budget"] == 64


def test_workflow_spawn_rejects_invalid_kind_and_detached_mode():
    from apxm import GraphRecorder, WorkflowTargetKind

    g = GraphRecorder("main_flow")

    with pytest.raises(ValueError, match="target_kind"):
        g.workflow_spawn(
            target_kind=WorkflowTargetKind.REGISTERED_FLOW,
            target="foo.air",
        )

    with pytest.raises(ValueError, match="await_result=True"):
        g.workflow_spawn(
            target_kind=WorkflowTargetKind.GRAPH_PATH,
            target="foo.air",
            await_result=False,
        )


def test_embed_compiled_flow():
    """Test g.embed() to inline-compose another graph."""
    from apxm import GraphRecorder, compile

    @compile()
    def helper(g: GraphRecorder):
        a = g.ask(name="step_a", prompt="Do A")
        b = g.ask(name="step_b", prompt="Do B based on {step_a}")
        g.done(b)

    g = GraphRecorder("main_flow")
    embedded = g.embed(helper, prefix="sub")
    g.done(embedded)

    graph = g.to_graph()

    # Should have the helper's nodes (step_a, step_b, return) prefixed + main's return
    node_names = [n.name for n in graph.nodes]
    assert "sub_step_a" in node_names
    assert "sub_step_b" in node_names


def test_embed_flow_module():
    """Test g.embed() with a FlowModule."""
    from apxm import GraphRecorder, FlowModule

    class MyModule(FlowModule):
        def define(self, g: GraphRecorder):
            return g.ask(name="inner", prompt="Inner task")

    g = GraphRecorder("main_flow")
    result = g.embed(MyModule(), prefix="mod")
    g.done(result)

    graph = g.to_graph()
    node_names = [n.name for n in graph.nodes]
    assert "mod_inner" in node_names


def test_load_graph():
    """Test load_graph() from JSON file."""
    import tempfile
    from pathlib import Path
    from apxm import GraphRecorder, load_graph

    # Create a graph and save it
    g = GraphRecorder("test_graph")
    g.param("input", "str")
    g.ask(name="process", prompt="Process {input}")
    graph = g.to_graph()

    with tempfile.NamedTemporaryFile(suffix=".json", mode="w", delete=False) as f:
        f.write(graph.to_json())
        tmp_path = f.name

    try:
        loaded = load_graph(tmp_path)
        assert loaded.name == "test_graph"
        assert len(loaded.nodes) == 1
        assert len(loaded.parameters) == 1
    finally:
        Path(tmp_path).unlink(missing_ok=True)


def test_call_loaded_graph():
    """Test g.call() with a loaded ApxmGraph."""
    import tempfile
    from pathlib import Path
    from apxm import GraphRecorder, load_graph

    # Create and save helper graph
    helper_g = GraphRecorder("helper_flow")
    helper_g.param("input", "str")
    helper_g.ask(name="process", prompt="Process {input}")
    helper_graph = helper_g.to_graph()

    with tempfile.NamedTemporaryFile(suffix=".json", mode="w", delete=False) as f:
        f.write(helper_graph.to_json())
        tmp_path = f.name

    try:
        loaded = load_graph(tmp_path)

        # Use in a new graph via call()
        g = GraphRecorder("main_flow")
        step1 = g.ask(name="get_input", prompt="What input?")
        step2 = g.call(loaded, input=step1)
        g.done(step2)

        graph = g.to_graph()
        flow_call_nodes = [n for n in graph.nodes if n.op == "FLOW_CALL"]
        assert len(flow_call_nodes) == 1
        assert flow_call_nodes[0].attributes["agent_name"] == "helper_flow"
        assert flow_call_nodes[0].attributes["input_names"] == ["input"]
        assert flow_call_nodes[0].attributes["args"]["input"] == "{input}"
    finally:
        Path(tmp_path).unlink(missing_ok=True)


def test_call_rejects_unknown_argument_names():
    from apxm import GraphRecorder

    helper_g = GraphRecorder("helper_flow")
    helper_g.param("input", "str")
    helper_graph = helper_g.to_graph()

    g = GraphRecorder("main_flow")
    step1 = g.ask(name="get_input", prompt="What input?")

    with pytest.raises(TypeError, match="unknown argument"):
        g.call(helper_graph, topic=step1)
