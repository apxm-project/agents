from apxm import (
    AgentConfig,
    Capability,
    DependencyType,
    GraphEdge,
    GraphRecorder,
    HookMode,
    LifecycleEvent,
    NodePolicy,
    ToolGroup,
    hook,
)
from apxm.constants import (
    CAPABILITY,
    CAPABILITY_SEARCH_SKILLS,
    DEPENDENCY_CONTROL,
    DEPENDENCY_DATA,
    INPUT_NAMES,
    PARAMS_JSON,
    CAPABILITY_GROUPS,
    TOOL_GROUP_FILE_READ,
    TOOL_GROUP_WEB,
)
from apxm.ir import validate_against_apxm


def test_dependency_type_normalizes_to_wire_value():
    edge = GraphEdge(from_id=1, to_id=2, dependency=DependencyType.CONTROL)

    assert edge.dependency == DEPENDENCY_CONTROL
    assert edge.to_dict()["dependency"] == DEPENDENCY_CONTROL


def test_graph_recorder_keeps_entry_metadata_explicit():
    assert GraphRecorder("entry_metadata").to_graph().metadata == {}


def test_graph_recorder_accepts_dependency_type():
    g = GraphRecorder("typed_edges")
    first = g.print(name="first", message="first")
    second = g.print(name="second", message="second")

    g.add_edge(first, second, dependency=DependencyType.CONTROL)

    graph = g.to_graph()
    assert graph.edges[0].dependency == DEPENDENCY_CONTROL


def test_graph_edge_default_uses_data_dependency():
    edge = GraphEdge(from_id=1, to_id=2)

    assert edge.dependency == DEPENDENCY_DATA


def test_lifecycle_event_and_hook_mode_normalize_to_wire_values():
    @hook(on=LifecycleEvent.PRE_CAP, mode=HookMode.GATE)
    def guard(ctx, call):
        return ctx.allow()

    assert guard.event == LifecycleEvent.PRE_CAP.value
    assert guard.mode == HookMode.GATE.value


def test_register_hook_accepts_lifecycle_enums():
    @hook(on=LifecycleEvent.PRE_ASK)
    def before_ask(ctx):
        return None

    g = GraphRecorder("typed_hooks")
    g.register_hook(event=LifecycleEvent.PRE_ASK, mode=HookMode.OBSERVE, fn=before_ask)

    result = validate_against_apxm(g.to_graph())
    assert result.valid, result.errors


def test_skill_search_lowers_query_to_search_skills_request():
    g = GraphRecorder("skill_search_contract")

    node = g.skill_search(query="review module boundaries")

    graph = g.to_graph()
    attrs = graph.nodes[node._node_id - 1].attributes
    assert attrs[CAPABILITY] == CAPABILITY_SEARCH_SKILLS
    assert attrs[PARAMS_JSON] == '{"request": "review module boundaries"}'


def test_tool_group_enum_normalizes_to_wire_value():
    g = GraphRecorder("tool_group_contract")

    node = g.ask(prompt="hello", capability_groups=[ToolGroup.WEB])

    graph = g.to_graph()
    attrs = graph.nodes[node._node_id - 1].attributes
    assert attrs[CAPABILITY_GROUPS] == [TOOL_GROUP_WEB]


def test_node_policy_accepts_tool_group_enums():
    policy = NodePolicy(capability_groups=[ToolGroup.WEB, ToolGroup.FILE_READ])

    attrs = policy.to_node_attributes()

    assert attrs[CAPABILITY_GROUPS] == [TOOL_GROUP_WEB, TOOL_GROUP_FILE_READ]


def test_agent_config_accepts_tool_group_enums():
    agent = AgentConfig(name="typed_agent", capability_groups=[ToolGroup.WEB])

    attrs = agent.to_node_attributes()

    assert attrs[CAPABILITY_GROUPS] == [TOOL_GROUP_WEB]


def test_invoke_accepts_capability_enum():
    g = GraphRecorder("typed_capability")

    node = g.invoke(capability=Capability.SEARCH_SKILLS, params={"request": "review"})

    graph = g.to_graph()
    attrs = graph.nodes[node._node_id - 1].attributes
    assert attrs[CAPABILITY] == CAPABILITY_SEARCH_SKILLS


def test_dotted_flow_parameter_placeholder_does_not_autowire():
    g = GraphRecorder("dotted_param")
    g.param("payload", "json")

    node = g.ask(prompt="{payload.event.id}")

    graph = g.to_graph()
    attrs = graph.nodes[node._node_id - 1].attributes
    assert INPUT_NAMES not in attrs
    assert graph.edges == []


def test_dotted_node_placeholder_autowires_root_input():
    g = GraphRecorder("dotted_node_ref")
    message = g.print(name="message", message='{"text":"hello"}')

    node = g.ask(prompt="{message.text}")

    graph = g.to_graph()
    attrs = graph.nodes[node._node_id - 1].attributes
    assert attrs[INPUT_NAMES] == ["message"]
    assert len(graph.edges) == 1
    assert graph.edges[0].from_id == message._node_id
    assert graph.edges[0].to_id == node._node_id
