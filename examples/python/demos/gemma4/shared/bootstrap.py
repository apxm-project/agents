"""Bootstrap sys.path so workflows can be invoked directly with `python3`.

A workflow file under `workflows/0N_*.py` calls `bootstrap_paths()` at import
time. It walks up to the APXM repo root, then injects the in-tree
`apxm-frontend` Python package and the demo's own `shared/` directory so the
file can be run from any cwd without `pip install`.
"""

from __future__ import annotations

import sys
from pathlib import Path


_REPO_MARKERS = ("Cargo.toml", "crates")


def _find_repo_root(start: Path) -> Path:
    for candidate in (start.resolve(), *start.resolve().parents):
        if all((candidate / marker).exists() for marker in _REPO_MARKERS):
            return candidate
    return Path.cwd().resolve()


def bootstrap_paths(file: str) -> None:
    """Inject `apxm-frontend/python` and `<demo>/` onto sys.path."""

    here = Path(file).resolve()
    repo_root = _find_repo_root(here)
    frontend = repo_root / "crates" / "compiler" / "apxm-frontend" / "python"
    if frontend.exists() and str(frontend) not in sys.path:
        sys.path.insert(0, str(frontend))

    # The demo root holds the shared/ package.
    # workflows/0N_<name>.py -> parents[1] is the demo root.
    demo_root = here.parents[1]
    if str(demo_root) not in sys.path:
        sys.path.insert(0, str(demo_root))


__all__ = ["bootstrap_paths"]
