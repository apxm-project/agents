"""Path helpers used by APXM examples and local scripts."""

from __future__ import annotations

import os
from pathlib import Path

from apxm.constants import ENV_APXM_HOME

_APXM_DIR = ".apxm"
_CRATES_DIR = "crates"
_WORKSPACE_MANIFEST = "Cargo.toml"
_GIT_DIR = ".git"


def find_repo_root(start: str | os.PathLike[str] | None = None) -> Path:
    """Return the nearest APXM checkout root, falling back to the current cwd.

    A directory qualifies if it contains the APXM workspace
    (`Cargo.toml` + `crates/`) or, for sibling repos like apxm-eval,
    a `.git` entry. The `.git` fallback lets companion repos use the
    `.apxm/` artifact convention without forking the helper.
    """

    current = Path(start or os.getcwd()).resolve()
    if current.is_file():
        current = current.parent

    for candidate in (current, *current.parents):
        if (candidate / _WORKSPACE_MANIFEST).is_file() and (candidate / _CRATES_DIR).is_dir():
            return candidate
        if (candidate / _GIT_DIR).exists():
            return candidate

    return Path.cwd().resolve()


def repo_path(*parts: str) -> Path:
    """Build an absolute path inside the current APXM checkout."""

    return find_repo_root().joinpath(*parts)


def local_apxm_path(*parts: str) -> Path:
    """Build an absolute path under the checkout-local .apxm directory."""

    return repo_path(_APXM_DIR, *parts)


def agent_cwd() -> str:
    """Return the cwd used by coding-agent examples."""

    explicit = os.environ.get(ENV_APXM_HOME)
    if explicit:
        return explicit
    return str(find_repo_root())
