"""Public authoring markers for typed Agent source.

These are compile-time declarations resolved by imported symbol identity and
type. They declare schemas, bindings, and handlers; they never execute the
authored body to discover the graph, and they carry no credential, grant,
endpoint, or runtime object.
"""

from __future__ import annotations

import hashlib
import inspect
from dataclasses import dataclass
from typing import Any, Callable, Generic, Optional, TypeVar

I = TypeVar("I")
O = TypeVar("O")
T = TypeVar("T")


def _digest(func: Callable[..., Any]) -> str:
    try:
        source = inspect.getsource(func)
    except (OSError, TypeError):
        source = func.__qualname__
    return "sha256:" + hashlib.sha256(source.encode("utf-8")).hexdigest()


def _type_name(annotation: Any, fallback: str) -> str:
    if annotation is None:
        return fallback
    if isinstance(annotation, str):
        return annotation
    return getattr(annotation, "__name__", str(annotation))


@dataclass(frozen=True, slots=True)
class ModelBinding:
    """An exact typed model target. Calling it records a model invocation."""

    target_ref: str
    input_type_ref: str = "ModelRequest"
    output_type_ref: str = "ModelResponse"

    def __call__(self, *args: Any, **kwargs: Any) -> Any:  # pragma: no cover
        raise RuntimeError("a Model is invoked inside a compiled Agent body")


@dataclass(frozen=True, slots=True)
class ToolBinding:
    """A model-callable action reference or a bundled typed handler."""

    target_ref: str
    input_type_ref: str = "ToolInput"
    output_type_ref: str = "ToolOutput"
    handler_digest: Optional[str] = None

    def __call__(self, *args: Any, **kwargs: Any) -> Any:  # pragma: no cover
        raise RuntimeError("a Tool is invoked inside a compiled Agent body")


@dataclass(frozen=True, slots=True)
class CapabilityBinding:
    """A typed executable action that is not a model-callable Tool."""

    target_ref: str
    input_type_ref: str = "CapabilityInput"
    output_type_ref: str = "CapabilityOutput"
    handler_digest: Optional[str] = None

    def __call__(self, *args: Any, **kwargs: Any) -> Any:  # pragma: no cover
        raise RuntimeError("a Capability is invoked inside a compiled Agent body")


@dataclass(frozen=True, slots=True)
class EventType:
    """A typed durable event reference whose wait records an event wait."""

    type_ref: str
    target_ref: str

    def wait(self, *args: Any, **kwargs: Any) -> Any:  # pragma: no cover
        raise RuntimeError("an Event is awaited inside a compiled Agent body")


@dataclass(frozen=True, slots=True)
class ContextSchema:
    """The typed initial and persistent Program Context schema."""

    type_ref: str
    default_present: bool


class _ModelFactory:
    def __getitem__(self, _types: Any) -> "_ModelFactory":
        return self

    def __call__(self, target_ref: str) -> ModelBinding:
        _reject_display_name(target_ref, "Model")
        return ModelBinding(target_ref=target_ref)


class _ToolFactory:
    def __getitem__(self, _types: Any) -> "_ToolFactory":
        return self

    def __call__(self, target: Any) -> ToolBinding:
        if callable(target) and not isinstance(target, str):
            return ToolBinding(
                target_ref=f"tool:{target.__qualname__}",
                handler_digest=_digest(target),
                input_type_ref=_first_param_type(target),
                output_type_ref=_return_type(target),
            )
        _reject_display_name(target, "Tool")
        return ToolBinding(target_ref=target)


class _CapabilityFactory:
    def __getitem__(self, _types: Any) -> "_CapabilityFactory":
        return self

    def __call__(self, target: Any) -> CapabilityBinding:
        if callable(target) and not isinstance(target, str):
            return CapabilityBinding(
                target_ref=f"capability:{target.__qualname__}",
                handler_digest=_digest(target),
                input_type_ref=_first_param_type(target),
                output_type_ref=_return_type(target),
            )
        _reject_display_name(target, "Capability")
        return CapabilityBinding(target_ref=target)


class _TypedEventFactory:
    """Binds a static Event payload type before its exact event reference."""

    def __init__(self, type_ref: str) -> None:
        self._type_ref = type_ref

    def __call__(self, target_ref: str) -> EventType:
        _reject_display_name(target_ref, "Event")
        return EventType(type_ref=self._type_ref, target_ref=target_ref)


class _EventFactory:
    def __getitem__(self, type_arg: Any) -> _TypedEventFactory:
        return _TypedEventFactory(_type_name(type_arg, "Event"))

    def __call__(self, target_ref: str) -> EventType:
        _reject_display_name(target_ref, "Event")
        return EventType(type_ref="Event", target_ref=target_ref)


def Context(cls: Any) -> ContextSchema:
    """Declare a typed Program Context schema from one typed class."""
    if not isinstance(cls, type):
        raise TypeError("Context decorates one typed class")
    default_present = any(
        not name.startswith("__") for name in getattr(cls, "__annotations__", {})
    )
    return ContextSchema(type_ref=cls.__name__, default_present=default_present)


def _first_param_type(func: Callable[..., Any]) -> str:
    signature = inspect.signature(func)
    for parameter in signature.parameters.values():
        return _type_name(
            None if parameter.annotation is inspect.Signature.empty else parameter.annotation,
            "HandlerInput",
        )
    return "HandlerInput"


def _return_type(func: Callable[..., Any]) -> str:
    signature = inspect.signature(func)
    annotation = signature.return_annotation
    return _type_name(
        None if annotation is inspect.Signature.empty else annotation,
        "HandlerOutput",
    )


def _reject_display_name(value: Any, marker: str) -> None:
    forbidden = {"default", "support", "search-web", ""}
    if isinstance(value, str) and value in forbidden:
        raise ValueError(
            f"{marker} accepts an exact typed reference, not a display name '{value}'"
        )


Model = _ModelFactory()
Tool = _ToolFactory()
Capability = _CapabilityFactory()
Event = _EventFactory()
