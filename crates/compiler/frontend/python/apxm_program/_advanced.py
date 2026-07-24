"""Advanced authoring markers: static Hooks and structured task scopes."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable


@dataclass(frozen=True, slots=True)
class HookDecl:
    """A static before/after Hook binding over a declared scope."""

    phase: str
    target_selector: str
    scope: str = "node"
    return_mode: str = "observe"


class _Hook:
    """The Hook declaration surface exposing before/after bindings."""

    def before(self, *, target: str, scope: str = "node", replace: bool = False) -> HookDecl:
        return HookDecl(
            phase="before",
            target_selector=target,
            scope=scope,
            return_mode="replace_result" if replace else "observe",
        )

    def after(self, *, target: str, scope: str = "node", replace: bool = False) -> HookDecl:
        return HookDecl(
            phase="after",
            target_selector=target,
            scope=scope,
            return_mode="replace_result" if replace else "observe",
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
