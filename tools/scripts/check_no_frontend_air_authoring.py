#!/usr/bin/env python3
"""Guard against a second frontend AIR text emitter.

The canonical AIR contract requires frontend packages to lower
`apxm.frontend-graph.v2` through the Rust-owned native bridge. This script fails
if any Python/TypeScript source under the frontend packages *constructs* AIR
text (an MLIR `module { ... }` / `func.func @...` block, or a bare `ais.<op>`
dialect string) via string literals/formatting instead of using canonical
lowering.

It is intentionally conservative (string/regex-based, not a real parser) and
allow-lists the known-safe patterns already in the tree:

- Prose inside comments/docstrings describing the AIR shape.
- Structural wrapper-stripping equality checks (`== "module {"`), which only
  ever compare against output already produced by the real printer.
- Test assertions that a printer's output *contains* a substring
  (`toContain(...)`, i.e. checking, not authoring).

Exit 0 = clean. Exit 1 = a candidate hand-authored AIR string was found.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# Only the frontend packages that must use canonical native lowering — not the
# Rust AIR builders themselves, which legitimately own AIR text.
SCAN_ROOTS = [
    REPO_ROOT / "crates" / "compiler" / "frontend" / "python" / "apxm_program",
    REPO_ROOT / "crates" / "compiler" / "frontend" / "typescript" / "src",
]

# Lines matching any of these are known-safe uses of AIR-shaped substrings
# (parsing/stripping output already produced by the real printer, or test
# assertions checking printer output) and are never flagged.
SAFE_LINE_PATTERNS = [
    re.compile(r'==\s*["\']module\s*\{["\']'),  # wrapper-stripping checks
    re.compile(r'===\s*["\']module\s*\{["\']'),
    re.compile(r"toContain\("),  # vitest: asserting on real printer output
    re.compile(r"\.contains\("),  # generic "does output contain" checks
    re.compile(r"\.region\(.*[\"'`]ais\."),  # FrontendGraph structural kind
    re.compile(r"\._structured_region\(.*[\"'`]ais\."),  # authored loop helper
]

# A candidate hand-authored AIR string: an opening quote directly followed by
# `module {` / `func.func @` / an `ais.<snake_case_op>` dialect token, i.e.
# code building AIR text as a string literal rather than calling the printer.
AUTHORING_PATTERNS = [
    re.compile(r'''["'`]\s*module\s*\{'''),
    re.compile(r'''["'`]\s*func\.func\s+@'''),
    re.compile(r'''["'`]\s*ais\.[a-z_]+'''),
]


# Generated code (op-catalog-derived attribute/opcode name constants, e.g.
# `AIS_SHARED_PREFIX_GROUP = "ais.shared_prefix_group"`) is not hand-authored
# AIR text — it is regenerated from the Rust catalog and carries `ais.`-
# prefixed *attribute names*, not constructed op/module text. Drift there is
# already guarded by `dekk agents check-frontend-codegen`.
EXCLUDED_DIR_PARTS = {"__pycache__", "_generated", "generated", "dist", "node_modules"}


def _iter_source_files():
    for root in SCAN_ROOTS:
        if not root.exists():
            continue
        for ext in ("*.py", "*.ts"):
            for path in root.rglob(ext):
                if EXCLUDED_DIR_PARTS.intersection(path.parts):
                    continue
                yield path


def _strip_comment_and_docstring_lines(lines: list[str]) -> list[bool]:
    """Return a same-length list of "is prose" flags (comment/docstring)."""
    is_prose = [False] * len(lines)
    in_py_docstring = False
    in_ts_block_comment = False
    for i, line in enumerate(lines):
        stripped = line.strip()
        if in_py_docstring:
            is_prose[i] = True
            if '"""' in stripped or "'''" in stripped:
                in_py_docstring = False
            continue
        if in_ts_block_comment:
            is_prose[i] = True
            if "*/" in stripped:
                in_ts_block_comment = False
            continue
        if stripped.startswith("#") or stripped.startswith("//") or stripped.startswith("*"):
            is_prose[i] = True
            continue
        if stripped.startswith('"""') or stripped.startswith("'''"):
            is_prose[i] = True
            # A single-line docstring (`"""...".""`) opens and closes on the
            # same line; only stay "inside" a docstring if this line has an
            # odd number of triple-quote markers (an unterminated opener).
            triple = stripped.count('"""') + stripped.count("'''")
            in_py_docstring = triple % 2 == 1
            continue
        if stripped.startswith("/**") or stripped.startswith("/*"):
            if "*/" not in stripped:
                in_ts_block_comment = True
            is_prose[i] = True
            continue
    return is_prose


def find_violations() -> list[str]:
    violations: list[str] = []
    for path in _iter_source_files():
        text = path.read_text(encoding="utf-8")
        lines = text.splitlines()
        is_prose = _strip_comment_and_docstring_lines(lines)
        for lineno, line in enumerate(lines, start=1):
            if is_prose[lineno - 1]:
                continue
            if any(safe.search(line) for safe in SAFE_LINE_PATTERNS):
                continue
            for pattern in AUTHORING_PATTERNS:
                if pattern.search(line):
                    rel = path.relative_to(REPO_ROOT)
                    violations.append(f"{rel}:{lineno}: {line.strip()}")
                    break
    return violations


def main() -> int:
    violations = find_violations()
    if violations:
        print(
            "Hand-authored AIR text detected outside the Rust-owned lowering "
            "path. Frontends must lower canonical FrontendGraph, not format "
            "AIR strings locally:\n",
            file=sys.stderr,
        )
        for violation in violations:
            print(f"  {violation}", file=sys.stderr)
        return 1
    print("OK: no hand-authored AIR text found in frontend packages.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
