"""Registered backend routing helpers for APXM Python graphs.

The backend registry is dynamic user/project configuration, not generated
frontend contract. This module reads the same APXM config locations as the
runtime and returns small typed route objects for graph construction.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python 3.10 compatibility
    import tomli as tomllib  # type: ignore[no-redef]

from . import constants as graph_keys

_CONFIG_ENV = graph_keys.ENV_APXM_CONFIG
_APXM_DIR = ".apxm"
_CONFIG_FILE = "config.toml"
_BACKENDS_KEY = "backends"
_MODELS_KEY = "models"
_NAME_KEY = "name"
_TYPE_KEY = "type"
_PROTOCOL_KEY = "protocol"
_ENDPOINT_KEY = "endpoint"
_MODEL_ID_KEY = "id"
_ALIASES_KEY = "aliases"
_TAGS_KEY = "tags"
_CONTEXT_WINDOW_KEY = "context_window"
_SUPPORTS_VISION_KEY = "supports_vision"
_SUPPORTS_FUNCTIONS_KEY = "supports_functions"
_SUPPORTS_THINKING_KEY = "supports_thinking"
_MAX_OUTPUT_TOKENS_KEY = "max_output_tokens"
_EMPTY_MODEL = "<none>"
_MOCK_PROTOCOL = "mock"
# Protocols whose runtime backend resolves a concrete model on its own when a
# node pins only the backend (the provider coalesces to its configured model).
# vLLM is deliberately excluded: it bails at execution without an explicit
# model, so for vLLM we keep raising the precise "pass model=" error at build
# time rather than deferring into a late runtime failure.
_RUNTIME_RESOLVES_MODEL = frozenset({"anthropic", "openai", "google", "ollama", "mock"})


@dataclass(frozen=True, slots=True)
class RegisteredModel:
    """Model entry registered under an APXM backend."""

    id: str
    aliases: tuple[str, ...] = ()
    tags: tuple[str, ...] = ()
    context_window: int = 0
    supports_vision: bool = False
    supports_functions: bool = False
    supports_thinking: bool = False
    max_output_tokens: int | None = None

    def matches(self, value: str) -> bool:
        return self.id == value or value in self.aliases

    def has_tag(self, tag: str) -> bool:
        return tag in self.tags


@dataclass(frozen=True, slots=True)
class RegisteredBackend:
    """Backend entry registered in APXM config.

    Secret-bearing fields such as API keys and headers are intentionally not
    represented here.
    """

    name: str
    backend_type: str
    protocol: str
    endpoint: str | None
    models: tuple[RegisteredModel, ...] = ()

    def model_for(self, value: str) -> RegisteredModel | None:
        return next((model for model in self.models if model.matches(value)), None)


@dataclass(frozen=True, slots=True)
class BackendRoute:
    """Concrete backend/model route that can stamp graph attributes."""

    backend: str
    model: str | None
    protocol: str


class BackendRegistryError(ValueError):
    """Raised when graph routing references unregistered backends or models."""


def list_backends(*, protocol: str | None = None) -> list[RegisteredBackend]:
    """List registered APXM backends from the active config file."""

    backends = _load_backends()
    if protocol is None:
        return backends
    return [backend for backend in backends if backend.protocol == protocol]


def get_backend(name: str) -> RegisteredBackend:
    """Return a registered backend by name or raise a registry error."""

    for backend in _load_backends():
        if backend.name == name:
            return backend
    raise BackendRegistryError(
        f"backend '{name}' is not registered. Registered backends: "
        f"{_format_backend_names(_load_backends())}"
    )


def select_backend(
    *,
    backend: str | None = None,
    model: str | None = None,
    alias: str | None = None,
    protocol: str | None = None,
    tag: str | None = None,
) -> BackendRoute:
    """Resolve a registered backend/model route.

    Selection is conservative: ambiguous criteria raise and ask the caller to
    pass a backend or exact model/alias.
    """

    if model is not None and alias is not None:
        raise BackendRegistryError("pass either model= or alias=, not both")

    candidates = _filter_backends(_load_backends(), protocol=protocol, backend=backend)
    lookup_model = model or alias

    if lookup_model is not None:
        matches = [
            (candidate, resolved)
            for candidate in candidates
            if (resolved := candidate.model_for(lookup_model)) is not None
        ]
        if tag is not None:
            matches = [
                (candidate, resolved)
                for candidate, resolved in matches
                if resolved.has_tag(tag)
            ]
        if not matches:
            raise BackendRegistryError(
                "no registered backend/model matched "
                f"{_format_selector(backend=backend, protocol=protocol, model=lookup_model, tag=tag)}. "
                f"Available routes: {_format_routes(candidates)}"
            )
        if len(matches) > 1:
            raise BackendRegistryError(
                "backend/model selection is ambiguous for "
                f"{_format_selector(backend=backend, protocol=protocol, model=lookup_model, tag=tag)}. "
                f"Matched routes: {_format_routes([candidate for candidate, _ in matches])}"
            )
        selected_backend, selected_model = matches[0]
        return BackendRoute(
            backend=selected_backend.name,
            model=selected_model.id,
            protocol=selected_backend.protocol,
        )

    if tag is not None:
        matches = [
            (candidate, resolved)
            for candidate in candidates
            for resolved in candidate.models
            if resolved.has_tag(tag)
        ]
        if len(matches) == 1:
            selected_backend, selected_model = matches[0]
            return BackendRoute(
                backend=selected_backend.name,
                model=selected_model.id,
                protocol=selected_backend.protocol,
            )
        if not matches:
            raise BackendRegistryError(
                f"no registered model has tag '{tag}'. Available routes: {_format_routes(candidates)}"
            )
        raise BackendRegistryError(
            f"model tag '{tag}' is ambiguous. Matched routes: "
            f"{_format_routes([candidate for candidate, _ in matches])}"
        )

    if backend is not None:
        selected = _single_backend(candidates, selector=backend)
        selected_model = _implicit_model(selected, allow_defer=True)
        return BackendRoute(
            backend=selected.name,
            model=selected_model.id if selected_model is not None else None,
            protocol=selected.protocol,
        )

    if len(candidates) == 1:
        selected = candidates[0]
        selected_model = _implicit_model(selected)
        return BackendRoute(
            backend=selected.name,
            model=selected_model.id if selected_model is not None else None,
            protocol=selected.protocol,
        )

    raise BackendRegistryError(
        "backend selection is ambiguous. Pass backend=, model=, or alias=. "
        f"Available routes: {_format_routes(candidates)}"
    )


def select(*args: Any, **kwargs: Any) -> BackendRoute:
    """Alias for select_backend()."""

    return select_backend(*args, **kwargs)


def resolve_node_route(
    *,
    backend: str | None,
    model: str | None,
    protocol: str | None = None,
) -> BackendRoute | None:
    """Validate and canonicalize a node-level backend/model route.

    Returns None when the node does not declare routing attributes.
    """

    if backend is None and model is None:
        return None
    return select_backend(backend=backend, model=model, protocol=protocol)


def validate_graph_routes(nodes: list[Any]) -> None:
    """Validate all routed LLM nodes and stamp canonical backend/model ids."""

    llm_ops = getattr(graph_keys, "LLM_OPS")
    for node in nodes:
        if node.op not in llm_ops:
            continue
        attrs = node.attributes
        provider = attrs.get(graph_keys.PROVIDER)
        protocol = _provider_protocol(provider)
        route = resolve_node_route(
            backend=attrs.get(graph_keys.BACKEND),
            model=attrs.get(graph_keys.MODEL),
            protocol=protocol,
        )
        if route is None:
            continue
        attrs[graph_keys.BACKEND] = route.backend
        if route.model is not None:
            attrs[graph_keys.MODEL] = route.model


def _provider_protocol(provider: Any) -> str | None:
    if provider is None:
        return None
    protocol = getattr(provider, "protocol", None)
    if protocol is not None:
        return str(protocol)
    if isinstance(provider, str):
        from ._generated.providers import resolve_provider

        return resolve_provider(provider).protocol
    return str(provider)


def config_path() -> Path | None:
    """Return the active APXM config path if one exists."""

    explicit = os.environ.get(_CONFIG_ENV)
    if explicit:
        return Path(explicit).expanduser()

    cwd = Path.cwd().resolve()
    for parent in (cwd, *cwd.parents):
        candidate = parent / _APXM_DIR / _CONFIG_FILE
        if candidate.exists():
            return candidate

    home_candidate = Path.home() / _APXM_DIR / _CONFIG_FILE
    return home_candidate if home_candidate.exists() else None


def _load_backends() -> list[RegisteredBackend]:
    path = config_path()
    if path is None or not path.exists():
        return []

    with path.open("rb") as handle:
        raw = tomllib.load(handle)

    raw_backends = raw.get(_BACKENDS_KEY, [])
    if not isinstance(raw_backends, list):
        raise BackendRegistryError(f"{path} has invalid '{_BACKENDS_KEY}' section")
    return [_parse_backend(item) for item in raw_backends]


def _parse_backend(raw: dict[str, Any]) -> RegisteredBackend:
    models = raw.get(_MODELS_KEY, [])
    if models is None:
        models = []
    return RegisteredBackend(
        name=str(raw[_NAME_KEY]),
        backend_type=str(raw.get(_TYPE_KEY, "")),
        protocol=str(raw[_PROTOCOL_KEY]),
        endpoint=raw.get(_ENDPOINT_KEY),
        models=tuple(_parse_model(model) for model in models),
    )


def _parse_model(raw: dict[str, Any]) -> RegisteredModel:
    return RegisteredModel(
        id=str(raw[_MODEL_ID_KEY]),
        aliases=tuple(str(value) for value in raw.get(_ALIASES_KEY, [])),
        tags=tuple(str(value) for value in raw.get(_TAGS_KEY, [])),
        context_window=int(raw.get(_CONTEXT_WINDOW_KEY, 0)),
        supports_vision=bool(raw.get(_SUPPORTS_VISION_KEY, False)),
        supports_functions=bool(raw.get(_SUPPORTS_FUNCTIONS_KEY, False)),
        supports_thinking=bool(raw.get(_SUPPORTS_THINKING_KEY, False)),
        max_output_tokens=raw.get(_MAX_OUTPUT_TOKENS_KEY),
    )


def _filter_backends(
    backends: list[RegisteredBackend],
    *,
    protocol: str | None,
    backend: str | None,
) -> list[RegisteredBackend]:
    filtered = backends
    if protocol is not None:
        filtered = [item for item in filtered if item.protocol == protocol]
    if backend is not None:
        filtered = [item for item in filtered if item.name == backend]
    if backend is not None and not filtered:
        raise BackendRegistryError(
            f"backend '{backend}' is not registered. Registered backends: "
            f"{_format_backend_names(backends)}"
        )
    if protocol is not None and not filtered:
        raise BackendRegistryError(
            f"no registered backends use protocol '{protocol}'. Registered backends: "
            f"{_format_backend_names(backends)}"
        )
    return filtered


def _single_backend(backends: list[RegisteredBackend], *, selector: str) -> RegisteredBackend:
    if len(backends) == 1:
        return backends[0]
    raise BackendRegistryError(
        f"backend '{selector}' is not registered. Registered backends: "
        f"{_format_backend_names(_load_backends())}"
    )


def _implicit_model(
    backend: RegisteredBackend, *, allow_defer: bool = False
) -> RegisteredModel | None:
    if len(backend.models) == 1:
        return backend.models[0]
    if not backend.models and backend.protocol == _MOCK_PROTOCOL:
        return None
    # Zero or several models: the model is left unset. Defer to the runtime only
    # when the caller explicitly pinned this backend (allow_defer) AND the
    # protocol self-resolves a model — the same contract the chat path relies on.
    # vLLM cannot, so it falls through to the precise build-time error below
    # instead of failing late at execution.
    if allow_defer and backend.protocol in _RUNTIME_RESOLVES_MODEL:
        return None
    if not backend.models:
        raise BackendRegistryError(
            f"backend '{backend.name}' has no registered models. Add one with "
            f"`dekk apxm backend add-model {backend.name} <model-id>`."
        )
    raise BackendRegistryError(
        f"backend '{backend.name}' serves multiple models. Pass model= or alias=. "
        f"Registered models: {_format_model_names(backend)}"
    )


def _format_backend_names(backends: list[RegisteredBackend]) -> str:
    if not backends:
        return _EMPTY_MODEL
    return ", ".join(sorted(backend.name for backend in backends))


def _format_model_names(backend: RegisteredBackend) -> str:
    if not backend.models:
        return _EMPTY_MODEL
    return ", ".join(model.id for model in backend.models)


def _format_routes(backends: list[RegisteredBackend]) -> str:
    pairs: list[str] = []
    for backend in backends:
        if not backend.models:
            pairs.append(f"{backend.name}:{_EMPTY_MODEL}")
            continue
        pairs.extend(f"{backend.name}:{model.id}" for model in backend.models)
    return ", ".join(pairs) if pairs else _EMPTY_MODEL


def _format_selector(
    *,
    backend: str | None,
    protocol: str | None,
    model: str | None,
    tag: str | None,
) -> str:
    parts = []
    if backend is not None:
        parts.append(f"backend={backend!r}")
    if protocol is not None:
        parts.append(f"protocol={protocol!r}")
    if model is not None:
        parts.append(f"model={model!r}")
    if tag is not None:
        parts.append(f"tag={tag!r}")
    return ", ".join(parts) if parts else "<empty selector>"


__all__ = [
    "BackendRegistryError",
    "BackendRoute",
    "RegisteredBackend",
    "RegisteredModel",
    "config_path",
    "get_backend",
    "list_backends",
    "resolve_node_route",
    "select",
    "select_backend",
    "validate_graph_routes",
]
