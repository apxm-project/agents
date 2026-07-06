import asyncio

import pytest

from apxm.errors import ServerError
from apxm.execution import CompiledFlow
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
