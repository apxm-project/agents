"""Normalization utilities for APXM graph attribute values.

Converts typed Python objects (ModelId, ProviderSpec, dataclasses) to
JSON-serializable primitives for the graph wire format.
"""

from __future__ import annotations

from dataclasses import asdict, is_dataclass
from typing import Any


def normalize_value(value: Any) -> Any:
    """Normalize a single attribute value to a JSON-serializable form."""
    try:
        from ._generated.models import ModelId
    except ImportError:
        ModelId = None  # type: ignore[assignment,misc]
    if ModelId is not None and isinstance(value, ModelId):
        return str(value)
    if hasattr(value, "to_dict") and callable(value.to_dict):
        return value.to_dict()
    if is_dataclass(value):
        return asdict(value)
    if isinstance(value, tuple):
        return [normalize_value(v) for v in value]
    if isinstance(value, list):
        return [normalize_value(v) for v in value]
    if isinstance(value, dict):
        return {k: normalize_value(v) for k, v in value.items()}
    return value


def normalize_provider(value: Any) -> str:
    """Extract provider id string from ProviderSpec or return string as-is."""
    try:
        from ._generated.providers import ProviderSpec
    except ImportError:
        ProviderSpec = None  # type: ignore[assignment,misc]
    if ProviderSpec is not None and isinstance(value, ProviderSpec):
        return value.id
    return str(value)


def normalize_attributes(attributes: dict[str, Any]) -> dict[str, Any]:
    """Normalize all non-None attribute values in a dict."""
    return {key: normalize_value(value) for key, value in attributes.items() if value is not None}
