"""Portable handler-manifest helpers for Python frontend artifacts."""

from __future__ import annotations

from enum import Enum
import inspect
from pathlib import Path
from typing import Any, Callable, NotRequired, TypedDict


# Mirror of crates/machine/contracts/src/types/handler_manifest.rs.
HANDLER_MANIFEST_VERSION = "apxm.handler-manifest.v1"
HANDLER_MANIFEST_SOURCE_DIRECTORY = "handlers"
HANDLER_MANIFEST_HANDLER_ID_PREFIX = "sha256:"


class HandlerKind(str, Enum):
    """Runtime role encoded in a handler manifest descriptor."""

    TOOL = "tool"
    HOOK = "hook"


class HandlerLanguage(str, Enum):
    """Authoring language encoded in a handler manifest descriptor."""

    PYTHON = "python"


class HandlerSource(TypedDict):
    """Artifact-local executable source for one handler."""

    artifact_path: str
    content: str


class HandlerDescriptor(TypedDict):
    """Serialized Python view of the versioned handler-manifest contract."""

    kind: HandlerKind
    language: HandlerLanguage
    handler_id: str
    module: str
    qualname: str
    name: str
    source: HandlerSource
    description: str
    schema: dict[str, Any]
    event: NotRequired[str]
    match: NotRequired[str]
    mode: NotRequired[str]


class HandlerManifest(TypedDict):
    """Versioned cross-language artifact metadata for handlers."""

    version: str
    handlers: list[HandlerDescriptor]


def source_for_callable(handler_id: str, fn: Callable[..., Any]) -> HandlerSource:
    """Return artifact-local source for a Python handler callable."""
    source_file = inspect.getsourcefile(fn)
    if not source_file:
        raise ValueError(
            f"handler {handler_id} has no source file and cannot be embedded in an artifact"
        )
    source_path = Path(source_file)
    try:
        content = source_path.read_text(encoding="utf-8")
    except OSError as exc:
        raise ValueError(
            f"handler {handler_id} source file {source_path} cannot be read: {exc}"
        ) from exc
    if not content:
        raise ValueError(f"handler {handler_id} source file {source_path} is empty")
    suffix = source_path.suffix or ".py"
    return {
        "artifact_path": (
            f"{HANDLER_MANIFEST_SOURCE_DIRECTORY}/"
            f"{handler_id.removeprefix(HANDLER_MANIFEST_HANDLER_ID_PREFIX)}{suffix}"
        ),
        "content": content,
    }


def tool_descriptor(
    *,
    handler_id: str,
    fn: Callable[..., Any],
    name: str,
    description: str,
    schema: dict[str, Any],
) -> HandlerDescriptor:
    """Build one portable Python tool descriptor."""
    module = getattr(fn, "__module__", "__unknown__") or "__unknown__"
    qualname = getattr(fn, "__qualname__", getattr(fn, "__name__", name))
    return {
        "kind": HandlerKind.TOOL,
        "language": HandlerLanguage.PYTHON,
        "handler_id": handler_id,
        "module": module,
        "qualname": qualname,
        "name": name,
        "source": source_for_callable(handler_id, fn),
        "description": description,
        "schema": schema,
    }


def hook_descriptor(
    *,
    handler_id: str,
    fn: Callable[..., Any],
    name: str,
    event: str,
    match: str,
    mode: str,
) -> HandlerDescriptor:
    """Build one portable Python hook descriptor."""
    module = getattr(fn, "__module__", "__unknown__") or "__unknown__"
    qualname = getattr(fn, "__qualname__", getattr(fn, "__name__", name))
    return {
        "kind": HandlerKind.HOOK,
        "language": HandlerLanguage.PYTHON,
        "handler_id": handler_id,
        "module": module,
        "qualname": qualname,
        "name": name,
        "source": source_for_callable(handler_id, fn),
        "description": "",
        "schema": {},
        "event": event,
        "match": match,
        "mode": mode,
    }


def manifest(handlers: list[HandlerDescriptor]) -> HandlerManifest:
    """Wrap descriptors in the versioned cross-language manifest object."""
    return {"version": HANDLER_MANIFEST_VERSION, "handlers": handlers}
