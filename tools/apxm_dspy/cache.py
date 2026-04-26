"""Content-addressed cache for DSPy optimization results."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path

from .types import ENV_NO_CACHE, TRUE_ENV_VALUES


def compute_cache_key(
    template_str: str,
    training_data_hash: str,
    optimizer: str,
    model: str,
    metric: str,
    auto: str,
    backend_fingerprint: str,
    dspy_version: str,
) -> str:
    """Compute BLAKE3-style cache key (using SHA256 as fallback).

    The key incorporates template content, training data, optimizer settings,
    backend identity, and DSPy version so that behavior changes invalidate the
    cache.
    """
    try:
        h = hashlib.blake2b(digest_size=32)
    except (AttributeError, ValueError):
        h = hashlib.sha256()

    h.update(template_str.encode("utf-8"))
    h.update(training_data_hash.encode("utf-8"))
    h.update(optimizer.encode("utf-8"))
    h.update(model.encode("utf-8"))
    h.update(metric.encode("utf-8"))
    h.update(auto.encode("utf-8"))
    h.update(backend_fingerprint.encode("utf-8"))
    h.update(dspy_version.encode("utf-8"))
    return h.hexdigest()


def hash_training_data(training_data: list[dict]) -> str:
    """Hash training data for cache key computation."""
    canonical = json.dumps(training_data, sort_keys=True, ensure_ascii=True)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def cache_dir(cache_root: Path) -> Path:
    """Return the compiler-provided DSPy result cache directory."""
    return cache_root


def _no_cache(disabled: bool = False) -> bool:
    """Check if caching is disabled via APXM_NO_CACHE."""
    return disabled or os.environ.get(ENV_NO_CACHE, "").lower() in TRUE_ENV_VALUES


def load_cached(
    cache_key: str,
    cache_root: Path,
    *,
    disabled: bool = False,
) -> dict | None:
    """Load a cached optimization result, or None if not found."""
    if _no_cache(disabled):
        return None

    path = cache_dir(cache_root) / f"{cache_key}.json"
    if not path.exists():
        return None

    try:
        with open(path) as f:
            return json.load(f)
    except (json.JSONDecodeError, OSError):
        return None


def store_cached(
    cache_key: str,
    result: dict,
    cache_root: Path,
    *,
    disabled: bool = False,
) -> None:
    """Store an optimization result in the cache."""
    if _no_cache(disabled):
        return

    path = cache_dir(cache_root) / f"{cache_key}.json"
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "w") as f:
            json.dump(result, f, indent=2)
    except OSError:
        pass  # Graceful degradation
