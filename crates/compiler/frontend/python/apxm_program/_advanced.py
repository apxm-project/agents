"""Advanced authoring markers: static Hooks and structured task scopes."""

from __future__ import annotations

import hashlib
import inspect
from dataclasses import dataclass, replace
from typing import Any, Callable, Optional

from ._generated.diagnostics import HOOK_DYNAMIC_REGISTRATION, HOOK_SCOPE_UNRESOLVED
from ._generated.frontend_graph import (
    HOOK_PHASE_AFTER,
    HOOK_PHASE_BEFORE,
    HOOK_SCOPE_NODE,
    HOOK_SCOPES,
    HookPhase,
    HookScope,
)
from ._markers import (
    CapabilityBinding,
    EventType,
    ModelBinding,
    SkillDecl,
    ToolBinding,
)

#: The declarations a Hook may name directly, as against by selector. It is the
#: same set TypeScript's `HookTarget` states, so `target=SearchWeb` is one
#: authoring shape in both languages rather than one language's convenience.
HOOK_TARGET_DECLARATIONS = (
    ModelBinding,
    ToolBinding,
    CapabilityBinding,
    EventType,
    SkillDecl,
)


@dataclass(frozen=True, slots=True)
class HookDecl:
    """A static before/after Hook binding over a declared scope.

    The decorated handler is retained, unexecuted, because a Hook's body is
    captured as ordinary Agent Program structure: its Capability and Model calls
    become the same typed intents an Agent body records. Nothing about a Hook is
    an opaque handler the artifact only names.
    """

    phase: HookPhase
    target_selector: str
    scope: HookScope = HOOK_SCOPE_NODE
    #: The declaration the author named, when they named it rather than spelled
    #: its selector. Capture resolves it back to the module name that binds it,
    #: which is the name a selector would have carried.
    target_declaration: Optional[Any] = None
    handler: Optional[Callable[..., Any]] = None
    handler_ref: Optional[str] = None
    handler_digest: Optional[str] = None
    input_type_ref: str = "AgentFacade"

    def __call__(self, handler: Callable[..., Any]) -> "HookDecl":
        """Bind one handler without invoking its authored behavior."""
        if not inspect.iscoroutinefunction(handler):
            raise TypeError("a Hook decorates one async def")
        return replace(
            self,
            handler=handler,
            handler_ref=handler.__name__,
            handler_digest=_digest(handler),
            input_type_ref=_first_parameter_type(handler),
        )


def _digest(handler: Callable[..., Any]) -> str:
    """Return the content digest pinning the source the body was captured from."""
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


class _Hook:
    """The Hook declaration surface exposing before/after bindings."""

    def before(self, *, target: Any, scope: str = HOOK_SCOPE_NODE) -> HookDecl:
        return _declare(HOOK_PHASE_BEFORE, target, scope)

    def after(self, *, target: Any, scope: str = HOOK_SCOPE_NODE) -> HookDecl:
        return _declare(HOOK_PHASE_AFTER, target, scope)


def _declare(phase: HookPhase, target: Any, scope: str) -> HookDecl:
    """Fail closed unless a Hook uses one closed static binding shape.

    A target is the declaration itself — ``target=SearchWeb``, the shape
    TypeScript takes — or the selector naming one, which is how a Hook reaches
    the Agent body or a loop, neither of which is a declaration an author holds.
    There is no ``replace`` argument: whether a Hook observes or replaces is
    read off its captured body, so the declaration cannot claim one thing while
    the body does another.
    """
    declaration: Optional[Any] = None
    if isinstance(target, HOOK_TARGET_DECLARATIONS):
        declaration, selector = target, ""
    elif isinstance(target, str) and target:
        selector = target
    else:
        raise TypeError(
            f"{HOOK_DYNAMIC_REGISTRATION}: a Hook is registered statically, so its "
            f"target is one declaration or the selector naming one, not {target!r}"
        )
    if scope not in HOOK_SCOPES:
        raise ValueError(
            f"{HOOK_SCOPE_UNRESOLVED}: a Hook scope is one of "
            f"{', '.join(HOOK_SCOPES)}, not {scope!r}"
        )
    return HookDecl(
        phase=phase,
        target_selector=selector,
        scope=scope,
        target_declaration=declaration,
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
