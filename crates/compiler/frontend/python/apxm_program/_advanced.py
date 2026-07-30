"""Advanced authoring markers: static Hooks and structured task scopes."""

from __future__ import annotations

import hashlib
import inspect
from dataclasses import dataclass, replace
from typing import Any, Callable, Optional


@dataclass(frozen=True, slots=True)
class HookDecl:
    """A static before/after Hook binding over a declared scope."""

    phase: str
    target_selector: str
    scope: str = "node"
    return_mode: str = "observe"
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
                "Unit" if self.return_mode == "observe" else _return_type(handler)
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

    def before(self, *, target: str, scope: str = "node", replace: bool = False) -> HookDecl:
        _validate_options(target, scope)
        return HookDecl(
            phase="before",
            target_selector=target,
            scope=scope,
            return_mode="replace_result" if replace else "observe",
        )

    def after(self, *, target: str, scope: str = "node", replace: bool = False) -> HookDecl:
        _validate_options(target, scope)
        return HookDecl(
            phase="after",
            target_selector=target,
            scope=scope,
            return_mode="replace_result" if replace else "observe",
        )


def _validate_options(target: str, scope: str) -> None:
    """Fail closed unless a Hook uses one closed static binding shape."""
    if not isinstance(target, str) or not target:
        raise TypeError("a Hook target is one non-empty static source selector")
    if scope not in {"agent", "loop", "node", "model", "capability"}:
        raise ValueError("a Hook scope is agent, loop, node, model, or capability")


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
