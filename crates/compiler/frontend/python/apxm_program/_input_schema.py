"""Project finite JSON input schemas from resolved Python type declarations."""

from __future__ import annotations

from typing import Any, NotRequired, Required, get_args, get_origin, get_type_hints, is_typeddict

from ._generated.frontend_graph import (
    INPUT_SCHEMA_TYPE_ARRAY,
    INPUT_SCHEMA_TYPE_BOOLEAN,
    INPUT_SCHEMA_TYPE_INTEGER,
    INPUT_SCHEMA_TYPE_NULL,
    INPUT_SCHEMA_TYPE_NUMBER,
    INPUT_SCHEMA_TYPE_OBJECT,
    INPUT_SCHEMA_TYPE_STRING,
)
from ._generated.frontend_records import EntrypointInputSchema

# Mirror of crates/machine/program/src/input_schema.rs.
_MAX_SCHEMA_DEPTH = 32
_MAX_SCHEMA_NODES = 512


def checked_input_schema(annotation: Any, bindings: dict[str, Any]) -> EntrypointInputSchema | None:
    """Leave unsupported or unresolved types ineligible for schema-bound admission."""
    ancestors: set[int] = set()
    nodes = 0

    def project(value: Any, depth: int) -> EntrypointInputSchema | None:
        nonlocal nodes
        nodes += 1
        if depth > _MAX_SCHEMA_DEPTH or nodes > _MAX_SCHEMA_NODES or id(value) in ancestors:
            return None
        from ._markers import EventRef
        if get_origin(value) is EventRef:
            return EntrypointInputSchema(
                type=INPUT_SCHEMA_TYPE_OBJECT,
                properties={"event_id": EntrypointInputSchema(type=INPUT_SCHEMA_TYPE_STRING), "generation": EntrypointInputSchema(type=INPUT_SCHEMA_TYPE_INTEGER)},
                required=("event_id", "generation"), additionalProperties=False,
            )
        for scalar, kind in (
            (str, INPUT_SCHEMA_TYPE_STRING),
            (float, INPUT_SCHEMA_TYPE_NUMBER),
            (int, INPUT_SCHEMA_TYPE_INTEGER),
            (bool, INPUT_SCHEMA_TYPE_BOOLEAN),
            (type(None), INPUT_SCHEMA_TYPE_NULL),
        ):
            if value is scalar:
                return EntrypointInputSchema(type=kind)
        ancestors.add(id(value))
        try:
            if get_origin(value) is list:
                arguments = get_args(value)
                items = project(arguments[0], depth + 1) if len(arguments) == 1 else None
                return EntrypointInputSchema(type=INPUT_SCHEMA_TYPE_ARRAY, items=items) if items else None
            if not is_typeddict(value):
                return None
            try:
                annotations = get_type_hints(value, globalns=bindings, localns=bindings, include_extras=True)
            except (NameError, TypeError, ValueError, RecursionError):
                return None
            properties: dict[str, EntrypointInputSchema] = {}
            required: list[str] = []
            for name, field_type in annotations.items():
                qualifier = get_origin(field_type)
                required_field = name in value.__required_keys__
                if qualifier in (Required, NotRequired):
                    required_field = qualifier is Required
                    field_type = get_args(field_type)[0]
                field_schema = project(field_type, depth + 1)
                if field_schema is None:
                    return None
                properties[name] = field_schema
                if required_field:
                    required.append(name)
            return EntrypointInputSchema(
                type=INPUT_SCHEMA_TYPE_OBJECT,
                properties=properties,
                required=tuple(required),
                additionalProperties=False,
            )
        finally:
            ancestors.remove(id(value))

    return project(annotation, 0)
