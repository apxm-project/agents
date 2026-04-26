"""Test @compile decorator and parameter derivation."""

import pytest

from apxm.constants import (
    ENV_APXM_CONFIG,
    ENV_APXM_EMIT_AIR,
    ENV_FLAG_ENABLED,
    OP_COMMUNICATE,
    OP_MERGE,
    OP_SPAWN_AGENT,
    TEMPLATE_STR,
    TOKEN_BUDGET,
    TOOL_GROUPS,
    TOOLS_ENABLED,
)
from .mocks import MOCK_AGENT_PROFILE, MOCK_AGENT_PROFILE_ALT

WEB_TOOL_GROUP = "web"
CAPTURED_RUN_ARGS = "captured_run_args"


def _write_backend_config(path):
    path.write_text(
        """
[[backends]]
name = "vllm-bench"
type = "local"
protocol = "vllm"
endpoint = "http://localhost:8915/v1"

[backends.headers]

[[backends.models]]
id = "bench-model"
aliases = ["bench"]

[[backends]]
name = "openai-prod"
type = "cloud"
protocol = "openai"
api_key = "env:OPENAI_API_KEY"

[backends.headers]

[[backends.models]]
id = "gpt-route"
aliases = ["default"]
""",
        encoding="utf-8",
    )


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


def test_compiled_flow_emits_air_when_requested(monkeypatch, capsys):
    """Python graph files emit AIR for the Dekk compiler driver."""
    from apxm import GraphRecorder, compile, run

    @compile()
    def emit_workflow(g: GraphRecorder):
        result = g.print(name="result", message="ok")
        g.done(result)

    monkeypatch.setenv(ENV_APXM_EMIT_AIR, ENV_FLAG_ENABLED)
    result = run(emit_workflow())
    captured = capsys.readouterr()

    assert result.content == ""
    assert "module" in captured.out


def test_compiled_function_to_air_includes_python_tool_sidecar():
    """Compiled graph AIR includes Python tool metadata when tools are registered."""
    from apxm import GraphRecorder, compile, tool
    from apxm.constants import PYTHON_HANDLER_ID, PYTHON_TOOL_MANIFEST_MODULE
    from apxm.constants import PYTHON_TOOLS_AIR_COMMENT_PREFIX

    @tool
    def fixture_tool() -> str:
        return "fixture"

    @compile()
    def tool_workflow(g: GraphRecorder):
        value = g.invoke_tool(fixture_tool, name="Fixture")
        g.print(name="Print", message="{value}")

    air = tool_workflow.to_air()

    assert air.startswith(PYTHON_TOOLS_AIR_COMMENT_PREFIX)
    assert PYTHON_HANDLER_ID in air
    assert PYTHON_TOOL_MANIFEST_MODULE in air


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
    assert "{topic}" in node.attributes[TEMPLATE_STR]


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
    template = node.attributes[TEMPLATE_STR]

    # Named placeholders are kept; the validator resolves them against
    # input_names / module parameters at compile time.
    assert "{subject}" in template
    assert "{action}" in template
    assert "{0}" not in template
    assert "{1}" not in template


def test_compile_default_provider_and_backend_stamping(tmp_path, monkeypatch):
    """Test @compile(default_provider, default_backend) stamps LLM nodes."""
    from apxm import GraphRecorder, compile
    from apxm._generated import constants as gen_keys
    from apxm._generated.providers import VLLM
    from apxm.backends import select_backend

    config = tmp_path / "config.toml"
    _write_backend_config(config)
    monkeypatch.setenv(ENV_APXM_CONFIG, str(config))
    route = select_backend(protocol=VLLM.protocol, alias="bench")

    @compile(default_provider=VLLM, default_route=route)
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
        assert node.attributes[gen_keys.MODEL] == "bench-model"


def test_compile_default_provider_does_not_override_per_node(tmp_path, monkeypatch):
    """Per-node provider/backend wins over @compile() defaults."""
    from apxm import GraphRecorder, compile
    from apxm._generated import constants as gen_keys
    from apxm._generated.providers import OPENAI, VLLM
    from apxm.backends import select_backend

    config = tmp_path / "config.toml"
    _write_backend_config(config)
    monkeypatch.setenv(ENV_APXM_CONFIG, str(config))
    default_route = select_backend(protocol=VLLM.protocol, alias="bench")
    explicit_route = select_backend(protocol=OPENAI.protocol, alias="default")

    @compile(default_provider=VLLM, default_route=default_route)
    def mixed_workflow(g: GraphRecorder):
        g.ask(name="default_routed", prompt="hello")
        g.ask(
            name="explicit_routed",
            prompt="hi",
            provider=OPENAI,
            route=explicit_route,
        )

    graph = mixed_workflow._graph
    by_name = {n.name: n for n in graph.nodes}

    default_node = by_name["default_routed"]
    assert default_node.attributes[gen_keys.PROVIDER] == "vllm"
    assert default_node.attributes[gen_keys.BACKEND] == "vllm-bench"
    assert default_node.attributes[gen_keys.MODEL] == "bench-model"

    explicit_node = by_name["explicit_routed"]
    assert explicit_node.attributes[gen_keys.PROVIDER] == "openai"
    assert explicit_node.attributes[gen_keys.BACKEND] == "openai-prod"
    assert explicit_node.attributes[gen_keys.MODEL] == "gpt-route"


def test_compile_default_policy_stamps_nodes():
    from apxm import GraphRecorder, NodePolicy, compile

    @compile(default_policy=NodePolicy(tool_groups=[WEB_TOOL_GROUP], token_budget=512))
    def policy_workflow(g: GraphRecorder):
        g.ask(name="step1", prompt="Research")

    node = policy_workflow._graph.nodes[0]
    assert node.attributes[TOOL_GROUPS] == [WEB_TOOL_GROUP]
    assert node.attributes[TOOLS_ENABLED] is True
    assert node.attributes[TOKEN_BUDGET] == 512


def test_compiled_function_run_sync_forwards_execution_options(monkeypatch):
    from apxm import ExecutionOptions, GraphRecorder, HookConfig, HookEvent, compile

    @compile()
    def flow(g: GraphRecorder):
        g.ask(name="step1", prompt="Research")

    captured: dict[str, object] = {}

    class FakeCompiledFlow:
        def run_sync(self, *args, session_id=None, execution=None):
            captured[CAPTURED_RUN_ARGS] = args
            captured["session_id"] = session_id
            captured["execution"] = execution
            return "ok"

    monkeypatch.setattr(flow, "_compiled_flow", FakeCompiledFlow())
    execution = ExecutionOptions(
        hooks=[HookConfig(event=HookEvent.NODE_COMPLETE, command="echo {{node_id}}")]
    )

    result = flow.run_sync(execution=execution)

    assert result == "ok"
    assert captured[CAPTURED_RUN_ARGS] == ()
    assert captured["session_id"] is None
    assert captured["execution"] is execution


def test_compile_with_team_sugar():
    """Test @compile with team sugar."""
    from apxm import GraphRecorder, compile

    @compile()
    def team_workflow(g: GraphRecorder, task: str):
        team = g.team("workers")
        alice = team.add("alice", profile=MOCK_AGENT_PROFILE)
        bob = team.add("bob", profile=MOCK_AGENT_PROFILE_ALT)

        alice.ask(f"Alice: do {{task}}")
        bob.ask(f"Bob: do {{task}}")

        result = team.merge()

    graph = team_workflow._graph

    # Check structure
    spawn_nodes = [n for n in graph.nodes if n.op == OP_SPAWN_AGENT]
    comm_nodes = [n for n in graph.nodes if n.op == OP_COMMUNICATE]
    merge_nodes = [n for n in graph.nodes if n.op == OP_MERGE]

    assert len(spawn_nodes) == 2
    assert len(comm_nodes) == 2
    assert len(merge_nodes) == 1
