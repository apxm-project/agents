#!/usr/bin/env python3
"""APXM data-layout configuration resolver.

Resolves where the roaming buckets (HF model cache, saved Docker
image store) live by walking env vars and the project config,
falling back to ``<repo>/.apxm/<bucket>/``.

Operator reference: ``docs/backends/storage-layout.md``.
"""

from __future__ import annotations

import os
import shutil
import tomllib
from dataclasses import dataclass
from pathlib import Path

CONFIG_FILENAME = "config.toml"
EXAMPLE_FILENAME = "config.example.toml"
WORKSPACE = ".apxm"
DEFAULT_HF_CACHE = "huggingface"
DEFAULT_IMAGE_STORE = "vllm-images"
SUPPORTED_VERSIONS = frozenset({1})


@dataclass(frozen=True)
class DataLayout:
    """Resolved paths plus a source tag per field (for ``doctor`` output)."""

    repo_root: Path
    data_dir: Path
    hf_cache: Path
    hf_cache_roots: tuple[Path, ...]
    model_roots: tuple[Path, ...]
    image_store: Path
    sources: dict[str, str]


def _load_toml(path: Path) -> dict:
    if not path.is_file():
        return {}
    with path.open("rb") as f:
        data = tomllib.load(f)
    version = data.get("schema_version")
    if version is not None and version not in SUPPORTED_VERSIONS:
        raise SystemExit(
            f"unsupported schema_version={version!r} in {path}; "
            f"supported: {sorted(SUPPORTED_VERSIONS)}"
        )
    return data


def _coerce_path(value: object) -> Path | None:
    if not isinstance(value, str):
        return None
    expanded = os.path.expandvars(value.strip())
    return Path(expanded).expanduser() if expanded else None


def _coerce_path_list(value: object) -> tuple[Path, ...]:
    if value is None:
        return ()
    if isinstance(value, str):
        raw_items = [item for item in value.split(os.pathsep)]
    elif isinstance(value, list):
        raw_items = value
    else:
        return ()

    paths: list[Path] = []
    for item in raw_items:
        path = _coerce_path(item)
        if path is not None:
            paths.append(path)
    return tuple(paths)


def _dedupe_paths(paths: tuple[Path, ...]) -> tuple[Path, ...]:
    unique: list[Path] = []
    seen: set[str] = set()
    for path in paths:
        key = str(path.resolve(strict=False))
        if key in seen:
            continue
        seen.add(key)
        unique.append(path)
    return tuple(unique)


def _nested(d: dict, *keys: str) -> object:
    cur: object = d
    for key in keys:
        if not isinstance(cur, dict):
            return None
        cur = cur.get(key)
    return cur


def _resolve_field(
    *,
    field: str,
    env_var: str,
    config_keys: tuple[str, ...],
    fallback: Path,
    fallback_source: str,
    environ: dict[str, str] | os._Environ[str],
    project_cfg: dict,
    sources: dict[str, str],
) -> Path:
    raw = environ.get(env_var, "").strip()
    if raw:
        sources[field] = f"env:{env_var}"
        return Path(os.path.expandvars(raw)).expanduser()
    val = _coerce_path(_nested(project_cfg, *config_keys))
    if val is not None:
        sources[field] = f"config:project:{'.'.join(config_keys)}"
        return val
    sources[field] = fallback_source
    return fallback


def _resolve_path_list_field(
    *,
    field: str,
    env_var: str,
    config_keys: tuple[str, ...],
    fallback: tuple[Path, ...],
    fallback_source: str,
    environ: dict[str, str] | os._Environ[str],
    project_cfg: dict,
    sources: dict[str, str],
) -> tuple[Path, ...]:
    raw = environ.get(env_var, "").strip()
    if raw:
        sources[field] = f"env:{env_var}"
        return _dedupe_paths(_coerce_path_list(raw))
    val = _coerce_path_list(_nested(project_cfg, *config_keys))
    if val:
        sources[field] = f"config:project:{'.'.join(config_keys)}"
        return _dedupe_paths(val)
    sources[field] = fallback_source
    return _dedupe_paths(fallback)


def resolve_data_layout(
    repo_root: Path,
    *,
    environ: dict[str, str] | os._Environ[str] = os.environ,
) -> DataLayout:
    """Resolve the data layout. Reads ``config.toml`` but creates nothing
    on disk; callers do their own ``mkdir -p``."""
    project_cfg = _load_toml(repo_root / WORKSPACE / CONFIG_FILENAME)
    sources: dict[str, str] = {}

    data_dir = _resolve_field(
        field="data.dir",
        env_var="APXM_HOME",
        config_keys=("data", "dir"),
        fallback=repo_root / WORKSPACE,
        fallback_source="default",
        environ=environ,
        project_cfg=project_cfg,
        sources=sources,
    )
    hf_cache = _resolve_field(
        field="hf_cache",
        env_var="APXM_VLLM_HF_HOME",
        config_keys=("data", "vllm", "hf_cache"),
        fallback=data_dir / DEFAULT_HF_CACHE,
        fallback_source="derived:data.dir",
        environ=environ,
        project_cfg=project_cfg,
        sources=sources,
    )
    extra_hf_cache_roots = _resolve_path_list_field(
        field="hf_cache_roots",
        env_var="APXM_VLLM_HF_CACHE_ROOTS",
        config_keys=("data", "vllm", "hf_cache_roots"),
        fallback=(),
        fallback_source="default",
        environ=environ,
        project_cfg=project_cfg,
        sources=sources,
    )
    hf_cache_roots = _dedupe_paths((hf_cache, *extra_hf_cache_roots))
    model_roots = _resolve_path_list_field(
        field="model_roots",
        env_var="APXM_VLLM_MODEL_ROOTS",
        config_keys=("data", "vllm", "model_roots"),
        fallback=(),
        fallback_source="default",
        environ=environ,
        project_cfg=project_cfg,
        sources=sources,
    )
    image_store = _resolve_field(
        field="image_store",
        env_var="APXM_VLLM_IMAGE_STORE",
        config_keys=("data", "vllm", "image_store"),
        fallback=data_dir / DEFAULT_IMAGE_STORE,
        fallback_source="derived:data.dir",
        environ=environ,
        project_cfg=project_cfg,
        sources=sources,
    )
    return DataLayout(repo_root, data_dir, hf_cache, hf_cache_roots, model_roots, image_store, sources)


def materialize_config(repo_root: Path) -> bool:
    """Copy ``config.example.toml`` to ``config.toml`` if missing.
    Idempotent; never overwrites an existing realized file."""
    workspace = repo_root / WORKSPACE
    workspace.mkdir(parents=True, exist_ok=True)
    realized = workspace / CONFIG_FILENAME
    if realized.is_file():
        return False
    example = workspace / EXAMPLE_FILENAME
    if not example.is_file():
        return False
    shutil.copyfile(example, realized)
    return True


def format_layout(layout: DataLayout, *, repo_root_label: str | None = None) -> str:
    """Multi-line ``doctor``-style report of the resolved layout."""
    rows = [
        ("data.dir", layout.data_dir, layout.sources["data.dir"]),
        ("hf_cache", layout.hf_cache, layout.sources["hf_cache"]),
        ("image_store", layout.image_store, layout.sources["image_store"]),
    ]
    key_w = max(len(k) for k, _, _ in rows)
    path_w = max(len(str(p)) for _, p, _ in rows)
    lines = ["APXM data layout (effective)"]
    for key, path, source in rows:
        lines.append(f"  {key:<{key_w}}  {str(path):<{path_w}}  [{source}]")
    hf_roots = os.pathsep.join(str(path) for path in layout.hf_cache_roots) or "<none>"
    model_roots = os.pathsep.join(str(path) for path in layout.model_roots) or "<none>"
    lines.append(f"  hf_cache_roots  {hf_roots}  [{layout.sources['hf_cache_roots']}]")
    lines.append(f"  model_roots     {model_roots}  [{layout.sources['model_roots']}]")
    lines.append(f"  repo_root    {repo_root_label or layout.repo_root}")
    return "\n".join(lines)
