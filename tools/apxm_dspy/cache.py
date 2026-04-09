"""Content-addressed cache for DSPy optimization results."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path


def compute_cache_key(
    template_str: str,
    training_data_hash: str,
    optimizer: str,
    model: str,
) -> str:
    """Compute BLAKE3-style cache key (using SHA256 as fallback).

    The key incorporates template content, training data, optimizer, and model
    so that any change invalidates the cache.
    """
    try:
        h = hashlib.blake2b(digest_size=32)
    except (AttributeError, ValueError):
        h = hashlib.sha256()

    h.update(template_str.encode("utf-8"))
    h.update(training_data_hash.encode("utf-8"))
    h.update(optimizer.encode("utf-8"))
    h.update(model.encode("utf-8"))
    return h.hexdigest()


def hash_training_data(training_data: list[dict]) -> str:
    """Hash training data for cache key computation."""
    canonical = json.dumps(training_data, sort_keys=True, ensure_ascii=True)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def cache_dir(project_root: Path | None = None) -> Path:
    """Get the DSPy cache directory.

    Looks for <repo>/.apxm/cache/dspy/, walking up from cwd if no root given.
    """
    if project_root is None:
        cwd = Path.cwd()
        for ancestor in [cwd] + list(cwd.parents):
            candidate = ancestor / ".apxm"
            if candidate.is_dir():
                project_root = ancestor
                break
        if project_root is None:
            project_root = cwd

    return project_root / ".apxm" / "cache" / "dspy"


def _no_cache() -> bool:
    """Check if caching is disabled via APXM_NO_CACHE."""
    return os.environ.get("APXM_NO_CACHE", "").lower() in ("1", "true")


def load_cached(cache_key: str, project_root: Path | None = None) -> dict | None:
    """Load a cached optimization result, or None if not found."""
    if _no_cache():
        return None

    path = cache_dir(project_root) / f"{cache_key}.json"
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
    project_root: Path | None = None,
) -> None:
    """Store an optimization result in the cache."""
    if _no_cache():
        return

    path = cache_dir(project_root) / f"{cache_key}.json"
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "w") as f:
            json.dump(result, f, indent=2)
    except OSError:
        pass  # Graceful degradation
