#!/usr/bin/env python3
"""Check authoring exports and teaching imports against the surface manifest."""

from __future__ import annotations

import ast
import json
import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
MANIFEST = REPO_ROOT / "contracts" / "vectors" / "apxm.frontend-surface.v1.json"
PYTHON_ROOT = REPO_ROOT / "crates" / "compiler" / "frontend" / "python" / "apxm_program" / "__init__.py"
TYPESCRIPT_ROOT = REPO_ROOT / "crates" / "compiler" / "frontend" / "typescript" / "src" / "index.ts"
TEACHING_DOCUMENTS = (
    *sorted((REPO_ROOT / "docs" / "guides").glob("*.md")),
    REPO_ROOT / "docs" / "agents" / "first-agent.md",
    REPO_ROOT / "crates" / "compiler" / "frontend" / "python" / "README.md",
    REPO_ROOT / "crates" / "compiler" / "frontend" / "typescript" / "README.md",
)


def public_names() -> set[str]:
    """Load the concrete public authoring declarations from the manifest."""
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    return (set(manifest["everyday"]) | set(manifest["advanced"])) - {"agent"}


def python_exports(path: Path) -> tuple[set[str], set[str]]:
    """Return the declared Python surface and every root-bound name."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    exported: set[str] = set()
    root_names: set[str] = set()
    for statement in tree.body:
        if isinstance(statement, ast.ImportFrom):
            root_names.update(alias.asname or alias.name for alias in statement.names)
        elif isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            root_names.add(statement.name)
        elif isinstance(statement, ast.Assign):
            for target in statement.targets:
                if isinstance(target, ast.Name):
                    root_names.add(target.id)
                    if target.id == "__all__" and isinstance(statement.value, (ast.List, ast.Tuple)):
                        exported.update(
                            value.value
                            for value in statement.value.elts
                            if isinstance(value, ast.Constant) and isinstance(value.value, str)
                        )
    return exported, root_names


def typescript_exports(path: Path) -> set[str]:
    """Extract root re-exports without depending on a TypeScript runtime."""
    text = path.read_text(encoding="utf-8")
    names: set[str] = set()
    for group in re.findall(r"export\s*\{(?P<body>.*?)\}\s*from", text, flags=re.DOTALL):
        for entry in group.split(","):
            name = entry.strip()
            if not name:
                continue
            name = re.sub(r"^type\s+", "", name)
            names.add(name.split(" as ")[-1].strip())
    return names


def imported_names(text: str, path: Path) -> list[str]:
    """Return authoring imports in teaching snippets, with their source path."""
    violations: list[str] = []
    patterns = (
        r"from\s+(?:apxm_program|apxm)\s+import\s+([^\n]+)",
        r"import\s*\{([^}]+)\}\s*from\s*[\"']@apxm/frontend[\"']",
    )
    allowed = public_names()
    for pattern in patterns:
        for match in re.finditer(pattern, text):
            names = [name.strip().split(" as ")[0].strip() for name in match.group(1).split(",")]
            for name in names:
                if name and name not in allowed:
                    violations.append(f"{path.relative_to(REPO_ROOT)} imports non-manifest name {name!r}")
    return violations


def check() -> list[str]:
    """Return every manifest-alignment failure."""
    expected = public_names()
    failures: list[str] = []

    python_surface, python_root = python_exports(PYTHON_ROOT)
    if python_surface != expected:
        failures.append(
            f"Python __all__ differs from manifest: expected {sorted(expected)}, got {sorted(python_surface)}"
        )
    leaked_python = python_root - expected - {"__all__", "annotations"}
    if leaked_python:
        failures.append(f"Python root binds non-manifest names: {sorted(leaked_python)}")

    ts_surface = typescript_exports(TYPESCRIPT_ROOT)
    if ts_surface != expected:
        failures.append(
            f"TypeScript root exports differ from manifest: expected {sorted(expected)}, got {sorted(ts_surface)}"
        )

    for document in TEACHING_DOCUMENTS:
        if document.exists():
            failures.extend(imported_names(document.read_text(encoding="utf-8"), document))
    return failures


def main() -> int:
    """Print a compact surface-alignment report."""
    failures = check()
    if failures:
        print("Frontend surface manifest alignment failed:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print("OK: frontend exports and teaching imports match apxm.frontend-surface.v1.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
