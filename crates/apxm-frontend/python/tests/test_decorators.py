"""Test @compile decorator and parameter derivation."""

import pytest


def test_compile_decorator_basic():
    """Test basic @compile decorator."""
    from apxm import GraphRecorder, compile

    @compile()
    def simple_workflow(g: GraphRecorder):
        g.ask("step1", "Do something")

    # Verify the decorated function has the right metadata
    assert hasattr(simple_workflow, "_graph")
    graph = simple_workflow._graph

    assert graph.name == "simple_workflow"
    assert len(graph.nodes) == 1


def test_compile_with_typed_params():
    """Test @compile with typed parameters that get derived automatically."""
    from apxm import GraphRecorder, compile

    @compile()
    def research_workflow(g: GraphRecorder, topic: str):
        g.ask("research", f"Research {{topic}}")

    graph = research_workflow._graph

    # Should have automatically derived a parameter named 'topic' of type 'str'
    assert len(graph.parameters) == 1
    assert graph.parameters[0].name == "topic"
    assert graph.parameters[0].type_name == "str"

    # The template should use named placeholder
    node = graph.nodes[0]
    # After conversion, named placeholders become positional
    assert "{0}" in node.attributes["template_str"]


def test_compile_with_multiple_params():
    """Test @compile with multiple typed parameters."""
    from apxm import GraphRecorder, compile

    @compile()
    def multi_param_workflow(g: GraphRecorder, topic: str, depth: int, threshold: float):
        g.ask("process", f"Process {{topic}} at depth {{depth}} with threshold {{threshold}}")

    graph = multi_param_workflow._graph

    assert len(graph.parameters) == 3

    param_map = {p.name: p.type_name for p in graph.parameters}
    assert param_map["topic"] == "str"
    assert param_map["depth"] == "int"
    assert param_map["threshold"] == "float"


def test_named_placeholder_conversion():
    """Test that named placeholders {topic} are converted to positional {0}."""
    from apxm import GraphRecorder, compile

    @compile()
    def placeholder_workflow(g: GraphRecorder, subject: str, action: str):
        g.ask("task", f"The {{subject}} will {{action}}")

    graph = placeholder_workflow._graph
    node = graph.nodes[0]
    template = node.attributes["template_str"]

    # Named placeholders should be converted to positional
    assert "{0}" in template
    assert "{1}" in template
    assert "{subject}" not in template
    assert "{action}" not in template


def test_compile_with_team_sugar():
    """Test @compile with team sugar."""
    from apxm import GraphRecorder, compile

    @compile()
    def team_workflow(g: GraphRecorder, task: str):
        team = g.team("workers")
        alice = team.add("alice", profile="claude")
        bob = team.add("bob", profile="codex")

        alice.ask(f"Alice: do {{task}}")
        bob.ask(f"Bob: do {{task}}")

        result = team.merge()

    graph = team_workflow._graph

    # Check structure
    spawn_nodes = [n for n in graph.nodes if n.op == "SPAWN_AGENT"]
    comm_nodes = [n for n in graph.nodes if n.op == "COMMUNICATE"]
    merge_nodes = [n for n in graph.nodes if n.op == "MERGE"]

    assert len(spawn_nodes) == 2
    assert len(comm_nodes) == 2
    assert len(merge_nodes) == 1
