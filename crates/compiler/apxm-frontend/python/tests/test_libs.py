"""Tests for ``apxm.libs`` — the agent-side import surface for compiled
APXM skill packs.

The tests stub out the module-level httpx client in :mod:`apxm.execution`
so they exercise the request-shape, kwarg-to-args mapping, and
response-unpacking contracts without standing up a real ``apxm-server``.
"""

from __future__ import annotations

import pytest

import apxm.execution as _execution
import apxm.libs as libs


class _FakeResponse:
    def __init__(self, status_code: int, payload: dict | None = None, text: str = ""):
        self.status_code = status_code
        self._payload = payload or {}
        self.text = text or str(payload)

    def json(self) -> dict:
        return self._payload


class _FakeClient:
    """Minimal async stand-in for the shared ``httpx.AsyncClient``."""

    def __init__(self) -> None:
        self.posts: list[tuple[str, dict]] = []
        self.gets: list[str] = []
        self.get_response: _FakeResponse | None = None
        self.post_response: _FakeResponse | None = None

    async def get(self, path: str) -> _FakeResponse:
        self.gets.append(path)
        assert self.get_response is not None, "get_response not configured"
        return self.get_response

    async def post(self, path: str, *, json: dict) -> _FakeResponse:
        self.posts.append((path, json))
        assert self.post_response is not None, "post_response not configured"
        return self.post_response


@pytest.fixture
def fake_client(monkeypatch):
    client = _FakeClient()

    async def _get_client_stub():
        return client

    monkeypatch.setattr(_execution, "_get_client", _get_client_stub)
    yield client


def _skill_detail(skill_id: str, version: str, inputs: list[dict]) -> dict:
    return {
        "skill_id": skill_id,
        "version": version,
        "manifest": {
            "skill_id": skill_id,
            "version": version,
            "entry_flow": "main",
            "inputs": inputs,
            "outputs": [],
        },
    }


def test_load_returns_handle_with_input_order(fake_client):
    fake_client.get_response = _FakeResponse(
        200,
        _skill_detail(
            "demo",
            "0.1.0",
            [
                {"name": "task", "type": "string", "required": True},
                {"name": "context", "type": "string", "required": False},
            ],
        ),
    )
    handle = libs.load("demo")
    assert handle.skill_id == "demo"
    assert handle.version == "0.1.0"
    assert handle.input_order == ("task", "context")
    assert fake_client.gets == ["/v1/skills/demo"]


def test_load_accepts_pinned_version(fake_client):
    fake_client.get_response = _FakeResponse(
        200, _skill_detail("demo", "0.2.0", [])
    )
    handle = libs.load("demo@0.2.0")
    assert handle.skill_id == "demo"
    assert handle.version == "0.2.0"
    assert fake_client.gets == ["/v1/skills/demo@0.2.0"]


def test_load_rejects_empty_id(fake_client):
    with pytest.raises(TypeError):
        libs.load("")


def test_load_surfaces_404(fake_client):
    fake_client.get_response = _FakeResponse(404, {}, text="missing")
    from apxm.errors import ServerError

    with pytest.raises(ServerError):
        libs.load("nope")


def test_invoke_maps_kwargs_to_positional_args(fake_client):
    fake_client.get_response = _FakeResponse(
        200,
        _skill_detail(
            "demo",
            "0.1.0",
            [
                {"name": "task", "type": "string"},
                {"name": "context", "type": "string"},
            ],
        ),
    )
    fake_client.post_response = _FakeResponse(
        200,
        {
            "execution_id": "exec-1",
            "content": "ok",
            "session_dir": "/tmp/sess",
            "results": {"summary": "ok"},
            "stats": {"executed_nodes": 2},
            "llm_usage": {"input_tokens": 7},
        },
    )
    handle = libs.load("demo")
    result = handle.invoke(task="do the thing", context="ctx")
    assert result.execution_id == "exec-1"
    assert result.content == "ok"
    assert result.session_dir == "/tmp/sess"
    assert result.results == {"summary": "ok"}
    assert result.stats == {"executed_nodes": 2}
    assert result.llm_usage == {"input_tokens": 7}

    assert fake_client.posts == [
        (
            "/v1/skills/demo/execute",
            {"args": ["do the thing", "ctx"]},
        )
    ]


def test_invoke_rejects_unknown_kwargs(fake_client):
    fake_client.get_response = _FakeResponse(
        200,
        _skill_detail(
            "demo", "0.1.0", [{"name": "task", "type": "string"}]
        ),
    )
    handle = libs.load("demo")
    with pytest.raises(TypeError):
        handle.invoke(task="x", bogus=1)


def test_invoke_forwards_session_id(fake_client):
    fake_client.get_response = _FakeResponse(
        200, _skill_detail("demo", "0.1.0", [])
    )
    fake_client.post_response = _FakeResponse(
        200, {"execution_id": "x", "results": {}}
    )
    handle = libs.load("demo")
    handle.invoke(session_id="s-123")
    assert fake_client.posts == [
        ("/v1/skills/demo/execute", {"args": [], "session_id": "s-123"})
    ]


def test_invoke_surfaces_non_200(fake_client):
    fake_client.get_response = _FakeResponse(
        200, _skill_detail("demo", "0.1.0", [])
    )
    fake_client.post_response = _FakeResponse(500, {}, text="boom")
    from apxm.errors import ServerError

    handle = libs.load("demo")
    with pytest.raises(ServerError):
        handle.invoke()
