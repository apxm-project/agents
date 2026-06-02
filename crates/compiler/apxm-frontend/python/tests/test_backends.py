"""Tests for registered backend routing helpers."""

from __future__ import annotations

import pytest

from apxm.constants import ENV_APXM_CONFIG


def _write_config(path):
    path.write_text(
        """
[[backends]]
name = "vllm-fork"
type = "local"
protocol = "vllm"
endpoint = "http://localhost:8916/v1"
api_key = "env:APXM_TEST_BACKEND_KEY"

[backends.headers]
Authorization = "Bearer <redacted>"

[[backends.models]]
id = "served-model"
aliases = ["default", "demo"]
tags = ["vllm", "showcase"]
context_window = 8192
supports_vision = true
supports_functions = false
supports_thinking = false
max_output_tokens = 4096

[[backends]]
name = "vllm-alt"
type = "local"
protocol = "vllm"
endpoint = "http://localhost:8917/v1"

[backends.headers]

[[backends.models]]
id = "other-model"
aliases = ["other"]
tags = ["vllm"]

[[backends]]
name = "mock-bench"
type = "local"
protocol = "mock"
endpoint = "http://mock"

[backends.headers]
""",
        encoding="utf-8",
    )


@pytest.fixture()
def backend_config(tmp_path, monkeypatch):
    config = tmp_path / "config.toml"
    _write_config(config)
    monkeypatch.setenv(ENV_APXM_CONFIG, str(config))
    return config


def test_list_backends_parses_registry_without_secrets(backend_config):
    from apxm.backends import list_backends

    backends = list_backends()
    assert [backend.name for backend in backends] == [
        "vllm-fork",
        "vllm-alt",
        "mock-bench",
    ]
    assert not hasattr(backends[0], "api_key")
    assert not hasattr(backends[0], "headers")


def test_select_backend_resolves_exact_model(backend_config):
    from apxm.backends import select_backend

    route = select_backend(protocol="vllm", model="served-model")
    assert route.backend == "vllm-fork"
    assert route.model == "served-model"
    assert route.protocol == "vllm"


def test_select_backend_resolves_alias_and_tag(backend_config):
    from apxm.backends import select_backend

    route = select_backend(protocol="vllm", alias="demo", tag="showcase")
    assert route.backend == "vllm-fork"
    assert route.model == "served-model"


def test_select_backend_rejects_unknown_model(backend_config):
    from apxm.backends import BackendRegistryError, select_backend

    with pytest.raises(BackendRegistryError, match="no registered backend/model matched"):
        select_backend(protocol="vllm", model="missing-model")


def test_select_backend_rejects_ambiguous_protocol(backend_config):
    from apxm.backends import BackendRegistryError, select_backend

    with pytest.raises(BackendRegistryError, match="ambiguous"):
        select_backend(protocol="vllm")


def test_graph_rejects_raw_backend_argument(backend_config):
    from apxm import GraphRecorder

    graph = GraphRecorder("bad_route")

    with pytest.raises(TypeError, match="select_backend"):
        graph.ask(name="bad", prompt="hello", backend="missing")


def test_graph_routes_stamp_model_for_single_model_backend(backend_config):
    from apxm import GraphRecorder
    from apxm._generated import constants as gen_keys
    from apxm.backends import select_backend

    graph = GraphRecorder("single_backend")
    route = select_backend(backend="vllm-alt")
    graph.ask(name="routed", prompt="hello", route=route)

    node = graph.to_graph().nodes[0]
    assert node.attributes[gen_keys.BACKEND] == "vllm-alt"
    assert node.attributes[gen_keys.MODEL] == "other-model"


@pytest.fixture()
def multi_model_config(tmp_path, monkeypatch):
    config = tmp_path / "config.toml"
    config.write_text(
        """
[[backends]]
name = "amd"
type = "cloud"
protocol = "anthropic"

[[backends.models]]
id = "claude-sonnet-4-6"

[[backends.models]]
id = "claude-opus-4-8"
""",
        encoding="utf-8",
    )
    monkeypatch.setenv(ENV_APXM_CONFIG, str(config))
    return config


def test_select_backend_defers_model_for_multi_model_backend(multi_model_config):
    # Pinning only the backend on a multi-model backend must NOT raise: the
    # model is left unset so the runtime resolves it (parity with the chat
    # path), instead of forcing the caller to disambiguate at compile time.
    from apxm.backends import select_backend

    route = select_backend(backend="amd")
    assert route.backend == "amd"
    assert route.model is None
    assert route.protocol == "anthropic"


def test_graph_omits_model_attr_for_deferred_multi_model_backend(multi_model_config):
    from apxm import GraphRecorder
    from apxm._generated import constants as gen_keys
    from apxm.backends import select_backend

    graph = GraphRecorder("deferred_backend")
    graph.ask(name="routed", prompt="hello", route=select_backend(backend="amd"))

    node = graph.to_graph().nodes[0]
    assert node.attributes[gen_keys.BACKEND] == "amd"
    assert gen_keys.MODEL not in node.attributes


def test_select_backend_still_resolves_explicit_model_on_multi_model_backend(
    multi_model_config,
):
    from apxm.backends import select_backend

    route = select_backend(backend="amd", model="claude-opus-4-8")
    assert route.backend == "amd"
    assert route.model == "claude-opus-4-8"
