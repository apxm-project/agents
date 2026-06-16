from apxm import DependencyType, GraphEdge, GraphRecorder, HookMode, LifecycleEvent, hook
from apxm.constants import DEPENDENCY_CONTROL, DEPENDENCY_DATA, PARAMS_JSON
from apxm.ir import validate_against_apxm


def test_dependency_type_normalizes_to_wire_value():
    edge = GraphEdge(from_id=1, to_id=2, dependency=DependencyType.CONTROL)

    assert edge.dependency == DEPENDENCY_CONTROL
    assert edge.to_dict()["dependency"] == DEPENDENCY_CONTROL


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
    @hook(on=LifecycleEvent.PRE_TOOL, mode=HookMode.GATE)
    def guard(ctx, call):
        return ctx.allow()

    assert guard.event == LifecycleEvent.PRE_TOOL.value
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
    assert attrs[PARAMS_JSON] == '{"request": "review module boundaries"}'
