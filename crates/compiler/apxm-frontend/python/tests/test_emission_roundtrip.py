"""Python AIR emission and compiler round-trip checks."""

import os
import shutil
import subprocess
import tempfile

import pytest

from apxm import GraphRecorder, WorkflowTargetKind
from apxm.constants import (
    AGENT_NAME,
    AWAIT_RESULT,
    CAPABILITY,
    CAPABILITY_NAME,
    CHECKPOINT_ID,
    COMMUNICATE_PROTOCOL_ACP,
    CWD,
    DESCRIPTION,
    MAX_ITERATIONS,
    MESSAGE,
    MODE,
    PARAMS_JSON,
    PROFILE,
    PROMPT,
    PROTOCOL,
    PYTHON_HANDLER_ID,
    RECIPIENT,
    TARGET,
    TARGET_KIND,
    TEAM_NAME,
    SESSION_ROOT,
)
from apxm._generated.emission import (
    emit_autonomous,
    emit_checkpoint,
    emit_communicate,
    emit_inv_tool,
    emit_register_capability,
    emit_spawn_agent,
    emit_spawn_team,
    emit_workflow_spawn,
)

from .mocks import MOCK_AGENT_PROFILE

MOCK_AGENT_PROFILE_NAME = MOCK_AGENT_PROFILE.name

# ---------------------------------------------------------------------------
# Unit tests: emitter functions produce correct MLIR fragments
# ---------------------------------------------------------------------------

class TestEmitterFunctions:
    """Verify individual emit_* functions produce valid MLIR text."""

    def test_emit_spawn_agent_primary_and_keywords(self):
        agent_name = "alice"
        mode_value = "auto"
        result = emit_spawn_agent(
            "%alice",
            {
                AGENT_NAME: agent_name,
                PROFILE: MOCK_AGENT_PROFILE_NAME,
                MODE: mode_value,
            },
            [],
        )
        assert result.startswith("%alice = ais.spawn_agent")
        assert f'"{agent_name}"' in result
        assert f'{PROFILE} = "{MOCK_AGENT_PROFILE_NAME}"' in result
        assert f'{MODE} = "{mode_value}"' in result
        assert result.endswith(": !ais.token")

    def test_emit_spawn_agent_no_keywords(self):
        agent_name = "bob"
        result = emit_spawn_agent("%bob", {AGENT_NAME: agent_name}, [])
        assert f'%bob = ais.spawn_agent "{agent_name}"' in result
        assert "{" not in result  # no attr-dict when no keywords

    def test_emit_spawn_team_primary_and_keywords(self):
        team_name = "ultrathink"
        cwd = "/tmp/project"
        result = emit_spawn_team(
            "%team",
            {TEAM_NAME: team_name, CWD: cwd},
            [],
        )
        assert f'"{team_name}"' in result
        assert f'{CWD} = "{cwd}"' in result

    def test_emit_communicate_with_syntactic_keyword(self):
        message = "Hello"
        recipient = "alice"
        result = emit_communicate(
            "%msg",
            {
                MESSAGE: message,
                RECIPIENT: recipient,
                PROTOCOL: COMMUNICATE_PROTOCOL_ACP,
            },
            [],
        )
        assert f'"{message}"' in result
        assert f'to "{recipient}"' in result
        assert f'{PROTOCOL} = "{COMMUNICATE_PROTOCOL_ACP}"' in result

    def test_emit_communicate_no_recipient(self):
        message = "Broadcast"
        result = emit_communicate("%msg", {MESSAGE: message}, [])
        assert f'"{message}"' in result
        assert " to " not in result

    def test_emit_register_capability_primary_and_keywords(self):
        capability_name = "my_tool"
        description = "A test tool"
        handler_id = "sha256:abc123"
        result = emit_register_capability(
            "%reg",
            {
                CAPABILITY_NAME: capability_name,
                DESCRIPTION: description,
                PYTHON_HANDLER_ID: handler_id,
            },
            [],
        )
        assert f'"{capability_name}"' in result
        assert f'{DESCRIPTION} = "{description}"' in result
        assert f'{PYTHON_HANDLER_ID} = "{handler_id}"' in result

    def test_emit_inv_tool_primary_and_params(self):
        capability_name = "my_tool"
        params_json = '{"value": 42}'
        result = emit_inv_tool(
            "%call",
            {CAPABILITY: capability_name, PARAMS_JSON: params_json},
            [],
        )
        assert result == f'%call = ais.inv_tool "{capability_name}" ("{{\\"value\\": 42}}") : !ais.token'

    def test_emit_autonomous_primary(self):
        prompt = "Investigate the issue"
        max_iterations = 3
        result = emit_autonomous(
            "%auto", {PROMPT: prompt, MAX_ITERATIONS: max_iterations}, []
        )
        assert f'"{prompt}"' in result
        assert f'{MAX_ITERATIONS} = {max_iterations} : i64' in result

    def test_emit_checkpoint_primary(self):
        checkpoint_id = "before_analysis"
        result = emit_checkpoint(
            "%ckpt", {CHECKPOINT_ID: checkpoint_id}, []
        )
        assert f'"{checkpoint_id}"' in result

    def test_emit_workflow_spawn_primary_and_keywords(self):
        target_path = "tests/quality_fixtures/qa_factual/workflow.air"
        session_root = ".apxm/child-sessions"
        result = emit_workflow_spawn(
            "%child",
            {
                TARGET_KIND: WorkflowTargetKind.AIR_PATH.value,
                TARGET: target_path,
                AWAIT_RESULT: True,
                SESSION_ROOT: session_root,
            },
            [],
        )
        assert result.startswith("%child = ais.workflow_spawn")
        assert f'"{WorkflowTargetKind.AIR_PATH.value}"' in result
        assert f'"{target_path}"' in result
        assert f'{AWAIT_RESULT} = true' in result
        assert f'{SESSION_ROOT} = "{session_root}"' in result


# ---------------------------------------------------------------------------
# Integration tests: Python to_air() -> Rust compiler accepts
# ---------------------------------------------------------------------------

def _has_dekk_compile() -> bool:
    """Check if dekk apxm compile is available."""
    return shutil.which("dekk") is not None


@pytest.fixture
def tmp_air_dir():
    d = tempfile.mkdtemp(prefix="apxm_roundtrip_")
    yield d
    shutil.rmtree(d, ignore_errors=True)


def _compile_air(air_text: str, tmp_dir: str, name: str) -> subprocess.CompletedProcess:
    """Write .air text to a file and invoke dekk apxm compile."""
    air_path = os.path.join(tmp_dir, f"{name}.air")
    obj_path = os.path.join(tmp_dir, f"{name}.apxmobj")
    with open(air_path, "w") as f:
        f.write(air_text)
    return subprocess.run(
        ["dekk", "apxm", "compile", air_path, "-o", obj_path],
        capture_output=True,
        text=True,
        timeout=60,
    )


class TestGraphToAirRoundTrip:
    """Build workflows with Python, emit .air, and verify text shape."""

    def test_spawn_agent_air(self):
        g = GraphRecorder("spawn_test")
        g.spawn_agent("alice", agent_name="alice", profile=MOCK_AGENT_PROFILE, mode="auto")
        air = g.to_air()

        assert 'ais.spawn_agent "alice"' in air
        assert f'profile = "{MOCK_AGENT_PROFILE.name}"' in air
        assert 'mode = "auto"' in air

    def test_communicate_air(self):
        g = GraphRecorder("comm_test")
        g.spawn_agent("alice", agent_name="alice", profile=MOCK_AGENT_PROFILE)
        g.communicate(name="msg", target_agent="alice", message="Hello world")
        air = g.to_air()

        assert 'ais.communicate "Hello world" to "alice"' in air

    def test_register_capability_air(self):
        g = GraphRecorder("regcap_test")
        g.register_capability(
            name="reg",
            capability_name="custom_tool",
            description="A custom tool",
        )
        air = g.to_air()

        assert 'ais.register_capability "custom_tool"' in air
        assert 'description = "A custom tool"' in air

    def test_autonomous_air(self):
        g = GraphRecorder("auto_test")
        g.autonomous(
            name="auto_region",
            prompt="Investigate the issue",
            max_iterations=3,
        )
        air = g.to_air()

        assert 'ais.autonomous "Investigate the issue"' in air
        assert 'max_iterations = 3 : i64' in air

    def test_checkpoint_air(self):
        g = GraphRecorder("ckpt_test")
        g.ask(name="step1", prompt="Do work")
        # Use _add_node directly because GraphRecorder.checkpoint() maps to
        # FENCE (barrier semantics), while the CHECKPOINT op is a distinct
        # durable-execution primitive.
        g._add_node("save", "CHECKPOINT", {"checkpoint_id": "mid_point"})
        air = g.to_air()

        assert 'ais.checkpoint "mid_point"' in air

    def test_workflow_spawn_air(self):
        g = GraphRecorder("spawn_child_test")
        g.workflow_spawn(
            name="child",
            target_kind=WorkflowTargetKind.AIR_PATH,
            target="tests/quality_fixtures/qa_factual/workflow.air",
            session_root=".apxm/child-sessions",
        )
        air = g.to_air()

        assert (
            f'ais.workflow_spawn "{WorkflowTargetKind.AIR_PATH.value}" '
            '"tests/quality_fixtures/qa_factual/workflow.air"'
        ) in air
        assert 'session_root = ".apxm/child-sessions"' in air
        assert "await_result = true" in air


@pytest.mark.skipif(
    not _has_dekk_compile(),
    reason="dekk not available — skipping compile round-trip",
)
class TestCompileRoundTrip:
    """Emit representative .air from Python and verify the Rust compiler parses it."""

    def test_compile_representative_workflow(self, tmp_air_dir):
        g = GraphRecorder("rt_representative")
        g.spawn_agent(
            "alice",
            agent_name="alice",
            profile=MOCK_AGENT_PROFILE,
            mode="architect",
        )
        g.communicate(name="hello", target_agent="alice", message="Start work")
        g.register_capability(
            name="reg",
            capability_name="my_tool",
            description="Test tool",
        )
        g.invoke(name="call", capability="my_tool", params={"value": 42})
        air = g.to_air()
        result = _compile_air(air, tmp_air_dir, "rt_representative")
        assert result.returncode == 0, f"Compile failed:\n{result.stderr}"
