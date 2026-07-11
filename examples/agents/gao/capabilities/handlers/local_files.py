from __future__ import annotations

from pathlib import Path

from apxm import tool


AGENT_ROOT = Path(__file__).resolve().parents[2]


@tool(name="list_files")
def list_files(relative_path: str = ".") -> str:
    """List files below a package-relative read root."""

    root = (AGENT_ROOT / relative_path).resolve()
    if AGENT_ROOT not in root.parents and root != AGENT_ROOT:
        raise ValueError("path escapes the agent package")
    if not root.is_dir():
        raise ValueError("read root is not a directory")
    return "\n".join(
        str(path.relative_to(AGENT_ROOT))
        for path in sorted(root.iterdir())
        if path.is_file()
    )
