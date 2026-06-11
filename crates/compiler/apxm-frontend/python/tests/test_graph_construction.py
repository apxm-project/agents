"""Test graph construction API."""
import pytest

from apxm.constants import (
    AGENT_NAME,
    ARGS,
    AWAIT_RESULT,
    DEPENDENCY_CONTROL,
    DEPENDENCY_DATA,
    FLOW_NAME,
    INPUT_NAMES,
    OP_ASK,
    OP_COMMUNICATE,
    OP_FLOW_CALL,
    OP_SPAWN_AGENT,
    OP_WAIT_ALL,
    OP_WORKFLOW_SPAWN,
    SESSION_ROOT,
    TARGET,
    TARGET_KIND,
    TEMPLATE_STR,
    TIMEOUT_MS,
    TOKEN_BUDGET,
    TOOL_GROUPS,
    TOOLS_ENABLED,
)

from .mocks import MOCK_AGENT_PROFILE, MOCK_AGENT_PROFILE_ALT

WEB_TOOL_GROUP = "web"
FILE_READ_TOOL_GROUP = "file:read"
HELPER_FLOW_NAME = "helper_flow"
MAIN_FLOW_NAME = "main_flow"
INPUT_PARAM = "input"
TOPIC_PARAM = "topic"
TOPIC_PLACEHOLDER = f"{{{TOPIC_PARAM}}}"
INPUT_PLACEHOLDER = f"{{{INPUT_PARAM}}}"


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
    assert graph.nodes[0].op == OP_ASK
    assert graph.nodes[0].attributes[TEMPLATE_STR] == "What is {topic}?"


def test_agent_config_emits_tool_groups_and_enables_grouped_tools():
    from apxm import AgentConfig

    agent = AgentConfig(name="researcher", tool_groups=[WEB_TOOL_GROUP, FILE_READ_TOOL_GROUP])
    attrs = agent.to_node_attributes()

    assert attrs[TOOL_GROUPS] == [WEB_TOOL_GROUP, FILE_READ_TOOL_GROUP]
    assert attrs[TOOLS_ENABLED] is True


def test_agent_config_explicit_tools_enabled_overrides_tool_group_default():
    from apxm import AgentConfig

    agent = AgentConfig(name="researcher", tool_groups=[WEB_TOOL_GROUP], tools_enabled=False)
    attrs = agent.to_node_attributes()

    assert attrs[TOOL_GROUPS] == [WEB_TOOL_GROUP]
    assert attrs[TOOLS_ENABLED] is False


def test_graph_policy_applies_default_node_attrs():
    from apxm import GraphRecorder, NodePolicy

    g = GraphRecorder(
        "policy_defaults",
        policy=NodePolicy(tool_groups=[WEB_TOOL_GROUP], token_budget=128, timeout_ms=2500),
    )
    g.ask(name="research", prompt="Find sources")

    graph = g.to_graph()
    attrs = graph.nodes[0].attributes
    assert attrs[TOOL_GROUPS] == [WEB_TOOL_GROUP]
    assert attrs[TOOLS_ENABLED] is True
    assert attrs[TOKEN_BUDGET] == 128
    assert attrs[TIMEOUT_MS] == 2500


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
    assert attrs[TOOL_GROUPS] == [FILE_READ_TOOL_GROUP]
    assert attrs[TOKEN_BUDGET] == 256
    assert attrs[TOOLS_ENABLED] is False


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
    g.add_edge(a, b, dependency=DEPENDENCY_CONTROL)
    # Data edge
    g.add_edge(b, c, dependency=DEPENDENCY_DATA)

    graph = g.to_graph()

    assert len(graph.edges) == 2
    control_edge = [e for e in graph.edges if e.dependency == DEPENDENCY_CONTROL][0]
    data_edge = [e for e in graph.edges if e.dependency == DEPENDENCY_DATA][0]

    assert control_edge.from_id == a._node_id
    assert control_edge.to_id == b._node_id
    assert data_edge.from_id == b._node_id
    assert data_edge.to_id == c._node_id


def test_auto_wire_resolves_outer_node_refs_from_list_comprehension():
    from apxm import GraphRecorder
    from apxm import constants as graph_keys

    g = GraphRecorder("comprehension_scope")
    artifact = g.ask(name="artifact", prompt="Produce context")
    extracts = [
        g.ask(name=f"extract_{index}", prompt="Use {artifact}")
        for index in range(2)
    ]

    graph = g.to_graph()

    for ref in extracts:
        node = next(node for node in graph.nodes if node.id == ref._node_id)
        assert node.attributes[graph_keys.INPUT_NAMES] == ["artifact"]
        assert any(
            edge.from_id == artifact._node_id
            and edge.to_id == ref._node_id
            and edge.dependency == graph_keys.DEPENDENCY_DATA
            for edge in graph.edges
        )


def test_auto_wire_respects_comprehension_shadowing():
    from apxm import GraphRecorder
    from apxm import constants as graph_keys

    g = GraphRecorder("comprehension_shadow")
    artifact = g.ask(name="artifact", prompt="Produce context")
    shadowed = [
        g.ask(name="shadowed", prompt="Use {artifact}")
        for artifact in ["literal context"]
    ][0]

    graph = g.to_graph()
    node = next(node for node in graph.nodes if node.id == shadowed._node_id)

    assert graph_keys.INPUT_NAMES not in node.attributes
    assert not any(
        edge.from_id == artifact._node_id
        and edge.to_id == shadowed._node_id
        and edge.dependency == graph_keys.DEPENDENCY_DATA
        for edge in graph.edges
    )


def test_spawn_and_communicate():
    """Test spawn_agent and communicate nodes."""
    from apxm import GraphRecorder
    from apxm.constants import DEPENDENCY_DATA, OP_COMMUNICATE, OP_SPAWN_AGENT

    g = GraphRecorder("spawn_test")
    spawn = g.spawn_agent("alice_spawn", agent_name="alice", profile=MOCK_AGENT_PROFILE)
    comm = g.communicate(name="alice_msg", target_agent="alice", message="Hello")

    graph = g.to_graph()

    assert len(graph.nodes) == 2
    assert graph.nodes[0].op == OP_SPAWN_AGENT
    assert graph.nodes[1].op == OP_COMMUNICATE
    assert any(
        edge.from_id == spawn._node_id
        and edge.to_id == comm._node_id
        and edge.dependency == DEPENDENCY_DATA
        for edge in graph.edges
    )


def test_spawn_and_communicate_requires_data_dependency():
    from apxm import ApxmGraph, GraphNode, validate_graph
    from apxm.constants import AGENT_NAME, MESSAGE, OP_COMMUNICATE, OP_SPAWN_AGENT, RECIPIENT

    graph = ApxmGraph(
        name="spawn_validation",
        nodes=[
            GraphNode(
                id=1,
                name="alice_spawn",
                op=OP_SPAWN_AGENT,
                attributes={AGENT_NAME: "alice"},
            ),
            GraphNode(
                id=2,
                name="alice_msg",
                op=OP_COMMUNICATE,
                attributes={RECIPIENT: "alice", MESSAGE: "Hello"},
            ),
        ],
    )

    errors = validate_graph(graph)

    assert any("SPAWN_AGENT token" in error for error in errors)


def test_spawn_and_communicate_rejects_control_dependency():
    from apxm import ApxmGraph, GraphEdge, GraphNode, validate_graph
    from apxm.constants import (
        AGENT_NAME,
        DEPENDENCY_CONTROL,
        MESSAGE,
        OP_COMMUNICATE,
        OP_SPAWN_AGENT,
        RECIPIENT,
    )

    graph = ApxmGraph(
        name="spawn_control_validation",
        nodes=[
            GraphNode(
                id=1,
                name="alice_spawn",
                op=OP_SPAWN_AGENT,
                attributes={AGENT_NAME: "alice"},
            ),
            GraphNode(
                id=2,
                name="alice_msg",
                op=OP_COMMUNICATE,
                attributes={RECIPIENT: "alice", MESSAGE: "Hello"},
            ),
        ],
        edges=[GraphEdge(from_id=1, to_id=2, dependency=DEPENDENCY_CONTROL)],
    )

    errors = validate_graph(graph)

    assert any("Control edges do not carry the spawn token" in error for error in errors)


def test_contract_validation_requires_spawn_data_dependency():
    from apxm import ApxmGraph, GraphNode
    from apxm.constants import AGENT_NAME, MESSAGE, OP_COMMUNICATE, OP_SPAWN_AGENT, RECIPIENT
    from apxm.ir import validate_against_apxm

    graph = ApxmGraph(
        name="spawn_contract_validation",
        nodes=[
            GraphNode(
                id=1,
                name="alice_spawn",
                op=OP_SPAWN_AGENT,
                attributes={AGENT_NAME: "alice"},
            ),
            GraphNode(
                id=2,
                name="alice_msg",
                op=OP_COMMUNICATE,
                attributes={RECIPIENT: "alice", MESSAGE: "Hello"},
            ),
        ],
    )

    result = validate_against_apxm(graph)

    assert not result.valid
    assert any("SPAWN_AGENT token" in error for error in result.errors)


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
    spawn_nodes = [n for n in graph.nodes if n.op == OP_SPAWN_AGENT]
    comm_nodes = [n for n in graph.nodes if n.op == OP_COMMUNICATE]
    wait_nodes = [n for n in graph.nodes if n.op == OP_WAIT_ALL]

    assert len(spawn_nodes) == 2
    assert len(comm_nodes) == 2
    assert len(wait_nodes) == 1


def test_agent_handle_ask_returns_node_refs():
    """Test AgentHandle ask() returns COMMUNICATE nodes."""
    from apxm import GraphRecorder, NodeRef

    g = GraphRecorder("handle_test")
    handle = g.spawn("alice", profile=MOCK_AGENT_PROFILE)
    first = handle.ask("Do task 1")
    second = handle.ask("Do task 2")
    third = handle.ask("Do task 3")

    graph = g.to_graph()

    # 1 spawn + 3 communicates
    assert len(graph.nodes) == 4
    comm_nodes = [n for n in graph.nodes if n.op == OP_COMMUNICATE]
    assert len(comm_nodes) == 3
    assert any(
        edge.from_id == handle.get_spawn_node()._node_id
        and edge.to_id == first._node_id
        and edge.dependency == DEPENDENCY_DATA
        for edge in graph.edges
    )
    assert isinstance(first, NodeRef)
    assert isinstance(second, NodeRef)
    assert isinstance(third, NodeRef)


def test_agent_handle_records_semantic_llm_operations():
    from apxm import GraphRecorder
    from apxm.constants import LLM_OPERATION, OP_ASK, OP_REASON, OP_THINK

    g = GraphRecorder("handle_llm_operations")
    handle = g.spawn("alice", profile=MOCK_AGENT_PROFILE)
    ask = handle.ask("Ask")
    think = handle.think("Think")
    reason = handle.reason("Reason")

    graph = g.to_graph()
    attrs_by_id = {node.id: node.attributes for node in graph.nodes}

    assert attrs_by_id[ask._node_id][LLM_OPERATION] == OP_ASK
    assert attrs_by_id[think._node_id][LLM_OPERATION] == OP_THINK
    assert attrs_by_id[reason._node_id][LLM_OPERATION] == OP_REASON


def test_validate_graph_rejects_invalid_llm_operation():
    from apxm import ApxmGraph, GraphNode, validate_graph
    from apxm.constants import LLM_OPERATION, MESSAGE, OP_COMMUNICATE, OP_PLAN, RECIPIENT

    graph = ApxmGraph(
        name="invalid_llm_operation",
        nodes=[
            GraphNode(
                id=1,
                name="agent_msg",
                op=OP_COMMUNICATE,
                attributes={
                    RECIPIENT: "external",
                    MESSAGE: "Hello",
                    LLM_OPERATION: OP_PLAN,
                },
            )
        ],
    )

    errors = validate_graph(graph)

    assert any(LLM_OPERATION in error and "expected one of" in error for error in errors)


def test_agent_handle_ask_auto_wires_node_refs():
    from apxm import GraphRecorder

    g = GraphRecorder("handle_auto_wire")
    source = g.ask(name="source", prompt="Produce context")
    handle = g.spawn("alice", profile=MOCK_AGENT_PROFILE)
    response = handle.ask("Use this context: {source}")

    graph = g.to_graph()
    comm_node = next(node for node in graph.nodes if node.id == response._node_id)
    assert comm_node.attributes[INPUT_NAMES] == ["source"]
    assert any(
        edge.from_id == source._node_id
        and edge.to_id == response._node_id
        and edge.dependency == DEPENDENCY_DATA
        for edge in graph.edges
    )


def test_graph_to_dict():
    """Test graph serialization to the in-memory dictionary shape."""
    from apxm import GraphRecorder

    g = GraphRecorder("dict_test")
    g.param("input", "str")
    g.ask(name="process", prompt="Process {input}")

    graph = g.to_graph()
    data = graph.to_dict()

    assert data["name"] == "dict_test"
    assert len(data["nodes"]) == 1
    assert len(data["parameters"]) == 1
    assert data["parameters"][0]["name"] == "input"


def test_graph_to_air_preserves_full_literals():
    from apxm import GraphRecorder

    long_prompt = (
        'Line 1 says "hello".\n'
        "Line 2 keeps going past sixty characters to verify the emitter never truncates values."
    )

    g = GraphRecorder("air_test")
    g.ask(name="emit", prompt=long_prompt)

    air = g.to_air()

    assert 'ais.ask "Line 1 says \\"hello\\".\\nLine 2 keeps going past sixty characters to verify the emitter never truncates values."' in air
    assert "module {" in air
    assert "func.func @air_test" in air
    assert "func.return" in air


@pytest.mark.parametrize("method_name", ["spawn_agent", "spawn"])
def test_spawn_rejects_raw_profile_string(method_name):
    from apxm import GraphRecorder

    g = GraphRecorder("strict_profile")

    with pytest.raises(TypeError, match="AgentRef"):
        if method_name == "spawn_agent":
            g.spawn_agent("alice", agent_name="alice", profile=MOCK_AGENT_PROFILE.name)
        else:
            g.spawn("alice", profile=MOCK_AGENT_PROFILE.name)


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
        result = g.ask(name="research", prompt=f"Research {TOPIC_PLACEHOLDER}")
        g.done(result)

    g = GraphRecorder(MAIN_FLOW_NAME)
    step1 = g.ask(name="get_topic", prompt="What topic?")
    step2 = g.call(helper, **{TOPIC_PARAM: step1})
    g.done(step2)

    graph = g.to_graph()

    # Should have: get_topic (ASK) + call_helper (FLOW_CALL) + return (RETURN)
    assert len(graph.nodes) == 3
    flow_call_nodes = [n for n in graph.nodes if n.op == OP_FLOW_CALL]
    assert len(flow_call_nodes) == 1
    assert flow_call_nodes[0].attributes[AGENT_NAME] == "helper"
    assert flow_call_nodes[0].attributes[FLOW_NAME] == "main"
    assert flow_call_nodes[0].attributes[INPUT_NAMES] == [TOPIC_PARAM]
    assert flow_call_nodes[0].attributes[ARGS][TOPIC_PARAM] == TOPIC_PLACEHOLDER

    # Should have data edge from step1 -> flow_call
    data_edges = [e for e in graph.edges if e.dependency == DEPENDENCY_DATA]
    assert any(e.from_id == step1._node_id and e.to_id == step2._node_id for e in data_edges)


def test_call_with_literal_args():
    """Test g.call() with literal (non-NodeRef) arguments."""
    from apxm import GraphRecorder, compile

    @compile()
    def helper(g: GraphRecorder, topic: str):
        g.ask(name="research", prompt=f"Research {TOPIC_PLACEHOLDER}")

    literal_topic = "AI safety"
    g = GraphRecorder(MAIN_FLOW_NAME)
    result = g.call(helper, **{TOPIC_PARAM: literal_topic})

    graph = g.to_graph()

    flow_call_node = [n for n in graph.nodes if n.op == OP_FLOW_CALL][0]
    # Literal args should be serialized in the args attribute
    assert literal_topic in str(flow_call_node.attributes.get(ARGS, ""))
    assert flow_call_node.attributes[ARGS][TOPIC_PARAM] == literal_topic


def test_flow_call_auto_wires_node_ref_args():
    """Test direct flow_call() auto-wires NodeRef argument values."""
    from apxm import GraphRecorder

    audience_param = "audience"
    audience_value = "engineers"
    g = GraphRecorder(MAIN_FLOW_NAME)
    step1 = g.ask(name="get_topic", prompt="What topic?")
    step2 = g.flow_call(
        agent_name="researcher",
        flow_name="main",
        args={TOPIC_PARAM: step1, audience_param: audience_value},
    )
    g.done(step2)

    graph = g.to_graph()
    flow_call_node = [n for n in graph.nodes if n.op == OP_FLOW_CALL][0]
    assert flow_call_node.attributes[INPUT_NAMES] == [TOPIC_PARAM]
    assert flow_call_node.attributes[ARGS][TOPIC_PARAM] == TOPIC_PLACEHOLDER
    assert flow_call_node.attributes[ARGS][audience_param] == audience_value

    data_edges = [e for e in graph.edges if e.dependency == DEPENDENCY_DATA]
    assert any(e.from_id == step1._node_id and e.to_id == step2._node_id for e in data_edges)


def test_workflow_spawn_auto_wires_node_ref_args():
    from apxm import GraphRecorder, WorkflowTargetKind

    target_path = "workflows/review.apxmw"
    audience_param = "audience"
    audience_value = "engineers"
    g = GraphRecorder(MAIN_FLOW_NAME)
    step1 = g.ask(name="get_topic", prompt="What topic?")
    step2 = g.workflow_spawn(
        target_kind=WorkflowTargetKind.WORKFLOW_PATH,
        target=target_path,
        args={TOPIC_PARAM: step1, audience_param: audience_value},
    )
    g.done(step2)

    graph = g.to_graph()
    spawn_node = [n for n in graph.nodes if n.op == OP_WORKFLOW_SPAWN][0]
    assert spawn_node.attributes[TARGET_KIND] == WorkflowTargetKind.WORKFLOW_PATH.value
    assert spawn_node.attributes[TARGET] == target_path
    assert spawn_node.attributes[AWAIT_RESULT] is True
    assert spawn_node.attributes[INPUT_NAMES] == [TOPIC_PARAM]
    assert spawn_node.attributes[ARGS][TOPIC_PARAM] == TOPIC_PLACEHOLDER
    assert spawn_node.attributes[ARGS][audience_param] == audience_value

    data_edges = [e for e in graph.edges if e.dependency == DEPENDENCY_DATA]
    assert any(e.from_id == step1._node_id and e.to_id == step2._node_id for e in data_edges)


def test_workflow_spawn_applies_node_policy_and_session_root():
    from apxm import GraphRecorder, NodePolicy, WorkflowTargetKind

    session_root = ".apxm/custom"
    g = GraphRecorder(MAIN_FLOW_NAME)
    node = g.workflow_spawn(
        target_kind=WorkflowTargetKind.AIR_PATH,
        target="steps/review.air",
        session_root=session_root,
        node_policy=NodePolicy(timeout_ms=2_500, token_budget=64),
    )

    graph = g.to_graph()
    spawn_node = [n for n in graph.nodes if n.id == node._node_id][0]
    assert spawn_node.attributes[SESSION_ROOT] == session_root
    assert spawn_node.attributes[TIMEOUT_MS] == 2500
    assert spawn_node.attributes[TOKEN_BUDGET] == 64


def test_workflow_spawn_rejects_invalid_kind_and_detached_mode():
    from apxm import GraphRecorder, WorkflowTargetKind

    g = GraphRecorder(MAIN_FLOW_NAME)

    with pytest.raises(ValueError, match=TARGET_KIND):
        g.workflow_spawn(
            target_kind=WorkflowTargetKind.REGISTERED_FLOW,
            target="foo.air",
        )

    with pytest.raises(ValueError, match="await_result=True"):
        g.workflow_spawn(
            target_kind=WorkflowTargetKind.AIR_PATH,
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


def test_call_embedded_graph_object():
    """Test g.call() with an in-memory ApxmGraph."""
    from apxm import GraphRecorder

    helper_g = GraphRecorder(HELPER_FLOW_NAME)
    helper_g.param(INPUT_PARAM, "str")
    helper_g.ask(name="process", prompt=f"Process {INPUT_PLACEHOLDER}")
    helper_graph = helper_g.to_graph()

    g = GraphRecorder(MAIN_FLOW_NAME)
    step1 = g.ask(name="get_input", prompt="What input?")
    step2 = g.call(helper_graph, **{INPUT_PARAM: step1})
    g.done(step2)

    graph = g.to_graph()
    flow_call_nodes = [n for n in graph.nodes if n.op == OP_FLOW_CALL]
    assert len(flow_call_nodes) == 1
    assert flow_call_nodes[0].attributes[AGENT_NAME] == HELPER_FLOW_NAME
    assert flow_call_nodes[0].attributes[INPUT_NAMES] == [INPUT_PARAM]
    assert flow_call_nodes[0].attributes[ARGS][INPUT_PARAM] == INPUT_PLACEHOLDER


def test_call_rejects_unknown_argument_names():
    from apxm import GraphRecorder

    helper_g = GraphRecorder("helper_flow")
    helper_g.param("input", "str")
    helper_graph = helper_g.to_graph()

    g = GraphRecorder("main_flow")
    step1 = g.ask(name="get_input", prompt="What input?")

    with pytest.raises(TypeError, match="unknown argument"):
        g.call(helper_graph, topic=step1)
