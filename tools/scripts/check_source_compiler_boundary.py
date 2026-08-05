#!/usr/bin/env python3
"""Reject downstream dependencies and retired compiler paths in the source lane."""

from __future__ import annotations

import importlib.util
import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_ROOTS = (
    REPO_ROOT / "crates/compiler/frontend/python/apxm_program",
    REPO_ROOT / "crates/compiler/frontend/typescript/src",
    REPO_ROOT / "crates/compiler/source-port/src",
    REPO_ROOT / "crates/machine/program/src/frontend_graph.rs",
    REPO_ROOT / "crates/machine/program/src/lower.rs",
    REPO_ROOT / "crates/machine/program/src/source_map.rs",
)
EXCLUDED_PARTS = {"__pycache__", "_generated", "generated", "dist", "node_modules"}
FORBIDDEN_PRODUCT_REFERENCES = re.compile(
    r"\b(?:clic|studio|gao|host-sdk|server-ms|auth-ms|widget)\b",
    re.IGNORECASE,
)
FORBIDDEN_IMPORTS = re.compile(
    r"(?:apxm_vllm|@apxm/(?:host-sdk|server|studio)|(?:from|import)\s+\b(?:server|studio|auth)\b)",
    re.IGNORECASE,
)
RETIRED_PATHS = (
    Path("crates/compiler/pipeline/src/air_builder"),
    Path("crates/compiler/frontend/python/apxm"),
    Path("crates/compiler/frontend/python/apxm_program/conversational.py"),
    Path("crates/compiler/frontend/python/apxm_program/gao.py"),
    Path("crates/compiler/frontend/native/typescript/js/conversational.ts"),
    Path("crates/compiler/frontend/native/typescript/js/gao.ts"),
)


def _iter_source_files():
    for root in SOURCE_ROOTS:
        if root.is_file():
            yield root
            continue
        for path in root.rglob("*"):
            if path.is_file() and not EXCLUDED_PARTS.intersection(path.parts):
                if path.suffix in {".py", ".rs", ".ts", ".mjs"}:
                    yield path


def _air_guard_violations() -> list[str]:
    module_path = REPO_ROOT / "tools/scripts/check_no_frontend_air_authoring.py"
    spec = importlib.util.spec_from_file_location("check_no_frontend_air_authoring", module_path)
    if spec is None or spec.loader is None:
        return [f"unable to load {module_path.relative_to(REPO_ROOT)}"]
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return list(module.find_violations())


def find_violations() -> list[str]:
    violations: list[str] = []
    for path in _iter_source_files():
        text = path.read_text(encoding="utf-8", errors="replace")
        relative = path.relative_to(REPO_ROOT)
        if FORBIDDEN_PRODUCT_REFERENCES.search(text):
            violations.append(f"{relative}: downstream product reference")
        if FORBIDDEN_IMPORTS.search(text):
            violations.append(f"{relative}: downstream dependency import")

    for relative in RETIRED_PATHS:
        if (REPO_ROOT / relative).exists():
            violations.append(f"{relative}: retired source/compiler path remains")

    violations.extend(
        f"{relative}: hand-authored AIR path" for relative in _air_guard_violations()
    )
    return violations


def main() -> int:
    violations = find_violations()
    if violations:
        print("Source/compiler boundary check failed:", file=sys.stderr)
        for violation in violations:
            print(f"  {violation}", file=sys.stderr)
        return 1
    print("OK: source/compiler boundary and rejected-path checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
