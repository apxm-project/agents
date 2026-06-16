"""@hook decorator — bind a Python callable to a conversational lifecycle event.

A hook reuses the SAME invocation path as `@tool`: it produces a stable
`handler_id` and registers the callable in the shared tool registry, so the
runtime `PythonToolBridge` dispatches tools and hooks through one mechanism
(constitution #4 — one Python-handler mechanism). The frontend records each
hook into the artifact's hooks sidecar; later (US3) a `REGISTER_HOOK` op lowers
the binding so it travels inside the artifact (AIR-portable, constitution #3).

This module owns the NEW conversational lifecycle hooks. The legacy
subprocess-`command=` hooks in ``config.HookConfig`` are a separate, older
surface and are unaffected here.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from typing import Any, Callable

# Lifecycle events a `@hook` can bind to (data-model.md / contracts/python-api.md).
LIFECYCLE_EVENTS: frozenset[str] = frozenset(
    {
        "session_start",
        "pre_turn",
        "post_turn",
        "pre_ask",
        "post_ask",
        "pre_tool",
        "post_tool",
    }
)

# Only `pre_*` events can gate because later hooks observe after side effects.
_GATE_EVENTS: frozenset[str] = frozenset(
    {"session_start", "pre_turn", "pre_ask", "pre_tool"}
)

HOOK_MODES: frozenset[str] = frozenset({"observe", "gate"})


def _make_handler_id(fn: Callable[..., Any]) -> str:
    """Stable handler id from module:qualname (mirrors @tool)."""
    module = getattr(fn, "__module__", "__unknown__") or "__unknown__"
    qualname = getattr(fn, "__qualname__", fn.__name__)
    key = f"{module}:{qualname}"
    return f"sha256:{hashlib.sha256(key.encode('utf-8')).hexdigest()}"


@dataclass
class HookFn:
    """A Python callable bound to a lifecycle event.

    # Attributes
        fn: The original callable.
        event: One of ``LIFECYCLE_EVENTS``.
        match: Glob over tool/op name (default ``*``).
        mode: ``observe`` | ``gate``.
        handler_id: Stable content hash shared with the tool bridge.
        name: Hook name (defaults to the function name).
    """

    fn: Callable[..., Any]
    event: str
    match: str
    mode: str
    handler_id: str
    name: str
    metadata: dict[str, Any] = field(default_factory=dict)

    def __call__(self, *args: Any, **kwargs: Any) -> Any:
        return self.fn(*args, **kwargs)


def hook(
    *,
    on: str,
    match: str = "*",
    mode: str = "observe",
) -> Callable[[Callable[..., Any]], HookFn]:
    """Bind a Python callable to a conversational lifecycle event.

    Args:
        on: The lifecycle event (one of ``LIFECYCLE_EVENTS``).
        match: Glob over the tool/op name the hook applies to (default ``*``).
        mode: ``observe`` (default) or ``gate`` (allow/deny/edit; ``pre_*`` only).
    """
    if on not in LIFECYCLE_EVENTS:
        valid = ", ".join(sorted(LIFECYCLE_EVENTS))
        raise ValueError(f"@hook(on={on!r}) is not a valid event; expected one of: {valid}")
    if mode not in HOOK_MODES:
        valid = ", ".join(sorted(HOOK_MODES))
        raise ValueError(f"@hook(mode={mode!r}) invalid; expected one of: {valid}")
    if mode == "gate" and on not in _GATE_EVENTS:
        raise ValueError(
            f"@hook(on={on!r}, mode='gate') is invalid; gate mode is only "
            f"permitted on pre-execution events: {', '.join(sorted(_GATE_EVENTS))}"
        )

    def _wrap(fn: Callable[..., Any]) -> HookFn:
        # Reuse the shared tool registry so the bridge dispatches hooks and
        # tools through one path (constitution #4).
        from .tools import _TOOL_REGISTRY

        handler_id = _make_handler_id(fn)
        _TOOL_REGISTRY[handler_id] = fn
        return HookFn(
            fn=fn,
            event=on,
            match=match,
            mode=mode,
            handler_id=handler_id,
            name=getattr(fn, "__name__", "hook"),
        )

    return _wrap


def hook_descriptor(h: HookFn) -> dict[str, Any]:
    """Build the artifact hooks-sidecar descriptor for a hook (mirrors tools)."""
    import inspect

    module = getattr(h.fn, "__module__", "__unknown__") or "__unknown__"
    qualname = getattr(h.fn, "__qualname__", h.fn.__name__)
    descriptor: dict[str, Any] = {
        "handler_id": h.handler_id,
        "module": module,
        "qualname": qualname,
        "name": h.name,
        "event": h.event,
        "match": h.match,
        "mode": h.mode,
    }
    source_file = inspect.getsourcefile(h.fn)
    if source_file:
        descriptor["source_file"] = source_file
    return descriptor


__all__ = ["HookFn", "LIFECYCLE_EVENTS", "HOOK_MODES", "hook", "hook_descriptor"]
