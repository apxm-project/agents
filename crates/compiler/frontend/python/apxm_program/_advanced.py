"""Advanced authoring markers: static Hooks and structured task scopes."""

from __future__ import annotations

import hashlib
import inspect
from dataclasses import dataclass, replace
from typing import Any, Callable, Optional

from ._generated.frontend_graph import (
    HOOK_PHASE_AFTER,
    HOOK_PHASE_BEFORE,
    HOOK_RETURN_MODE_OBSERVE,
    HOOK_RETURN_MODE_REPLACE_RESULT,
    HOOK_SCOPE_NODE,
    HOOK_SCOPES,
    HookPhase,
    HookReturnMode,
    HookScope,
)


@dataclass(frozen=True, slots=True)
class HookDecl:
    """A static before/after Hook binding over a declared scope."""

    phase: HookPhase
    target_selector: str
    scope: HookScope = HOOK_SCOPE_NODE
    return_mode: HookReturnMode = HOOK_RETURN_MODE_OBSERVE
    handler_ref: Optional[str] = None
    handler_digest: Optional[str] = None
    input_type_ref: str = "AgentFacade"
    output_type_ref: str = "HookResult"

    def __call__(self, handler: Callable[..., Any]) -> "HookDecl":
        """Bind one handler without invoking its authored behavior."""
        if not inspect.iscoroutinefunction(handler):
            raise TypeError("a Hook decorates one async def")
        return replace(
            self,
            handler_ref=handler.__name__,
            handler_digest=_digest(handler),
            input_type_ref=_first_parameter_type(handler),
            output_type_ref=(
                "Unit"
                if self.return_mode == HOOK_RETURN_MODE_OBSERVE
                else _return_type(handler)
            ),
        )


def _digest(handler: Callable[..., Any]) -> str:
    """Return the content digest used by the separate Hook handler bundle."""
    try:
        source = inspect.getsource(handler)
    except (OSError, TypeError):
        source = handler.__qualname__
    return "sha256:" + hashlib.sha256(source.encode("utf-8")).hexdigest()


def _type_name(annotation: Any, fallback: str) -> str:
    """Render one source annotation as the contract-facing type reference."""
    if annotation is inspect.Signature.empty or annotation is None:
        return fallback
    if isinstance(annotation, str):
        return annotation
    return getattr(annotation, "__name__", str(annotation))


def _first_parameter_type(handler: Callable[..., Any]) -> str:
    """Resolve the Hook callback input type without evaluating the callback."""
    for parameter in inspect.signature(handler).parameters.values():
        return _type_name(parameter.annotation, "AgentFacade")
    return "AgentFacade"


def _return_type(handler: Callable[..., Any]) -> str:
    """Resolve the Hook callback result type without evaluating the callback."""
    return _type_name(inspect.signature(handler).return_annotation, "HookResult")


class _Hook:
    """The Hook declaration surface exposing before/after bindings."""

    def before(
        self, *, target: str, scope: str = HOOK_SCOPE_NODE, replace: bool = False
    ) -> HookDecl:
        return _declare(HOOK_PHASE_BEFORE, target, scope, replace)

    def after(
        self, *, target: str, scope: str = HOOK_SCOPE_NODE, replace: bool = False
    ) -> HookDecl:
        return _declare(HOOK_PHASE_AFTER, target, scope, replace)


def _declare(phase: HookPhase, target: str, scope: str, replace: bool) -> HookDecl:
    """Fail closed unless a Hook uses one closed static binding shape."""
    if not isinstance(target, str) or not target:
        raise TypeError("a Hook target is one non-empty static source selector")
    if scope not in HOOK_SCOPES:
        raise ValueError(f"a Hook scope is one of {', '.join(HOOK_SCOPES)}")
    return HookDecl(
        phase=phase,
        target_selector=target,
        scope=scope,
        return_mode=(
            HOOK_RETURN_MODE_REPLACE_RESULT if replace else HOOK_RETURN_MODE_OBSERVE
        ),
    )


class TaskGroup:
    """A language-neutral structured concurrency scope with mandatory join."""

    def __enter__(self) -> "TaskGroup":  # pragma: no cover
        raise RuntimeError("TaskGroup is a compiled structured scope in an Agent body")

    def __exit__(self, *args: Any) -> None:  # pragma: no cover
        raise RuntimeError("TaskGroup is a compiled structured scope in an Agent body")

    async def __aenter__(self) -> "TaskGroup":  # pragma: no cover
        raise RuntimeError("TaskGroup is a compiled structured scope in an Agent body")

    async def __aexit__(self, *args: Any) -> None:  # pragma: no cover
        raise RuntimeError("TaskGroup is a compiled structured scope in an Agent body")


Hook = _Hook()
