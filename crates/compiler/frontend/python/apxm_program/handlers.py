"""Authoring the Capability handlers a Python Agent package ships.

A package that only *references* Capabilities is a package that can only use
what someone else shipped. This is the other half: the declaration that says
*here is the implementation*, in the same shape the TypeScript packaging surface
uses, so a Python package is a conforming implementation of the frontend surface
rather than a consumer of one.

Two inversions make the declaration hard to misuse:

* **`required` is on the property.** There is no sibling `required` list to fall
  out of step with the properties it names — a field says whether it is required
  where it says what it is.
* **`additional_properties` is stated, never defaulted.** Whether a Capability
  accepts arguments it does not declare is a decision, and an unstated decision
  is the permissive one exactly when that is worst.

The argument type follows from the schema: the properties are the only place the
shape is written, so the handler cannot declare one shape and validate another.

``capability(...)`` returns a :class:`CapabilityId` — the id *is* the
implementation. A program references the handler it ships by naming the object
that implements it, so referencing one Capability while implementing another is
not a mistake this surface can express.

**A Python declaration is not yet a dispatchable implementation.** Executing a
package handler takes three things beyond the declaration: a deterministic
bundler that turns the callable into the manifest's artifact-local ``source``,
a private worker adapter a composition root can be supplied with, and a
``language`` this manifest admits — and ``apxm.handler-manifest`` admits only
``typescript`` today. Until all three land, ``apxm agent`` grants a package
exactly the ids it ships a ``capabilities/<id>/handler.ts`` for, so a program
naming a Python-declared handler is refused at compile rather than admitted
against an implementation that is not there. The declaration is honest about
what it is: the shape a package handler states about itself, in the fields the
manifest names, waiting on the bundler that can carry it.
"""

from __future__ import annotations

import hashlib
import inspect
from typing import Any, Callable, Mapping, Optional, TypedDict

from ._generated.diagnostics import (
    CAPABILITY_HANDLER_OPEN_OBJECT,
    CAPABILITY_HANDLER_READ_ONLY_UNDECLARED,
    CAPABILITY_HANDLER_UNTYPED_SCHEMA,
    DiagnosticCode,
)

__all__ = [
    "CapabilityHandlerError",
    "CapabilityId",
    "answer",
    "capability",
    "integer",
    "schema",
    "text",
]

#: The envelope one handler returns, mirroring the TypeScript packaging surface.
ANSWER_KIND = "apxm.tool-answer"

#: Content-addressed handler identity prefix, per `apxm.handler-manifest`.
HANDLER_ID_PREFIX = "sha256:"


class CapabilityHandlerError(ValueError):
    """A handler declaration outside the closed authoring shape."""

    def __init__(self, code: DiagnosticCode, detail: str) -> None:
        super().__init__(f"{code}: {detail}")
        self.code = code


class CapabilityId(str):
    """The exact Capability reference a shipped handler both names and implements.

    It is a ``str``, so it is accepted wherever an exact Capability reference is
    — ``Tool[I, O](my_capability)`` — and it carries the descriptor, so the
    reference and the implementation are one object rather than two spellings
    that have to agree.
    """

    __slots__ = ("descriptor",)

    def __new__(cls, name: str, descriptor: "HandlerDescriptor") -> "CapabilityId":
        identity = super().__new__(cls, name)
        object.__setattr__(identity, "descriptor", descriptor)
        return identity


class HandlerDescriptor:
    """One shipped handler, in the fields `apxm.handler-manifest` names."""

    __slots__ = (
        "name",
        "description",
        "read_only",
        "schema",
        "run",
        "handler_id",
        "module",
        "qualname",
    )

    def __init__(
        self,
        *,
        name: str,
        description: str,
        read_only: bool,
        schema: Mapping[str, Any],
        run: Callable[..., Any],
    ) -> None:
        self.name = name
        self.description = description
        self.read_only = read_only
        self.schema = schema
        self.run = run
        self.module = getattr(run, "__module__", None) or "__unknown__"
        self.qualname = getattr(run, "__name__", None) or "handler"
        self.handler_id = handler_id(self.module, self.qualname)

    def manifest_entry(self) -> dict[str, Any]:
        """The portable handler-manifest fields this declaration states.

        The artifact-local `source` is not one of them: only a packaging build
        knows what it bundled, so it supplies that rather than the author.
        """
        return {
            "kind": "tool",
            "handler_id": self.handler_id,
            "module": self.module,
            "qualname": self.qualname,
            "name": self.name,
            "description": self.description,
            "schema": dict(self.schema),
            "read_only": self.read_only,
        }


class Property(dict):
    """One declared argument: its type, its constraints, and whether it is required.

    ``required`` lives here rather than in a sibling list, so there is no second
    place that can disagree about which arguments a Capability needs.
    """

    def __init__(self, *, required: bool, **schema: Any) -> None:
        super().__init__(schema)
        self.required = required


class CapabilityDefinition(TypedDict):
    """Everything one shipped Capability handler states about itself."""

    name: str
    description: str
    read_only: bool
    input: Mapping[str, Any]
    run: Callable[..., Any]


def text(
    *, required: bool, min_length: Optional[int] = None, description: str = ""
) -> Property:
    """Declare one string argument."""
    constraints: dict[str, Any] = {"type": "string"}
    if min_length is not None:
        constraints["minLength"] = min_length
    if description:
        constraints["description"] = description
    return Property(required=required, **constraints)


def integer(
    *, required: bool, minimum: Optional[int] = None, description: str = ""
) -> Property:
    """Declare one integer argument."""
    constraints: dict[str, Any] = {"type": "integer"}
    if minimum is not None:
        constraints["minimum"] = minimum
    if description:
        constraints["description"] = description
    return Property(required=required, **constraints)


def schema(
    *, additional_properties: bool, properties: Mapping[str, Property]
) -> dict[str, Any]:
    """Build the JSON Schema object one Capability accepts.

    ``additional_properties`` has no default: an object that silently accepts
    arguments it never declared is a decision, and it is made here or not at all.
    """
    if not isinstance(additional_properties, bool):
        raise CapabilityHandlerError(
            CAPABILITY_HANDLER_OPEN_OBJECT,
            "a Capability states whether it accepts undeclared arguments",
        )
    if not properties:
        raise CapabilityHandlerError(
            CAPABILITY_HANDLER_UNTYPED_SCHEMA,
            "a Capability declares at least one typed argument",
        )
    for field, declaration in properties.items():
        if not isinstance(declaration, Property):
            raise CapabilityHandlerError(
                CAPABILITY_HANDLER_UNTYPED_SCHEMA,
                f"argument '{field}' is declared with a typed handler property",
            )
    return {
        "type": "object",
        "properties": {name: dict(value) for name, value in properties.items()},
        # Derived from the properties, never authored beside them.
        "required": [name for name, value in properties.items() if value.required],
        "additionalProperties": additional_properties,
    }


def answer(value: Mapping[str, Any]) -> dict[str, Any]:
    """Return one typed business result from a handler body."""
    if not isinstance(value, Mapping):
        raise CapabilityHandlerError(
            CAPABILITY_HANDLER_UNTYPED_SCHEMA,
            "a Capability answers with one structured result",
        )
    return {"kind": ANSWER_KIND, "value": dict(value)}


def capability(definition: CapabilityDefinition) -> CapabilityId:
    """Declare one Capability this package implements, and return its exact id."""
    name = definition.get("name")
    description = definition.get("description")
    read_only = definition.get("read_only")
    argument_schema = definition.get("input")
    run = definition.get("run")

    if not isinstance(name, str) or not name:
        raise CapabilityHandlerError(
            CAPABILITY_HANDLER_UNTYPED_SCHEMA, "a Capability handler states its name"
        )
    if not isinstance(description, str) or not description:
        raise CapabilityHandlerError(
            CAPABILITY_HANDLER_UNTYPED_SCHEMA,
            f"Capability '{name}' states what it does",
        )
    if not isinstance(read_only, bool):
        raise CapabilityHandlerError(
            CAPABILITY_HANDLER_READ_ONLY_UNDECLARED,
            f"Capability '{name}' states whether it mutates state outside itself",
        )
    if not isinstance(argument_schema, Mapping) or argument_schema.get("type") != "object":
        raise CapabilityHandlerError(
            CAPABILITY_HANDLER_UNTYPED_SCHEMA,
            f"Capability '{name}' accepts one declared object schema",
        )
    if "additionalProperties" not in argument_schema:
        raise CapabilityHandlerError(
            CAPABILITY_HANDLER_OPEN_OBJECT,
            f"Capability '{name}' states whether it accepts undeclared arguments",
        )
    if not callable(run) or inspect.isclass(run):
        raise CapabilityHandlerError(
            CAPABILITY_HANDLER_UNTYPED_SCHEMA,
            f"Capability '{name}' is implemented by one callable",
        )

    return CapabilityId(
        name,
        HandlerDescriptor(
            name=name,
            description=description,
            read_only=read_only,
            schema=argument_schema,
            run=run,
        ),
    )


def handler_id(module: str, qualname: str) -> str:
    """Return the stable content-addressed identity of one package handler."""
    digest = hashlib.sha256(f"{module}:{qualname}".encode("utf-8")).hexdigest()
    return f"{HANDLER_ID_PREFIX}{digest}"
