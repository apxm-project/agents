"""A Python package declares the Capability handlers it ships."""

from __future__ import annotations

import pytest

from apxm_program import Capability, Tool
from apxm_program.capabilities import READ
from apxm_program.handlers import (
    CapabilityHandlerError,
    CapabilityId,
    answer,
    capability,
    handler_id,
    schema,
    text,
)


def propose_edit(args: dict) -> dict:
    """Return a before/after proposal without changing a file."""
    return answer({**args, "mutates": False})


EDIT = capability(
    {
        "name": "edit",
        "description": "Return a before/after proposal without changing a file.",
        "read_only": True,
        "input": schema(
            additional_properties=False,
            properties={
                "file_path": text(required=True, min_length=1),
                "note": text(required=False),
            },
        ),
        "run": propose_edit,
    }
)


def test_the_reference_and_the_implementation_are_one_object() -> None:
    # A program names the handler it ships rather than a string that has to
    # agree with one, so the two cannot come apart.
    assert isinstance(EDIT, CapabilityId)
    assert EDIT == "edit"
    binding = Tool[dict, dict](EDIT)
    assert binding.target_ref == "edit"
    assert EDIT.descriptor.handler_id == handler_id(
        propose_edit.__module__, "propose_edit"
    )


def test_an_invented_bare_reference_is_refused() -> None:
    # `edit` is the id this package's own handler declares, and a bare string
    # spelling it is exactly the second spelling the declaration object exists
    # to remove: it compares equal to the declaration and is still refused,
    # because being the declaration is what admits it, not equalling one.
    assert EDIT == "edit"
    for marker, code in ((Tool, "ToolRefNotCapability"), (Capability, "CapabilityRefNotExact")):
        for invented in ("edit", "cap.search"):
            with pytest.raises(ValueError) as rejection:
                marker[dict, dict](invented)
            assert str(rejection.value).startswith(f"{code}: ")


def test_a_builtin_catalogue_id_is_still_a_reference() -> None:
    # The other admitted arm: an id the generated catalogue mints needs no
    # package handler, because the compiler's builtin allowlist admits it.
    assert Tool[dict, dict](READ).target_ref == "read"
    assert Capability[dict, dict](READ).target_ref == "read"


def test_required_is_stated_on_the_property_it_describes() -> None:
    declared = EDIT.descriptor.schema
    # The required list is derived from the properties, so no second spelling
    # of the same fact can drift from the first.
    assert declared["required"] == ["file_path"]
    assert set(declared["properties"]) == {"file_path", "note"}
    assert declared["additionalProperties"] is False


def test_the_manifest_entry_carries_the_handler_s_own_read_only_declaration() -> None:
    entry = EDIT.descriptor.manifest_entry()
    assert entry["read_only"] is True
    assert entry["name"] == "edit"
    assert entry["schema"] == EDIT.descriptor.schema
    # Only a packaging build knows what it bundled, so the author states no
    # artifact-local source.
    assert "source" not in entry


@pytest.mark.parametrize(
    "overrides, code",
    [
        ({"read_only": None}, "CapabilityHandlerReadOnlyUndeclared"),
        ({"description": ""}, "CapabilityHandlerUntypedSchema"),
        ({"input": {"type": "object", "properties": {}}}, "CapabilityHandlerOpenObject"),
    ],
)
def test_an_unstated_decision_is_refused_rather_than_defaulted(
    overrides: dict, code: str
) -> None:
    definition = {
        "name": "edit",
        "description": "Return a before/after proposal without changing a file.",
        "read_only": True,
        "input": schema(
            additional_properties=False,
            properties={"file_path": text(required=True)},
        ),
        "run": propose_edit,
        **overrides,
    }
    with pytest.raises(CapabilityHandlerError) as rejection:
        capability(definition)
    assert rejection.value.code == code


def test_an_object_that_would_silently_accept_undeclared_arguments_is_refused() -> None:
    with pytest.raises(CapabilityHandlerError):
        schema(additional_properties=None, properties={"a": text(required=True)})
    with pytest.raises(CapabilityHandlerError):
        schema(additional_properties=False, properties={})
    with pytest.raises(CapabilityHandlerError):
        schema(additional_properties=False, properties={"a": {"type": "string"}})
