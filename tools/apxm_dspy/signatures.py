"""Build DSPy signatures from named-placeholder APXM templates."""

from __future__ import annotations

import re

_PLACEHOLDER_RE = re.compile(r"\{(\w+)\}")


def extract_placeholders(template_str: str) -> list[str]:
    """Extract `{name}` placeholder names from a template string, in order
    of first appearance, with duplicates removed."""
    seen: dict[str, None] = {}
    for m in _PLACEHOLDER_RE.finditer(template_str):
        seen.setdefault(m.group(1), None)
    return list(seen.keys())


def extract_fields_from_template(
    template_str: str,
    training_data: list[dict],
) -> tuple[list[str], str]:
    """Determine input field names and output field name.

    Returns (input_fields, output_field).
    """
    # Try to infer from training data keys
    if training_data:
        first = training_data[0]
        input_keys = sorted(first.get("inputs", {}).keys())
        if input_keys:
            return input_keys, "output"

    # Fall back to placeholder-based naming
    placeholders = extract_placeholders(template_str)
    if placeholders:
        input_fields = [f"context_{name}" for name in placeholders]
    else:
        input_fields = ["context_input"]
    return input_fields, "output"


def template_to_signature_str(
    template_str: str,
    training_data: list[dict],
) -> str:
    """Convert template + training data to a DSPy signature string.

    Example: "context_topic, context_question -> output"
    """
    input_fields, output_field = extract_fields_from_template(template_str, training_data)
    return ", ".join(input_fields) + " -> " + output_field
