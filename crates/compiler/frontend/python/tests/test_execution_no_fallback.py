"""Python execution failure-mode coverage."""

import asyncio

import pytest

from apxm.errors import ServerError
from apxm.execution import CompiledFlow
from apxm.constants import ENV_APXM_BIN
from apxm.proxy import GraphRecorder
import apxm.execution as execution


def _flow() -> CompiledFlow:
    graph = GraphRecorder("server_required")
    message = graph.print(name="message", message="hello")
    graph.done(message)
    return CompiledFlow(graph.to_graph())


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
