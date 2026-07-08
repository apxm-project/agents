"""@hook decorator — bind a Python callable to a conversational lifecycle event.

A hook reuses the SAME invocation path as `@tool`: it produces a stable
`handler_id` and registers the callable in the shared tool registry, so the
runtime `PythonHandlerBridge` dispatches tools and hooks through one mechanism
(constitution #4 — one Python-handler mechanism). The frontend records each
hook as a `REGISTER_HOOK` op, so the binding travels inside canonical AIR.

This module owns conversational lifecycle hooks. The command-based hooks in
``config.HookConfig`` are a separate surface and are unaffected here.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Callable


class LifecycleEvent(str, Enum):
    SESSION_START = "session_start"
    PRE_TURN = "pre_turn"
    POST_TURN = "post_turn"
    PRE_ASK = "pre_ask"
    POST_ASK = "post_ask"
    PRE_CAP = "pre_cap"
    POST_CAP = "post_cap"


class HookMode(str, Enum):
    OBSERVE = "observe"
    GATE = "gate"


LIFECYCLE_EVENTS: frozenset[str] = frozenset(event.value for event in LifecycleEvent)
GATE_LIFECYCLE_EVENTS: frozenset[str] = frozenset(
    {
        LifecycleEvent.SESSION_START.value,
        LifecycleEvent.PRE_TURN.value,
        LifecycleEvent.PRE_ASK.value,
        LifecycleEvent.PRE_CAP.value,
    }
)
HOOK_MODES: frozenset[str] = frozenset(mode.value for mode in HookMode)


def normalize_lifecycle_event(value: LifecycleEvent | str) -> str:
    if isinstance(value, LifecycleEvent):
        return value.value
    if isinstance(value, str):
        return value
    raise TypeError("hook event must be a LifecycleEvent or string")


def normalize_hook_mode(value: HookMode | str) -> str:
    if isinstance(value, HookMode):
        return value.value
    if isinstance(value, str):
        return value
    raise TypeError("hook mode must be a HookMode or string")


def _make_handler_id(fn: Callable[..., Any]) -> str:
    """Stable handler id from module:qualname."""
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
    on: LifecycleEvent | str,
    match: str = "*",
    mode: HookMode | str = HookMode.OBSERVE,
) -> Callable[[Callable[..., Any]], HookFn]:
    """Bind a Python callable to a conversational lifecycle event.

    Args:
        on: The lifecycle event (one of ``LIFECYCLE_EVENTS``).
        match: Glob over the tool/op name the hook applies to (default ``*``).
        mode: ``observe`` (default) or ``gate`` (allow/deny/edit; ``pre_*`` only).
    """
    event_value = normalize_lifecycle_event(on)
    mode_value = normalize_hook_mode(mode)
    if event_value not in LIFECYCLE_EVENTS:
        valid = ", ".join(sorted(LIFECYCLE_EVENTS))
        raise ValueError(f"@hook(on={event_value!r}) is not a valid event; expected one of: {valid}")
    if mode_value not in HOOK_MODES:
        valid = ", ".join(sorted(HOOK_MODES))
        raise ValueError(f"@hook(mode={mode_value!r}) invalid; expected one of: {valid}")
    if mode_value == HookMode.GATE.value and event_value not in GATE_LIFECYCLE_EVENTS:
        raise ValueError(
            f"@hook(on={event_value!r}, mode='gate') is invalid; gate mode is only "
            f"permitted on pre-execution events: {', '.join(sorted(GATE_LIFECYCLE_EVENTS))}"
        )

    def _wrap(fn: Callable[..., Any]) -> HookFn:
        # Reuse the shared tool registry so the bridge dispatches hooks and
        # tools through one path (constitution #4).
        from .tools import _TOOL_REGISTRY

        handler_id = _make_handler_id(fn)
        _TOOL_REGISTRY[handler_id] = fn
        return HookFn(
            fn=fn,
            event=event_value,
            match=match,
            mode=mode_value,
            handler_id=handler_id,
            name=getattr(fn, "__name__", "hook"),
        )

    return _wrap


def hook_descriptor(h: HookFn) -> dict[str, Any]:
    """Build a handler manifest descriptor for a hook."""
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


__all__ = [
    "GATE_LIFECYCLE_EVENTS",
    "HookFn",
    "HookMode",
    "HOOK_MODES",
    "LifecycleEvent",
    "LIFECYCLE_EVENTS",
    "hook",
    "hook_descriptor",
    "normalize_hook_mode",
    "normalize_lifecycle_event",
]
