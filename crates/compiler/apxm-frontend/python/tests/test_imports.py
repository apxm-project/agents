"""Test that generated imports work correctly."""

def test_import_constants():
    """Verify constants can be imported from _generated."""
    import apxm.constants as public_constants
    from apxm import WorkflowTargetKind
    from apxm._generated.constants import (
        AGENT_NAME,
        AWAIT_RESULT,
        MODEL,
        SESSION_ROOT,
        TARGET_KIND,
        TEMPLATE_STR,
        WORKFLOW_TARGET_KIND_GRAPH_PATH,
        WORKFLOW_SPAWN_PATH_TARGET_KINDS,
    )
    assert MODEL == public_constants.MODEL
    assert AGENT_NAME == public_constants.AGENT_NAME
    assert TEMPLATE_STR == public_constants.TEMPLATE_STR
    assert TARGET_KIND == public_constants.TARGET_KIND
    assert SESSION_ROOT == public_constants.SESSION_ROOT
    assert AWAIT_RESULT == public_constants.AWAIT_RESULT
    assert WORKFLOW_TARGET_KIND_GRAPH_PATH == WorkflowTargetKind.GRAPH_PATH.value
    assert WORKFLOW_TARGET_KIND_GRAPH_PATH in WORKFLOW_SPAWN_PATH_TARGET_KINDS


def test_import_operations():
    """Verify operations can be imported from _generated."""
    from apxm.constants import OP_ASK, OP_SPAWN_AGENT, OP_THINK, OP_WORKFLOW_SPAWN
    from apxm._generated.operations import ASK, SPAWN_AGENT, THINK, WORKFLOW_SPAWN
    assert ASK.op == OP_ASK
    assert THINK.op == OP_THINK
    assert SPAWN_AGENT.op == OP_SPAWN_AGENT
    assert WORKFLOW_SPAWN.op == OP_WORKFLOW_SPAWN


def test_import_agents():
    """Verify agents can be imported from _generated."""
    from apxm._generated.agents import ALL_AGENTS, claude
    assert claude in ALL_AGENTS


def test_graph_imports():
    """Verify graph module exports work."""
    from apxm import (
        GraphRecorder,
        NodeRef,
        ApxmGraph,
        WorkflowTargetKind,
        compile,
        AgentHandle,
        BackendRoute,
        Team,
        agent_cwd,
        find_repo_root,
        list_backends,
        local_apxm_path,
        repo_path,
        select_backend,
    )
    assert GraphRecorder is not None
    assert NodeRef is not None
    assert ApxmGraph is not None
    assert WorkflowTargetKind.GRAPH_PATH.value is not None
    assert compile is not None
    assert AgentHandle is not None
    assert BackendRoute is not None
    assert Team is not None
    assert callable(agent_cwd)
    assert callable(find_repo_root)
    assert callable(local_apxm_path)
    assert callable(repo_path)
    assert callable(list_backends)
    assert callable(select_backend)


def test_import_error_types():
    """Verify error types can be imported."""
    from apxm import ApxmError, CompilationError, ExecutionError, ServerError
    assert issubclass(CompilationError, ApxmError)
    assert issubclass(ExecutionError, ApxmError)
    assert issubclass(ServerError, ApxmError)


def test_import_run():
    """Verify run() can be imported."""
    from apxm import emit_air_if_requested, run, run_workflow_file
    assert callable(emit_air_if_requested)
    assert callable(run)
    assert callable(run_workflow_file)


def test_import_execution_result():
    """Verify execution result types can be imported."""
    from apxm import (
        ExecutionResult,
        ExecutionStats,
        LLMUsage,
        WorkflowRunResult,
        new_session,
    )
    assert ExecutionResult is not None
    assert ExecutionStats is not None
    assert LLMUsage is not None
    assert WorkflowRunResult is not None
    assert callable(new_session)


def test_import_flow_module():
    """Verify FlowModule can be imported."""
    from apxm import FlowModule
    assert FlowModule is not None


def test_path_helpers_find_repo_root_and_home_override(tmp_path, monkeypatch):
    """Path helpers avoid fixed parent counts and respect APXM_HOME."""
    from apxm.constants import ENV_APXM_HOME
    from apxm.paths import agent_cwd, find_repo_root, local_apxm_path, repo_path

    root = tmp_path / "checkout"
    nested = root / "examples" / "python"
    nested.mkdir(parents=True)
    (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
    (root / "crates").mkdir()

    assert find_repo_root(nested) == root
    monkeypatch.chdir(nested)
    assert repo_path("examples") == root / "examples"
    assert local_apxm_path("sessions") == root / ".apxm" / "sessions"

    override = tmp_path / "agent-cwd"
    monkeypatch.setenv(ENV_APXM_HOME, str(override))
    assert agent_cwd() == str(override)


def test_path_helpers_find_repo_root_accepts_git_marker(tmp_path):
    """Sibling repos like apxm-eval qualify via a bare `.git` marker."""
    from apxm.paths import find_repo_root

    sibling_root = tmp_path / "apxm-eval"
    nested = sibling_root / "examples" / "python" / "benchmarks"
    nested.mkdir(parents=True)
    (sibling_root / ".git").mkdir()

    assert find_repo_root(nested) == sibling_root


def test_contract_find_repo_root_accepts_git_marker(tmp_path):
    """apxm.contract.find_repo_root accepts `.git` for companion repos."""
    from apxm.contract import find_repo_root as contract_find_repo_root

    sibling_root = tmp_path / "apxm-eval"
    nested = sibling_root / "examples" / "python" / "benchmarks"
    nested.mkdir(parents=True)
    (sibling_root / ".git").mkdir()

    assert contract_find_repo_root(nested) == sibling_root
