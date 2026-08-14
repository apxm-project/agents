"""Public authoring markers for typed Agent source.

These are compile-time declarations resolved by imported symbol identity and
type. They declare schemas, bindings, and handlers; they never execute the
authored body to discover the graph, and they carry no credential, grant,
endpoint, or runtime object.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional, TypeVar

from ._generated.permissions import Permission

I = TypeVar("I")
O = TypeVar("O")
T = TypeVar("T")


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
    """A static model-callable capability reference."""

    target_ref: str
    input_type_ref: str = "ToolInput"
    output_type_ref: str = "ToolOutput"
    permission: Optional[Permission] = None

    def __call__(self, *args: Any, **kwargs: Any) -> Any:  # pragma: no cover
        raise RuntimeError("a Tool is invoked inside a compiled Agent body")


@dataclass(frozen=True, slots=True)
class CapabilityBinding:
    """A static executable capability reference that is not a Tool."""

    target_ref: str
    input_type_ref: str = "CapabilityInput"
    output_type_ref: str = "CapabilityOutput"
    permission: Optional[Permission] = None

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


class _TypedFactory:
    """A marker whose type parameters are supplied by subscription.

    ``_type_parameters`` names them in order. It is the Python spelling of the
    same arity a TypeScript marker states as ``<Input, Output>``, which is what
    lets the surface conformance gate compare the two by argument shape rather
    than by trusting that a subscript exists.
    """

    _type_parameters: tuple[str, ...] = ()

    def __getitem__(self, types: Any) -> "_TypedFactory":
        supplied = types if isinstance(types, tuple) else (types,)
        if len(supplied) != len(self._type_parameters):
            expected = ", ".join(self._type_parameters)
            raise TypeError(f"{type(self).__name__} takes [{expected}]")
        return self


class _ModelFactory(_TypedFactory):
    _type_parameters = ("input", "output")

    def __call__(self, ref: str) -> ModelBinding:
        _require_exact_reference(ref, "Model")
        return ModelBinding(target_ref=ref)


class _ToolFactory(_TypedFactory):
    _type_parameters = ("input", "output")

    def __call__(
        self, capability_ref: str, *, permission: Optional[Permission] = None
    ) -> ToolBinding:
        _require_exact_reference(capability_ref, "Tool")
        return ToolBinding(target_ref=capability_ref, permission=permission)


class _CapabilityFactory(_TypedFactory):
    _type_parameters = ("input", "output")

    def __call__(
        self, ref: str, *, permission: Optional[Permission] = None
    ) -> CapabilityBinding:
        _require_exact_reference(ref, "Capability")
        return CapabilityBinding(target_ref=ref, permission=permission)


class _TypedEventFactory:
    """Binds a static Event payload type before its exact event reference."""

    def __init__(self, type_ref: str) -> None:
        self._type_ref = type_ref

    def __call__(self, ref: str) -> EventType:
        _require_exact_reference(ref, "Event")
        return EventType(type_ref=self._type_ref, target_ref=ref)


class _EventFactory(_TypedFactory):
    _type_parameters = ("payload",)

    def __getitem__(self, types: Any) -> _TypedEventFactory:
        super().__getitem__(types)
        return _TypedEventFactory(_type_name(types, "Event"))

    def __call__(self, ref: str) -> EventType:
        _require_exact_reference(ref, "Event")
        return EventType(type_ref="Event", target_ref=ref)


def Context(schema: Any) -> ContextSchema:
    """Declare a typed Program Context schema from one typed class."""
    if not isinstance(schema, type):
        raise TypeError("Context decorates one typed class")
    default_present = any(
        not name.startswith("__") for name in getattr(schema, "__annotations__", {})
    )
    return ContextSchema(type_ref=schema.__name__, default_present=default_present)


def _require_exact_reference(value: Any, marker: str) -> None:
    forbidden = {"default", "model.default", "support", "search-web", ""}
    if not isinstance(value, str) or value in forbidden:
        raise ValueError(
            f"{marker} accepts an exact typed reference, not a display name '{value}'"
        )


Model = _ModelFactory()
Tool = _ToolFactory()
Capability = _CapabilityFactory()
Event = _EventFactory()
