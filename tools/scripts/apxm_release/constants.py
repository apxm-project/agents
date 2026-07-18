"""Shared release paths and constants."""

from __future__ import annotations

from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
PYTHON_PROJECT = REPO_ROOT / "crates" / "compiler" / "frontend" / "python"
RELEASE_ROOT = REPO_ROOT / ".apxm" / "releases"
TARGET_RELEASE_DIR = REPO_ROOT / "target" / "release"
CHECKSUM_FILE = "SHA256SUMS"
INTERNAL_PREFIX = "apxm"
PRIVATE_PYTHON_CLASSIFIER = "Private :: Do Not Upload"
CANONICAL_PYTHON_DISTRIBUTIONS = frozenset(("apxm-frontend", "apxm-compiler"))
REGISTRY_MANIFEST_SCHEMA = "apxm.private-package-registry.v1"
REGISTRY_SIGNATURE_NAMESPACE = "apxm.private-package-registry.v1"
RELEASE_BINARIES = ("apxm",)
RELEASE_DOCS = (
    "README.md",
    "CHANGELOG.md",
    "CONTRIBUTING.md",
    "RELEASING.md",
    "LICENSE",
    "SECURITY.md",
    ".dekk.toml",
)
