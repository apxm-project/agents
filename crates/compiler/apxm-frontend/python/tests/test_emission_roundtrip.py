"""Round-trip tests: Python to_air() -> Rust compiler parse.

Every op that previously had broken emission (spawn_agent, communicate,
register_capability, spawn_team, autonomous, checkpoint) is emitted via
Python, and the resulting .air text is validated for MLIR compliance.

For the three most common ops (spawn_agent, communicate, register_capability),
we additionally invoke `dekk apxm compile` to prove the Rust parser accepts
the emitted text.
"""

import os
import shutil
import subprocess
import tempfile

import pytest

from apxm import GraphRecorder, WorkflowTargetKind
from apxm._generated.emission import (
    emit_autonomous,
    emit_checkpoint,
    emit_communicate,
    emit_register_capability,
    emit_spawn_agent,
    emit_spawn_team,
    emit_workflow_spawn,
)

MOCK_AGENT_PROFILE = "mock-agent-profile"
MOCK_AGENT_PROFILE_ALT = "mock-agent-profile-alt"

# ---------------------------------------------------------------------------
# Unit tests: emitter functions produce correct MLIR fragments
# ---------------------------------------------------------------------------

class TestEmitterFunctions:
    """Verify individual emit_* functions produce valid MLIR text."""

    def test_emit_spawn_agent_primary_and_keywords(self):
        result = emit_spawn_agent(
            "%alice",
            {"agent_name": "alice", "profile": MOCK_AGENT_PROFILE, "mode": "auto"},
            [],
        )
        assert result.startswith("%alice = ais.spawn_agent")
        assert '"alice"' in result
        assert f'profile = "{MOCK_AGENT_PROFILE}"' in result
        assert 'mode = "auto"' in result
        assert result.endswith(": !ais.token")

    def test_emit_spawn_agent_no_keywords(self):
        result = emit_spawn_agent("%bob", {"agent_name": "bob"}, [])
        assert '%bob = ais.spawn_agent "bob"' in result
        assert "{" not in result  # no attr-dict when no keywords

    def test_emit_spawn_team_primary_and_keywords(self):
        result = emit_spawn_team(
            "%team",
            {"team_name": "ultrathink", "cwd": "/tmp/project"},
            [],
        )
        assert '"ultrathink"' in result
        assert 'cwd = "/tmp/project"' in result

    def test_emit_communicate_with_syntactic_keyword(self):
        result = emit_communicate(
            "%msg",
            {"message": "Hello", "recipient": "alice", "protocol": "acp"},
            [],
        )
        assert '"Hello"' in result
        assert 'to "alice"' in result
        assert 'protocol = "acp"' in result

    def test_emit_communicate_no_recipient(self):
        result = emit_communicate("%msg", {"message": "Broadcast"}, [])
        assert '"Broadcast"' in result
        assert " to " not in result

    def test_emit_register_capability_primary_and_keywords(self):
        result = emit_register_capability(
            "%reg",
            {
                "capability_name": "my_tool",
                "description": "A test tool",
                "python_handler_id": "sha256:abc123",
            },
            [],
        )
        assert '"my_tool"' in result
        assert 'description = "A test tool"' in result
        assert 'python_handler_id = "sha256:abc123"' in result

    def test_emit_autonomous_primary(self):
        result = emit_autonomous(
            "%auto", {"prompt": "Investigate the issue", "max_iterations": 3}, []
        )
        assert '"Investigate the issue"' in result
        assert 'max_iterations = 3 : i64' in result

    def test_emit_checkpoint_primary(self):
        result = emit_checkpoint(
            "%ckpt", {"checkpoint_id": "before_analysis"}, []
        )
        assert '"before_analysis"' in result

    def test_emit_workflow_spawn_primary_and_keywords(self):
        result = emit_workflow_spawn(
            "%child",
            {
                "target_kind": WorkflowTargetKind.GRAPH_PATH.value,
                "target": "tests/quality_fixtures/qa_factual/graph.air",
                "await_result": True,
                "session_root": ".apxm/child-sessions",
            },
            [],
        )
        assert result.startswith("%child = ais.workflow_spawn")
        assert f'"{WorkflowTargetKind.GRAPH_PATH.value}"' in result
        assert '"tests/quality_fixtures/qa_factual/graph.air"' in result
        assert 'await_result = true' in result
        assert 'session_root = ".apxm/child-sessions"' in result


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
    """Build graphs with Python, emit .air, verify text shape."""

    def test_spawn_agent_air(self):
        g = GraphRecorder("spawn_test")
        g.spawn_agent("alice", agent_name="alice", profile=MOCK_AGENT_PROFILE, mode="auto")
        air = g.to_air()

        assert 'ais.spawn_agent "alice"' in air
        assert f'profile = "{MOCK_AGENT_PROFILE}"' in air
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
            target_kind=WorkflowTargetKind.GRAPH_PATH,
            target="tests/quality_fixtures/qa_factual/graph.air",
            session_root=".apxm/child-sessions",
        )
        air = g.to_air()

        assert (
            f'ais.workflow_spawn "{WorkflowTargetKind.GRAPH_PATH.value}" '
            '"tests/quality_fixtures/qa_factual/graph.air"'
        ) in air
        assert 'session_root = ".apxm/child-sessions"' in air
        assert "await_result = true" in air


@pytest.mark.skipif(
    not _has_dekk_compile(),
    reason="dekk not available — skipping compile round-trip",
)
class TestCompileRoundTrip:
    """Emit .air from Python, pass to Rust compiler, verify it parses."""

    def test_compile_spawn_agent(self, tmp_air_dir):
        g = GraphRecorder("rt_spawn")
        g.spawn_agent(
            "alice",
            agent_name="alice",
            profile=MOCK_AGENT_PROFILE,
            mode="architect",
        )
        air = g.to_air()
        result = _compile_air(air, tmp_air_dir, "rt_spawn")
        assert result.returncode == 0, f"Compile failed:\n{result.stderr}"

    def test_compile_communicate(self, tmp_air_dir):
        g = GraphRecorder("rt_comm")
        g.spawn_agent("bob", agent_name="bob", profile=MOCK_AGENT_PROFILE_ALT)
        g.communicate(name="msg", target_agent="bob", message="Analyze this")
        air = g.to_air()
        result = _compile_air(air, tmp_air_dir, "rt_comm")
        assert result.returncode == 0, f"Compile failed:\n{result.stderr}"

    def test_compile_register_capability(self, tmp_air_dir):
        g = GraphRecorder("rt_regcap")
        g.register_capability(
            name="reg",
            capability_name="my_tool",
            description="Test tool",
            python_handler_id="sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        air = g.to_air()
        result = _compile_air(air, tmp_air_dir, "rt_regcap")
        assert result.returncode == 0, f"Compile failed:\n{result.stderr}"

    def test_compile_mixed_ops(self, tmp_air_dir):
        """Graph with spawn_agent + communicate + register_capability."""
        g = GraphRecorder("rt_mixed")
        g.spawn_agent("alice", agent_name="alice", profile=MOCK_AGENT_PROFILE)
        g.communicate(name="hello", target_agent="alice", message="Start work")
        g.register_capability(
            name="reg_tool",
            capability_name="analyzer",
            description="Analysis capability",
        )
        air = g.to_air()
        result = _compile_air(air, tmp_air_dir, "rt_mixed")
        assert result.returncode == 0, f"Compile failed:\n{result.stderr}"
