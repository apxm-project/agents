"""Build DSPy signatures from {N}-style APXM templates."""

from __future__ import annotations

import re

_PLACEHOLDER_RE = re.compile(r"\{(\d+)\}")


def extract_placeholders(template_str: str) -> list[int]:
    """Extract {N} placeholder indices from a template string."""
    return sorted(set(int(m.group(1)) for m in _PLACEHOLDER_RE.finditer(template_str)))


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
        input_fields = [f"context_{i}" for i in placeholders]
    else:
        input_fields = ["context_0"]
    return input_fields, "output"


def template_to_signature_str(
    template_str: str,
    training_data: list[dict],
) -> str:
    """Convert template + training data to a DSPy signature string.

    Example: "context_0, context_1 -> output"
    """
    input_fields, output_field = extract_fields_from_template(template_str, training_data)
    return ", ".join(input_fields) + " -> " + output_field
