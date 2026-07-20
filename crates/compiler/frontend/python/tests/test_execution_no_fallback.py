"""Python execution failure-mode coverage."""

import asyncio
import json
import subprocess

import pytest

from apxm.errors import ServerError
from apxm.execution import CompiledFlow
from apxm.constants import ENV_APXM_BIN
from apxm.config import ExecutionOptions, HookConfig, HookEvent
from apxm.proxy import GraphRecorder
import apxm.execution as execution


def _flow() -> CompiledFlow:
    graph = GraphRecorder("server_required")
    message = graph.print(name="message", message="hello")
    graph.done(message)
    return CompiledFlow(graph.to_graph(), air_text=_canonical_air())


def _canonical_air() -> str:
    return json.dumps(
        {
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {
                    "node_id": "n.model",
                    "op": "model.call",
                    "operands": {"model_target_ref": "model.default"},
                }
            ],
            "structural_ir": [{"region_id": "r.return", "kind": "return"}],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": [],
            },
        }
    )


def test_run_requires_server_when_http_client_is_unavailable(monkeypatch):
    async def unavailable_client():
        raise RuntimeError("httpx is required for remote execution")

    def local_subprocess(*_args, **_kwargs):
        raise AssertionError("run() must not use local subprocess implicitly")

    monkeypatch.setattr(execution, "_get_client", unavailable_client)
    monkeypatch.setattr(CompiledFlow, "_run_local_subprocess", local_subprocess)

    with pytest.raises(ServerError, match="httpx is required"):
        asyncio.run(_flow().run())


def test_binary_resolution_rejects_a_checkout_target_fallback(monkeypatch, tmp_path):
    monkeypatch.delenv(ENV_APXM_BIN, raising=False)
    checkout_binary = tmp_path / "target" / "debug" / "apxm"
    checkout_binary.parent.mkdir(parents=True)
    checkout_binary.touch()
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(execution.shutil, "which", lambda _name: None)

    with pytest.raises(RuntimeError, match="No 'apxm' binary found"):
        execution._find_apxm_binary()


def test_compiled_flow_load_rejects_legacy_mlir_module(tmp_path):
    artifact = tmp_path / "legacy.air"
    artifact.write_text("module { func.func @main() }", encoding="utf-8")

    with pytest.raises(ValueError, match="canonical apxm.air.v1 JSON"):
        CompiledFlow.load(artifact)


def test_local_subprocess_executes_canonical_air(monkeypatch, tmp_path):
    apxm_bin = tmp_path / "apxm"
    apxm_bin.write_text("#!/bin/sh\n", encoding="utf-8")
    monkeypatch.setenv(ENV_APXM_BIN, str(apxm_bin))

    observed: dict[str, object] = {}

    def fake_run(cmd, *, capture_output, text, check, env):
        observed["cmd"] = cmd
        air_path = cmd[-1]
        observed["air"] = open(air_path, encoding="utf-8").read()
        return subprocess.CompletedProcess(
            cmd,
            0,
            stdout=json.dumps(
                {
                    "content": None,
                    "results": {},
                    "stats": {"executed_nodes": 1, "failed_nodes": 0, "duration_ms": 0},
                    "llm_usage": {"input_tokens": 1, "output_tokens": 1, "total_requests": 1},
                }
            ),
            stderr="",
        )

    monkeypatch.setattr(execution.subprocess, "run", fake_run)
    flow = CompiledFlow(GraphRecorder("canonical").to_graph(), air_text=_canonical_air())

    result = flow._run_local_subprocess(
        execution=ExecutionOptions(
            hooks=[HookConfig(event=HookEvent.GRAPH_START, command="true")]
        )
    )

    assert result.stats.executed_nodes == 1
    cmd = observed["cmd"]
    assert isinstance(cmd, list)
    assert cmd[0] == str(apxm_bin)
    assert cmd[-2] == "execute-canonical"
    assert json.loads(observed["air"])["schema_version"] == "apxm.air.v1"
