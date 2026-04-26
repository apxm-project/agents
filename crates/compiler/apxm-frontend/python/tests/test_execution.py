"""Test execution result types and helpers."""

import json
from pathlib import Path
import pytest

from apxm.constants import (
    AIR_PAYLOAD,
    ARGS,
    ENV_APXM_BIN,
    MAX_SCHEMA_RETRIES,
    OUTPUT_SCHEMA,
    SESSION_ROOT,
    TOKEN_BUDGET,
)


def _assert_local_execute_cli_command(
    cmd: list[str],
    *,
    expected_binary: str,
    expected_arg: str,
    expected_session_root: str | None = None,
) -> None:
    import apxm.execution as execution_mod

    assert cmd[0] == expected_binary
    execute_index = cmd.index(execution_mod._CLI_EXECUTE_SUBCOMMAND)
    assert execution_mod._CLI_JSON_FLAG in cmd[:execute_index]
    assert Path(cmd[execute_index + 1]).suffix == ".air"
    assert expected_arg in cmd[execute_index + 2:]
    if expected_session_root is None:
        assert execution_mod._CLI_EMIT_SESSION_FLAG not in cmd
    else:
        session_flag_index = cmd.index(execution_mod._CLI_EMIT_SESSION_FLAG)
        assert cmd[session_flag_index + 1] == expected_session_root


def _assert_workflow_run_cli_command(
    cmd: list[str],
    *,
    expected_binary: str,
    expected_path: str,
    expected_args: dict[str, object] | None = None,
    expected_session_root: str | None = None,
) -> None:
    import apxm.execution as execution_mod

    assert cmd[0] == expected_binary
    workflow_index = cmd.index(execution_mod._CLI_WORKFLOW_SUBCOMMAND)
    assert execution_mod._CLI_JSON_FLAG in cmd[:workflow_index]
    assert cmd[workflow_index + 1] == execution_mod._CLI_RUN_SUBCOMMAND
    assert cmd[workflow_index + 2] == expected_path

    if expected_session_root is None:
        assert execution_mod._CLI_SESSION_ROOT_FLAG not in cmd
    else:
        session_flag_index = cmd.index(execution_mod._CLI_SESSION_ROOT_FLAG)
        assert cmd[session_flag_index + 1] == expected_session_root

    if expected_args is None:
        assert execution_mod._CLI_ARGS_JSON_FLAG not in cmd
    else:
        args_flag_index = cmd.index(execution_mod._CLI_ARGS_JSON_FLAG)
        assert json.loads(cmd[args_flag_index + 1]) == expected_args


def _toml_table(section: str, values: dict[str, object]) -> str:
    import apxm.config as config_mod

    lines = [f"[[{section}]]"]
    lines.extend(f"{key} = {config_mod._toml_value(value)}" for key, value in values.items())
    return "\n".join(lines)


def test_execution_result_from_response():
    """Test ExecutionResult.from_response() with a typical server response."""
    from apxm.execution import ExecutionResult

    data = {
        "content": "Hello, world!",
        "execution_id": "exec-123",
        "session_dir": "/tmp/apxm/sessions/test-session",
        "metrics_path": "/tmp/apxm/metrics.json",
        "profile_path": "/tmp/apxm/profile.json",
        "results": {"0": "Hello, world!"},
        "stats": {
            "executed_nodes": 3,
            "failed_nodes": 0,
            "duration_ms": 1234,
        },
        "llm_usage": {
            "input_tokens": 100,
            "output_tokens": 200,
            "total_requests": 2,
        },
    }

    result = ExecutionResult.from_response(data)

    assert result.content == "Hello, world!"
    assert result.execution_id == "exec-123"
    assert result.session_dir == "/tmp/apxm/sessions/test-session"
    assert result.metrics_path == "/tmp/apxm/metrics.json"
    assert result.profile_path == "/tmp/apxm/profile.json"
    assert result.results == {"0": "Hello, world!"}
    assert result.stats.executed_nodes == 3
    assert result.stats.failed_nodes == 0
    assert result.stats.duration_ms == 1234
    assert result.llm_usage.input_tokens == 100
    assert result.llm_usage.output_tokens == 200
    assert result.llm_usage.total_requests == 2


def test_execution_result_from_empty_response():
    """Test ExecutionResult.from_response() with minimal data."""
    from apxm.execution import ExecutionResult

    result = ExecutionResult.from_response({})

    assert result.content is None
    assert result.execution_id is None
    assert result.session_dir is None
    assert result.metrics_path is None
    assert result.profile_path is None
    assert result.results == {}
    assert result.stats.executed_nodes == 0
    assert result.llm_usage.input_tokens == 0


def test_execution_result_defaults():
    """Test ExecutionResult default values."""
    from apxm.execution import ExecutionResult

    result = ExecutionResult()

    assert result.content is None
    assert result.execution_id is None
    assert result.session_dir is None
    assert result.metrics_path is None
    assert result.profile_path is None
    assert result.results == {}
    assert result.stats.executed_nodes == 0
    assert result.llm_usage.total_requests == 0


def test_new_session_returns_string():
    """Test new_session() returns a valid session ID string."""
    from apxm.execution import new_session

    session = new_session()
    assert isinstance(session, str)
    assert session.startswith("s-")
    assert len(session) > 5


def test_new_session_unique():
    """Test new_session() returns unique IDs."""
    from apxm.execution import new_session

    sessions = {new_session() for _ in range(100)}
    assert len(sessions) == 100


def test_error_hierarchy():
    """Test error class hierarchy."""
    from apxm.errors import ApxmError, CompilationError, ExecutionError, ServerError

    assert issubclass(CompilationError, ApxmError)
    assert issubclass(ExecutionError, ApxmError)
    assert issubclass(ServerError, ApxmError)

    # All are also Exception subclasses
    assert issubclass(ApxmError, Exception)


def test_compiled_flow_build_request():
    """Test CompiledFlow._build_request() produces correct structure."""
    from apxm import ExecutionOptions, GraphRecorder
    from apxm.execution import CompiledFlow

    import apxm.config as config_mod

    g = GraphRecorder("test_flow")
    g.ask(name="step1", prompt="Do something")
    graph = g.to_graph()

    args = ("arg1", "arg2")
    session_id_field = config_mod.ExecutionOptions.__dataclass_fields__["session_id"].name
    session_id = "test-session"
    session_root = "/tmp/apxm/sessions"
    output_schema = {"type": "object"}
    max_schema_retries = 3
    flow = CompiledFlow(graph)
    request = flow._build_request(
        args,
        execution=ExecutionOptions(
            session_id=session_id,
            session_root=session_root,
            token_budget=256,
            output_schema=output_schema,
            max_schema_retries=max_schema_retries,
        ),
    )

    assert request[AIR_PAYLOAD].strip()
    assert request[ARGS] == list(args)
    assert request[session_id_field] == session_id
    assert request[SESSION_ROOT] == session_root
    assert request[TOKEN_BUDGET] == 256
    assert request[OUTPUT_SCHEMA] == output_schema
    assert request[MAX_SCHEMA_RETRIES] == max_schema_retries


def test_compiled_flow_build_request_no_session():
    """Test _build_request without session_id."""
    from apxm import GraphRecorder
    from apxm.execution import CompiledFlow

    g = GraphRecorder("test_flow")
    g.ask(name="step1", prompt="Do something")
    graph = g.to_graph()

    flow = CompiledFlow(graph)
    request = flow._build_request(())

    import apxm.config as config_mod

    session_id_field = config_mod.ExecutionOptions.__dataclass_fields__["session_id"].name
    assert session_id_field not in request
    assert request[ARGS] == []


def test_execution_options_render_local_cli_config():
    import apxm.config as config_mod
    from apxm import (
        ExecutionOptions,
        HookConfig,
        HookEvent,
        LoopGuardMiddlewareConfig,
        SearchDepth,
        SearchWebConfig,
        TimeoutMiddlewareConfig,
        ToolsConfig,
    )

    options = ExecutionOptions(
        hooks=[HookConfig(event=HookEvent.NODE_COMPLETE, command="echo {{node_id}}")],
        middlewares=[
            TimeoutMiddlewareConfig(default_timeout_ms=5000),
            LoopGuardMiddlewareConfig(max_repeats=2),
        ],
    )

    toml = options.config_toml()
    expected_hook = HookConfig(
        event=HookEvent.NODE_COMPLETE,
        command="echo {{node_id}}",
    ).to_toml_table()
    expected_timeout = TimeoutMiddlewareConfig(default_timeout_ms=5000).to_toml_table()
    expected_loop_guard = LoopGuardMiddlewareConfig(max_repeats=2).to_toml_table()

    assert toml == "\n\n".join(
        [
            _toml_table(config_mod._TOML_SECTION_HOOKS, expected_hook),
            _toml_table(config_mod._TOML_SECTION_MIDDLEWARES, expected_timeout),
            _toml_table(config_mod._TOML_SECTION_MIDDLEWARES, expected_loop_guard),
        ]
    ) + "\n"
    assert options.requires_local_cli() is True

    search_web = SearchWebConfig(search_depth=SearchDepth.ADVANCED)
    assert search_web.search_depth is SearchDepth.ADVANCED
    tools = ToolsConfig(search_web=search_web)
    assert tools.to_dict()["search_web"]["search_depth"] == SearchDepth.ADVANCED.value


def test_hook_event_members_are_stable_frontend_contract():
    from apxm import HookEvent

    assert list(HookEvent) == [
        HookEvent.GRAPH_START,
        HookEvent.GRAPH_END,
        HookEvent.NODE_START,
        HookEvent.NODE_COMPLETE,
        HookEvent.NODE_ERROR,
        HookEvent.TOOL_START,
        HookEvent.TOOL_END,
    ]


def test_execution_options_render_all_hook_events_in_local_cli_config():
    import apxm.config as config_mod
    from apxm import ExecutionOptions, HookConfig, HookEvent

    hooks = [
        HookConfig(event=event, command=f"emit-{event.value}")
        for event in HookEvent
    ]

    toml = ExecutionOptions(hooks=hooks).config_toml()

    assert toml == "\n\n".join(
        _toml_table(config_mod._TOML_SECTION_HOOKS, hook.to_toml_table())
        for hook in hooks
    ) + "\n"


def test_search_web_config_rejects_non_enum_search_depth():
    from apxm import SearchWebConfig

    with pytest.raises(TypeError, match="search_depth must be a SearchDepth"):
        SearchWebConfig(search_depth=object())


def test_execution_options_session_root_does_not_force_local_cli():
    from apxm import ExecutionOptions

    session_root = ".apxm/sessions"
    options = ExecutionOptions(session_root=session_root)

    assert options.requires_local_cli() is False
    assert options.server_request_fields()[SESSION_ROOT] == session_root


def test_hook_config_rejects_non_enum_events():
    from apxm import HookConfig

    with pytest.raises(TypeError, match="HookConfig.event must be a HookEvent"):
        HookConfig(event=object(), command="echo nope")


def test_run_workflow_file_uses_json_contract(monkeypatch):
    import subprocess
    import json as pyjson
    from apxm.execution import WorkflowRunResult, run_workflow_file

    apxm_bin = str(Path("/tmp/test-bin") / "apxm")
    monkeypatch.setattr("apxm.execution._find_apxm_binary", lambda: apxm_bin)

    def fake_run(cmd, capture_output, text, check, env):
        _assert_workflow_run_cli_command(
            cmd,
            expected_binary=apxm_bin,
            expected_path="demo.apxmw",
            expected_args={"topic": "middleware"},
            expected_session_root=".apxm/sessions",
        )
        assert capture_output is True
        assert text is True
        assert check is False
        assert isinstance(env, dict)
        return subprocess.CompletedProcess(
            cmd,
            0,
            stdout=pyjson.dumps(
                {
                    "workflow_name": "demo",
                    "status": "Success",
                    "duration_ms": 123,
                    "session_dir": "/tmp/apxm/workflow-demo",
                    "step_results": {},
                    "output": "done",
                }
            ),
            stderr="",
        )

    monkeypatch.setattr("subprocess.run", fake_run)

    result = run_workflow_file(
        "demo.apxmw",
        args={"topic": "middleware"},
        session_root=".apxm/sessions",
    )

    assert isinstance(result, WorkflowRunResult)
    assert result.session_dir == "/tmp/apxm/workflow-demo"
    assert '"workflow_name": "demo"' in result.stdout


def test_run_workflow_file_omits_session_root_when_unset(monkeypatch):
    import subprocess
    import json as pyjson
    from apxm.execution import run_workflow_file

    apxm_bin = str(Path("/tmp/test-bin") / "apxm")
    monkeypatch.setattr("apxm.execution._find_apxm_binary", lambda: apxm_bin)

    def fake_run(cmd, capture_output, text, check, env):
        _assert_workflow_run_cli_command(
            cmd,
            expected_binary=apxm_bin,
            expected_path="demo.apxmw",
            expected_args={"topic": "middleware"},
        )
        assert isinstance(env, dict)
        return subprocess.CompletedProcess(
            cmd,
            0,
            stdout=pyjson.dumps(
                {
                    "workflow_name": "demo",
                    "status": "Success",
                    "duration_ms": 123,
                    "session_dir": "/tmp/apxm/workflow-demo",
                    "step_results": {},
                    "output": None,
                }
            ),
            stderr="",
        )

    monkeypatch.setattr("subprocess.run", fake_run)

    result = run_workflow_file("demo.apxmw", args={"topic": "middleware"})
    assert result.session_dir == "/tmp/apxm/workflow-demo"


def test_compiled_flow_local_fallback_uses_execute_json_contract(monkeypatch):
    import subprocess
    import json as pyjson
    from apxm import ExecutionOptions, GraphRecorder, HookConfig, HookEvent
    from apxm.execution import CompiledFlow, ExecutionResult

    apxm_bin = str(Path("/tmp/test-bin") / "apxm")
    monkeypatch.setattr("apxm.execution._find_apxm_binary", lambda: apxm_bin)

    g = GraphRecorder("local_execute")
    g.ask(name="step1", prompt="Do something")
    flow = CompiledFlow(g.to_graph())

    def fake_run(cmd, capture_output, text, check, env):
        _assert_local_execute_cli_command(cmd, expected_binary=apxm_bin, expected_arg="hello")
        assert capture_output is True
        assert text is True
        assert check is False
        assert isinstance(env, dict)
        return subprocess.CompletedProcess(
            cmd,
            0,
            stdout=pyjson.dumps(
                {
                    "content": "done",
                    "execution_id": "exec-123",
                    "session_dir": "/tmp/apxm/sessions/local-execute",
                    "metrics_path": "/tmp/apxm/metrics.json",
                    "profile_path": "/tmp/apxm/profile.json",
                    "results": {"0": "done"},
                    "stats": {
                        "executed_nodes": 1,
                        "failed_nodes": 0,
                        "duration_ms": 42,
                    },
                    "llm_usage": {
                        "input_tokens": 11,
                        "output_tokens": 7,
                        "total_requests": 1,
                    },
                }
            ),
            stderr="Session: /tmp/should-not-be-parsed\n",
        )

    monkeypatch.setattr("subprocess.run", fake_run)

    result = flow.run_sync(
        "hello",
        execution=ExecutionOptions(
            hooks=[HookConfig(event=HookEvent.NODE_COMPLETE, command="echo {{node_id}}")]
        ),
    )

    assert isinstance(result, ExecutionResult)
    assert result.content == "done"
    assert result.execution_id == "exec-123"
    assert result.session_dir == "/tmp/apxm/sessions/local-execute"
    assert result.metrics_path == "/tmp/apxm/metrics.json"
    assert result.profile_path == "/tmp/apxm/profile.json"
    assert result.stats.executed_nodes == 1


def test_compiled_flow_local_fallback_forwards_session_root(monkeypatch):
    import subprocess
    import json as pyjson
    from apxm import ExecutionOptions, GraphRecorder, HookConfig, HookEvent
    from apxm.execution import CompiledFlow

    apxm_bin = str(Path("/tmp/test-bin") / "apxm")
    monkeypatch.setattr("apxm.execution._find_apxm_binary", lambda: apxm_bin)

    g = GraphRecorder("local_execute_session_root")
    g.ask(name="step1", prompt="Do something")
    flow = CompiledFlow(g.to_graph())

    def fake_run(cmd, capture_output, text, check, env):
        _assert_local_execute_cli_command(
            cmd,
            expected_binary=apxm_bin,
            expected_arg="hello",
            expected_session_root=".apxm/sessions",
        )
        assert isinstance(env, dict)
        return subprocess.CompletedProcess(
            cmd,
            0,
            stdout=pyjson.dumps(
                {
                    "content": "done",
                    "session_dir": "/tmp/apxm/sessions/local-execute",
                    "results": {"0": "done"},
                    "stats": {
                        "executed_nodes": 1,
                        "failed_nodes": 0,
                        "duration_ms": 42,
                    },
                    "llm_usage": {
                        "input_tokens": 0,
                        "output_tokens": 0,
                        "total_requests": 0,
                    },
                }
            ),
            stderr="",
        )

    monkeypatch.setattr("subprocess.run", fake_run)

    result = flow.run_sync(
        "hello",
        execution=ExecutionOptions(
            session_root=".apxm/sessions",
            hooks=[HookConfig(event=HookEvent.NODE_COMPLETE, command="echo {{node_id}}")],
        ),
    )

    assert result.session_dir == "/tmp/apxm/sessions/local-execute"


def test_execution_result_rejects_non_string_session_dir():
    from apxm.execution import ExecutionResult

    with pytest.raises(TypeError, match="session_dir must be a string or null"):
        ExecutionResult.from_response({"session_dir": ["bad"]})


def test_execution_result_rejects_non_string_execution_paths():
    from apxm.execution import ExecutionResult

    with pytest.raises(TypeError, match="execution_id must be a string or null"):
        ExecutionResult.from_response({"execution_id": ["bad"]})
    with pytest.raises(TypeError, match="metrics_path must be a string or null"):
        ExecutionResult.from_response({"metrics_path": ["bad"]})
    with pytest.raises(TypeError, match="profile_path must be a string or null"):
        ExecutionResult.from_response({"profile_path": ["bad"]})


def test_compiled_flow_local_fallback_rejects_non_object_json(monkeypatch):
    import subprocess
    from apxm import ExecutionOptions, GraphRecorder, HookConfig, HookEvent
    from apxm.errors import ExecutionError
    from apxm.execution import CompiledFlow

    monkeypatch.setattr(
        "apxm.execution._find_apxm_binary",
        lambda: str(Path("/tmp/test-bin") / "apxm"),
    )

    g = GraphRecorder("local_execute_non_object")
    g.ask(name="step1", prompt="Do something")
    flow = CompiledFlow(g.to_graph())

    def fake_run(cmd, capture_output, text, check, env):
        assert isinstance(env, dict)
        return subprocess.CompletedProcess(cmd, 0, stdout="[]", stderr="")

    monkeypatch.setattr("subprocess.run", fake_run)

    with pytest.raises(ExecutionError, match="non-object payload"):
        flow.run_sync(
            "hello",
            execution=ExecutionOptions(
                hooks=[HookConfig(event=HookEvent.NODE_COMPLETE, command="echo {{node_id}}")]
            ),
        )


def test_cli_error_message_prefers_json_error_payload():
    from apxm.execution import _cli_error_message

    assert (
        _cli_error_message('{"error":"structured failure"}', "ignored", "fallback")
        == "structured failure"
    )


def test_find_apxm_binary_prefers_explicit_env(monkeypatch, tmp_path):
    from apxm.execution import _find_apxm_binary

    custom = tmp_path / "apxm-custom"
    custom.write_text("", encoding="utf-8")
    monkeypatch.setenv(ENV_APXM_BIN, str(custom))
    monkeypatch.setattr("shutil.which", lambda _name: None)

    assert _find_apxm_binary() == str(custom)


def test_find_apxm_binary_accepts_explicit_path_command(monkeypatch, tmp_path):
    from apxm.execution import _find_apxm_binary

    command = tmp_path / "dekk"
    command.write_text("", encoding="utf-8")
    monkeypatch.setenv(ENV_APXM_BIN, "dekk")
    monkeypatch.setattr("shutil.which", lambda name: str(command) if name == "dekk" else None)

    assert _find_apxm_binary() == str(command)


def test_find_apxm_binary_falls_back_to_repo_checkout(monkeypatch, tmp_path):
    from apxm.execution import _find_apxm_binary

    binary = tmp_path / "target" / "debug" / "apxm"
    binary.parent.mkdir(parents=True)
    binary.write_text("", encoding="utf-8")

    monkeypatch.delenv(ENV_APXM_BIN, raising=False)
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr("shutil.which", lambda _name: None)

    assert _find_apxm_binary() == str(binary)


def test_find_apxm_binary_prefers_release_checkout_binary(monkeypatch, tmp_path):
    from apxm.execution import _find_apxm_binary

    release_binary = tmp_path / "target" / "release" / "apxm"
    debug_binary = tmp_path / "target" / "debug" / "apxm"
    release_binary.parent.mkdir(parents=True)
    debug_binary.parent.mkdir(parents=True)
    release_binary.write_text("", encoding="utf-8")
    debug_binary.write_text("", encoding="utf-8")

    monkeypatch.delenv(ENV_APXM_BIN, raising=False)
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr("shutil.which", lambda _name: None)

    assert _find_apxm_binary() == str(release_binary)


def test_subprocess_env_for_checkout_binary_includes_lib_dir(tmp_path):
    from apxm.execution import _subprocess_env_for_apxm

    binary = tmp_path / "target" / "debug" / "apxm"
    lib_dir = tmp_path / "target" / "debug" / "lib"
    lib_dir.mkdir(parents=True)
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text("", encoding="utf-8")

    env = _subprocess_env_for_apxm(str(binary))
    assert env["LD_LIBRARY_PATH"].split(":")[0] == str(lib_dir)


def test_subprocess_env_for_release_checkout_binary_includes_lib_dir(tmp_path):
    from apxm.execution import _subprocess_env_for_apxm

    binary = tmp_path / "target" / "release" / "apxm"
    lib_dir = tmp_path / "target" / "release" / "lib"
    lib_dir.mkdir(parents=True)
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text("", encoding="utf-8")

    env = _subprocess_env_for_apxm(str(binary))
    assert env["LD_LIBRARY_PATH"].split(":")[0] == str(lib_dir)


def test_run_wrapper():
    """Test apxm.run() wrapper."""
    import asyncio
    from apxm.execution import run

    async def simple():
        return 42

    result = run(simple())
    assert result == 42


def test_compiled_flow_save_load_roundtrip():
    """Test save/load roundtrip preserves canonical AIR."""
    import tempfile
    from pathlib import Path
    from apxm import GraphRecorder
    from apxm.execution import CompiledFlow

    g = GraphRecorder("roundtrip_test")
    g.param("topic", "str")
    g.ask(name="step1", prompt="Research {0}")
    graph = g.to_graph()

    flow = CompiledFlow(graph)

    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
        tmp_path = tmp.name

    try:
        flow.save(tmp_path)
        loaded = CompiledFlow.load(tmp_path)
        saved = json.loads(Path(tmp_path).read_text(encoding="utf-8"))
        assert loaded._air_text == saved[AIR_PAYLOAD]
        assert loaded._air_text == graph.to_air()
    finally:
        Path(tmp_path).unlink(missing_ok=True)
