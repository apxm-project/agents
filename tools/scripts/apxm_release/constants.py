"""Shared release paths and constants."""

from __future__ import annotations

from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
PYTHON_PROJECT = REPO_ROOT / "crates" / "compiler" / "apxm-frontend" / "python"
RELEASE_ROOT = REPO_ROOT / ".apxm" / "releases"
TARGET_RELEASE_DIR = REPO_ROOT / "target" / "release"
CHECKSUM_FILE = "SHA256SUMS"
INTERNAL_PREFIX = "apxm"
RELEASE_BINARIES = ("apxm", "apxm-server", "apxm-mcp-server")
RELEASE_DOCS = (
    "README.md",
    "CHANGELOG.md",
    "CONTRIBUTING.md",
    "RELEASING.md",
    "LICENSE",
    "SECURITY.md",
    ".dekk.toml",
)
