"""Test @compile decorator and parameter derivation."""

import pytest


def test_compile_decorator_basic():
    """Test basic @compile decorator."""
    from apxm import GraphRecorder, compile

    @compile()
    def simple_workflow(g: GraphRecorder):
        g.ask(name="step1", prompt="Do something")

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
        g.ask(name="research", prompt=f"Research {{topic}}")

    graph = research_workflow._graph

    # Should have automatically derived a parameter named 'topic' of type 'str'
    assert len(graph.parameters) == 1
    assert graph.parameters[0].name == "topic"
    assert graph.parameters[0].type_name == "str"

    # The template should preserve the named placeholder verbatim
    node = graph.nodes[0]
    assert "{topic}" in node.attributes["template_str"]


def test_compile_with_multiple_params():
    """Test @compile with multiple typed parameters."""
    from apxm import GraphRecorder, compile

    @compile()
    def multi_param_workflow(g: GraphRecorder, topic: str, depth: int, threshold: float):
        g.ask(name="process", prompt=f"Process {{topic}} at depth {{depth}} with threshold {{threshold}}")

    graph = multi_param_workflow._graph

    assert len(graph.parameters) == 3

    param_map = {p.name: p.type_name for p in graph.parameters}
    assert param_map["topic"] == "str"
    assert param_map["depth"] == "int"
    assert param_map["threshold"] == "float"


def test_named_placeholders_are_preserved():
    """Named placeholders {subject}/{action} survive emission verbatim."""
    from apxm import GraphRecorder, compile

    @compile()
    def placeholder_workflow(g: GraphRecorder, subject: str, action: str):
        g.ask(name="task", prompt=f"The {{subject}} will {{action}}")

    graph = placeholder_workflow._graph
    node = graph.nodes[0]
    template = node.attributes["template_str"]

    # Named placeholders are kept; the validator resolves them against
    # input_names / module parameters at compile time.
    assert "{subject}" in template
    assert "{action}" in template
    assert "{0}" not in template
    assert "{1}" not in template


def test_compile_default_provider_and_backend_stamping():
    """Test @compile(default_provider, default_backend) stamps LLM nodes."""
    from apxm import GraphRecorder, compile
    from apxm._generated import constants as gen_keys
    from apxm._generated.providers import VLLM

    @compile(default_provider=VLLM, default_backend="vllm-bench")
    def vllm_workflow(g: GraphRecorder, topic: str):
        g.ask(name="step1", prompt=f"Research {{topic}}")
        g.ask(name="step2", prompt=f"Summarize {{topic}}")

    graph = vllm_workflow._graph
    from apxm.constants import LLM_OPS

    llm_nodes = [n for n in graph.nodes if n.op in LLM_OPS]
    assert len(llm_nodes) == 2
    for node in llm_nodes:
        assert node.attributes[gen_keys.PROVIDER] == "vllm"
        assert node.attributes[gen_keys.BACKEND] == "vllm-bench"


def test_compile_default_provider_does_not_override_per_node():
    """Per-node provider/backend wins over @compile() defaults."""
    from apxm import GraphRecorder, compile
    from apxm._generated import constants as gen_keys
    from apxm._generated.providers import VLLM

    @compile(default_provider=VLLM, default_backend="vllm-bench")
    def mixed_workflow(g: GraphRecorder):
        g.ask(name="default_routed", prompt="hello")
        g.ask(name="explicit_routed", prompt="hi", provider="openai", backend="openai-prod")

    graph = mixed_workflow._graph
    by_name = {n.name: n for n in graph.nodes}

    default_node = by_name["default_routed"]
    assert default_node.attributes[gen_keys.PROVIDER] == "vllm"
    assert default_node.attributes[gen_keys.BACKEND] == "vllm-bench"

    explicit_node = by_name["explicit_routed"]
    assert explicit_node.attributes[gen_keys.PROVIDER] == "openai"
    assert explicit_node.attributes[gen_keys.BACKEND] == "openai-prod"


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
