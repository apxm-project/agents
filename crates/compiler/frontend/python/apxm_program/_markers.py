"""Public authoring markers for typed Agent source.

These are compile-time declarations resolved by imported symbol identity and
type. They declare schemas, bindings, and handlers; they never execute the
authored body to discover the graph, and they carry no credential, grant,
endpoint, or runtime object.
"""

from __future__ import annotations

from typing import Any, Awaitable, Generic, NamedTuple, Optional, TypeVar

from ._generated.capabilities import BUILTIN_CAPABILITIES
from ._host_capabilities import (
    HOST_CAPABILITY_REF_PREFIX,
    declared_host_capabilities,
    is_declared_host_capability,
)
from ._generated.diagnostics import (
    CAPABILITY_DISPLAY_NAME_REJECTED,
    CAPABILITY_REF_NOT_EXACT,
    CONTEXT_NOT_TYPED,
    DiagnosticCode,
    EVENT_NOT_TYPED,
    EVENT_WAIT_OUTSIDE_BODY,
    MODEL_DISPLAY_NAME_REJECTED,
    MODEL_UNTYPED_SCHEMA,
    SKILL_ENTRY_PATH_NOT_CANONICAL,
    SKILL_ID_NOT_EXACT,
    SKILL_INSTRUCTIONS_OVERLONG,
    SKILL_LOAD_OUTSIDE_BODY,
    SKILL_SOURCE_AMBIGUOUS,
    SKILL_SOURCE_MISSING,
    TOOL_DISPLAY_NAME_REJECTED,
    TOOL_REF_NOT_CAPABILITY,
)
from ._generated.frontend_graph import (
    SKILL_INSTRUCTION_KIND_ENTRY,
    SKILL_INSTRUCTION_KIND_INLINE,
)
from ._generated.frontend_records import (
    SkillInstructionSource,
)
from ._generated.permissions import PERMISSION_DECISIONS, Permission
from .handlers import CapabilityId

#: The ceiling the skill-reading capability enforces on a body it loads. An
#: inline skill is the same trusted context landing in the same model window,
#: so it is refused here rather than at execution.
MAX_INSTRUCTION_BYTES = 128 * 1024

I = TypeVar("I")
O = TypeVar("O")
T = TypeVar("T")


def _type_name(annotation: Any, fallback: str) -> str:
    if annotation is None:
        return fallback
    if isinstance(annotation, str):
        return annotation
    return getattr(annotation, "__name__", str(annotation))


class ModelBinding(NamedTuple):
    """An exact typed model target. Calling it records a model invocation."""

    target_ref: str
    input_type_ref: str = "ModelRequest"
    output_type_ref: str = "ModelResponse"

    def __call__(self, *args: Any, **kwargs: Any) -> Any:  # pragma: no cover
        raise RuntimeError("a Model is invoked inside a compiled Agent body")


class ToolBinding(NamedTuple):
    """A static model-callable capability reference."""

    target_ref: str
    input_type_ref: str = "ToolInput"
    output_type_ref: str = "ToolOutput"
    permission: Optional[Permission] = None

    def __call__(self, *args: Any, **kwargs: Any) -> Any:  # pragma: no cover
        raise RuntimeError("a Tool is invoked inside a compiled Agent body")


class CapabilityBinding(NamedTuple):
    """A static executable capability reference that is not a Tool."""

    target_ref: str
    input_type_ref: str = "CapabilityInput"
    output_type_ref: str = "CapabilityOutput"
    permission: Optional[Permission] = None
    output_annotation: Any = None

    def __call__(self, *args: Any, **kwargs: Any) -> Any:  # pragma: no cover
        raise RuntimeError("a Capability is invoked inside a compiled Agent body")


class EventRef(Generic[T]):
    """An opaque admitted reservation, never constructed by program source."""

    __slots__ = ()
    event_id: str
    generation: int

    def __new__(cls) -> EventRef[T]:
        raise TypeError("EventRef values enter through admitted input or Capability results")


class EventType(NamedTuple, Generic[T]):
    """A typed durable event reference whose wait records an event wait."""

    type_ref: str
    target_ref: str
    payload_type: type[T]

    def wait(self, reference: EventRef[T]) -> Awaitable[T]:  # pragma: no cover
        raise RuntimeError(
            f"{EVENT_WAIT_OUTSIDE_BODY}: an Event is awaited inside a compiled Agent body"
        )


class _SkillEntrySource(NamedTuple):
    """Immutable internal representation of a package-carried skill source."""

    kind: str
    path: str


class _SkillInlineSource(NamedTuple):
    """Immutable internal representation of an inline skill source."""

    kind: str
    text: str


class SkillDecl(NamedTuple):
    """A declared Agent Skill: instructions plus where they live.

    Loading one is an ordinary Capability invocation, not a construct of its
    own: ``await skill.load()`` records a ``capability.invoke`` on the
    skill-reading capability, so the authority to read the instructions is
    declared in the artifact like any other.
    """

    skill_id: str
    instruction_source: SkillInstructionSource

    def load(self, *args: Any, **kwargs: Any) -> Any:  # pragma: no cover
        raise RuntimeError(
            f"{SKILL_LOAD_OUTSIDE_BODY}: a Skill is loaded inside a compiled Agent body"
        )


class ContextSchema(tuple, Generic[T]):
    """The typed initial and persistent Program Context schema."""

    __slots__ = ()

    def __new__(cls, type_ref: str, default_present: bool, schema_type: type[T]) -> ContextSchema[T]:
        return tuple.__new__(cls, (type_ref, default_present, schema_type))

    @property
    def type_ref(self) -> str:
        return self[0]

    @property
    def default_present(self) -> bool:
        return self[1]

    @property
    def schema_type(self) -> type[T]:
        return self[2]


class _FrozenFactoryType(type):
    """Keep marker factory methods immutable after the frontend loads."""

    def __setattr__(cls, name: str, value: Any) -> None:
        raise AttributeError("authoring marker factories are immutable after import")


class _TypedFactory(metaclass=_FrozenFactoryType):
    """A marker whose type parameters are supplied by subscription.

    ``_type_parameters`` names them in order. It is the Python spelling of the
    same arity a TypeScript marker states as ``<Input, Output>``, which is what
    lets the surface conformance gate compare the two by argument shape rather
    than by trusting that a subscript exists.

    Subscription returns a *new* factory rather than the module singleton, so a
    marker can tell a declaration that stated its types from one that did not.
    """

    _type_parameters: tuple[str, ...] = ()

    def __init__(self, typed: bool = False) -> None:
        self._typed = typed
        self._type_arguments: tuple[Any, ...] = ()

    def __getitem__(self, types: Any) -> "_TypedFactory":
        supplied = types if isinstance(types, tuple) else (types,)
        if len(supplied) != len(self._type_parameters):
            expected = ", ".join(self._type_parameters)
            raise TypeError(f"{type(self).__name__} takes [{expected}]")
        factory = type(self)(typed=True)
        factory._type_arguments = supplied
        return factory


class _ModelFactory(_TypedFactory):
    _type_parameters = ("input", "output")

    def __call__(self, ref: str) -> ModelBinding:
        _require_exact_reference(ref, "Model", MODEL_DISPLAY_NAME_REJECTED)
        if not self._typed:
            raise ValueError(
                f"{MODEL_UNTYPED_SCHEMA}: Model states the request and response it "
                "carries as Model[Input, Output]"
            )
        return ModelBinding(target_ref=ref)


class _ToolFactory(_TypedFactory):
    _type_parameters = ("input", "output")

    def __call__(
        self, capability_ref: str, *, permission: Optional[Permission] = None
    ) -> ToolBinding:
        _require_exact_reference(capability_ref, "Tool", TOOL_DISPLAY_NAME_REJECTED)
        _require_capability_reference(capability_ref, "Tool", TOOL_REF_NOT_CAPABILITY)
        _require_permission(permission, "Tool")
        return ToolBinding(target_ref=capability_ref, permission=permission)


class _CapabilityFactory(_TypedFactory):
    _type_parameters = ("input", "output")

    def __call__(
        self, ref: str, *, permission: Optional[Permission] = None
    ) -> CapabilityBinding:
        _require_exact_reference(ref, "Capability", CAPABILITY_DISPLAY_NAME_REJECTED)
        _require_capability_reference(ref, "Capability", CAPABILITY_REF_NOT_EXACT)
        _require_permission(permission, "Capability")
        return CapabilityBinding(target_ref=ref, permission=permission, output_annotation=self._type_arguments[1] if self._typed else None)


class _TypedEventFactory(Generic[T]):
    """Binds a static Event payload type before its exact event reference."""

    def __init__(self, payload_type: type[T]) -> None:
        self._payload_type = payload_type

    def __call__(self, ref: str) -> EventType[T]:
        _require_exact_reference(ref, "Event", EVENT_NOT_TYPED)
        return EventType(type_ref=_type_name(self._payload_type, "Event"), target_ref=ref, payload_type=self._payload_type)


class _EventFactory(_TypedFactory):
    _type_parameters = ("payload",)

    def __getitem__(self, types: type[T]) -> _TypedEventFactory[T]:
        super().__getitem__(types)
        return _TypedEventFactory(types)

    def __call__(self, ref: str) -> EventType:
        _require_exact_reference(ref, "Event", EVENT_NOT_TYPED)
        raise TypeError(f"{EVENT_NOT_TYPED}: Event requires an explicit finite payload type")


def Context(schema: type[T]) -> ContextSchema[T]:
    """Declare a typed Program Context schema from one typed class."""
    if not isinstance(schema, type):
        raise TypeError(f"{CONTEXT_NOT_TYPED}: Context decorates one typed class")
    fields = [name for name in getattr(schema, "__annotations__", {}) if not name.startswith("__")]
    default_present = all(name in vars(schema) for name in fields)
    return ContextSchema(type_ref=schema.__name__, default_present=default_present, schema_type=schema)


def Skill(
    skill_id: str, *, entry: Optional[str] = None, text: Optional[str] = None
) -> SkillDecl:
    """Declare one Agent Skill, its instructions carried one of two ways.

    ``entry`` names the package file holding them, which the carrying package's
    integrity chain hashes. ``text`` writes them here, inside the source bundle
    the artifact digest already covers. They are the two routes an edit takes to
    the artifact digest, so exactly one is stated: both is a contradiction about
    where the instructions live, and neither leaves the skill with no body.
    """
    if not _is_exact_reference(skill_id):
        raise ValueError(
            f"{SKILL_ID_NOT_EXACT}: Skill accepts an exact typed reference, not a "
            f"display name '{skill_id}'"
        )
    if entry is not None and text is not None:
        raise ValueError(
            f"{SKILL_SOURCE_AMBIGUOUS}: Skill '{skill_id}' states both a package "
            "entry and inline text; instructions live in one place"
        )
    if entry is None and text is None:
        raise ValueError(
            f"{SKILL_SOURCE_MISSING}: Skill '{skill_id}' states neither a package "
            "entry nor inline text"
        )
    source: SkillInstructionSource
    if text is not None:
        if not text or len(text.encode("utf-8")) > MAX_INSTRUCTION_BYTES:
            raise ValueError(
                f"{SKILL_INSTRUCTIONS_OVERLONG}: Skill '{skill_id}' states an empty "
                f"or oversized body; a loaded skill is at most {MAX_INSTRUCTION_BYTES} bytes"
            )
        source = _SkillInlineSource(kind=SKILL_INSTRUCTION_KIND_INLINE, text=text)
    else:
        expected = _skill_entry_path(skill_id)
        if entry != expected:
            raise ValueError(
                f"{SKILL_ENTRY_PATH_NOT_CANONICAL}: Skill '{skill_id}' carries its "
                f"instructions at '{expected}', not '{entry}'"
            )
        source = _SkillEntrySource(kind=SKILL_INSTRUCTION_KIND_ENTRY, path=expected)
    return SkillDecl(skill_id=skill_id, instruction_source=source)


def _skill_entry_path(skill_id: str) -> str:
    """The one package path the folder contract recognizes for this skill."""
    return f"skills/{skill_id}/SKILL.md"


def _is_exact_reference(value: Any) -> bool:
    forbidden = {"default", "model.default", "support", "search-web", ""}
    return isinstance(value, str) and value not in forbidden


def _require_exact_reference(value: Any, marker: str, code: DiagnosticCode) -> None:
    if not _is_exact_reference(value):
        raise ValueError(
            f"{code}: {marker} accepts an exact typed reference, not a "
            f"display name '{value}'"
        )


def _require_capability_reference(
    value: Any, marker: str, code: DiagnosticCode
) -> None:
    """Refuse a Capability reference that neither catalogue nor package mints.

    TypeScript narrows this to a union and the compiler settles it; Python has
    no type-checker in the build, so the same closed set is settled here, where
    the binding is constructed. Two arms are admitted, and they are the two
    places a Capability can come from:

    * a builtin id the generated catalogue mints, which the compiler's builtin
      allowlist admits without any registration;
    * a ``host:<id>`` reference the package's ``agent.toml`` declares as
      ``[[capabilities.host]]``, which the runtime never executes and the
      embedding host answers; and
    * the :class:`CapabilityId` a ``capability(...)`` declaration returns, which
      carries the implementation it names.

    ``CapabilityId`` subclasses ``str`` so an author can still print it and
    compare it to the id it spells, but ``isinstance`` — not ``==`` — is what is
    asked here, so an equal bare string is not mistaken for the declaration that
    would have implemented it.
    """
    if isinstance(value, CapabilityId) or value in BUILTIN_CAPABILITIES:
        return
    if isinstance(value, str) and value.startswith(HOST_CAPABILITY_REF_PREFIX):
        if is_declared_host_capability(value):
            return
        declared = declared_host_capabilities()
        raise ValueError(
            f"{code}: {marker} names the host-fulfilled Capability '{value}', "
            "which the package manifest does not declare as "
            "[[capabilities.host]]. "
            + (
                "This package declares none"
                if not declared
                else "It declares " + ", ".join(declared)
            )
        )
    raise ValueError(
        f"{code}: {marker} accepts a builtin catalogue id, a host-fulfilled "
        f"'host:' reference the manifest declares, or the handler declaration "
        f"that implements one, not '{value}'"
    )


def _require_permission(value: Any, marker: str) -> None:
    """Keep runtime callers on the generated closed permission vocabulary.

    Python type annotations do not run when a package is imported, and a
    ``Permission`` dataclass can therefore be constructed with an invalid
    decision or an empty reason. Refuse those values while the authored marker
    is built so malformed ``requested_permission`` records never reach the
    FrontendGraph bridge.
    """
    if value is None:
        return
    if not isinstance(value, Permission):
        raise TypeError(
            f"{marker} permission is one of the generated Allow, Ask, or Deny "
            f"markers, not {value!r}"
        )
    if value.decision not in PERMISSION_DECISIONS:
        raise ValueError(
            f"{marker} permission decision must be one of "
            f"{', '.join(PERMISSION_DECISIONS)}, not {value.decision!r}"
        )
    if value.reason is not None and (
        not isinstance(value.reason, str) or not value.reason
    ):
        raise ValueError(f"{marker} permission reason must be a non-empty string")


Model = _ModelFactory()
Tool = _ToolFactory()
Capability = _CapabilityFactory()
Event = _EventFactory()
