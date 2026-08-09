#!/usr/bin/env python3
"""Fail closed when reference-host publications name a downstream dependency."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
VECTOR_RELATIVE_PATH = "vectors/apxm.reference-host.neutrality.v1.json"
PUBLISHED_VECTOR_RELATIVE_PATH = (
    "reference-host/vectors/apxm.reference-host.neutrality.v1.json"
)
RELEASE_MANIFEST_RELATIVE_PATH = (
    "reference-host/manifests/apxm.reference-host-release-manifest.v1.json"
)
VECTOR_ID = "apxm.reference-host.neutrality.v1"
EXPECTED_SCAN_TARGETS = (
    "contracts/descriptors/apxm.agents-owner-descriptor.v1.json",
    "contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json",
    "contracts/reference-host/manifests/apxm.reference-host-execution-manifest.v1.json",
    "crates/tools/cli/src/bin/reference_host.rs",
    "docs/agents/runtime-profile-parity.md",
)


class NeutralityError(ValueError):
    """Raised when a generic reference-host neutrality check cannot pass."""


def load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise NeutralityError(f"{label} is not valid UTF-8 JSON: {error}") from error
    if not isinstance(value, dict):
        raise NeutralityError(f"{label} must be a JSON object")
    return value


def _vector_paths(root: Path) -> tuple[Path, Path]:
    return (
        root / "contracts" / VECTOR_RELATIVE_PATH,
        root / "contracts" / PUBLISHED_VECTOR_RELATIVE_PATH,
    )


def _forbidden_pattern(vector: dict[str, Any]) -> re.Pattern[str]:
    patterns = vector.get("forbidden_patterns")
    if not isinstance(patterns, list) or not patterns or not all(
        isinstance(pattern, str) and pattern for pattern in patterns
    ):
        raise NeutralityError("neutrality vector forbidden_patterns must be non-empty strings")
    try:
        return re.compile("|".join(f"(?:{pattern})" for pattern in patterns), re.IGNORECASE)
    except re.error as error:
        raise NeutralityError(f"neutrality vector contains an invalid forbidden pattern: {error}") from error


def scan_text(text: str, pattern: re.Pattern[str]) -> list[str]:
    return sorted({match.group(0) for match in pattern.finditer(text)})


def validate_vector(vector: dict[str, Any]) -> re.Pattern[str]:
    if vector.get("schema_version") != VECTOR_ID:
        raise NeutralityError("neutrality vector schema_version drifted")
    if vector.get("semantic_owner") != "agents":
        raise NeutralityError("neutrality vector semantic_owner must be agents")
    if vector.get("profiles") != ["embedded", "reference-host"]:
        raise NeutralityError("neutrality vector profiles drifted")
    targets = vector.get("scan_targets")
    if targets != list(EXPECTED_SCAN_TARGETS):
        raise NeutralityError("neutrality vector scan_targets drifted")
    if not all(
        isinstance(target, str) and target for target in targets
    ):
        raise NeutralityError("neutrality vector scan_targets must be non-empty strings")
    cases = vector.get("negative_cases")
    if not isinstance(cases, list) or not cases:
        raise NeutralityError("neutrality vector negative_cases are missing")
    names: set[str] = set()
    pattern = _forbidden_pattern(vector)
    for case in cases:
        if not isinstance(case, dict):
            raise NeutralityError("neutrality vector case must be an object")
        name = case.get("name")
        if not isinstance(name, str) or not name or name in names:
            raise NeutralityError("neutrality vector case names must be unique strings")
        names.add(name)
        value = case.get("input")
        if not isinstance(value, str) or not value:
            raise NeutralityError(f"neutrality vector case {name!r} input is missing")
        expected = case.get("expected_match")
        if not isinstance(expected, bool) or bool(scan_text(value, pattern)) != expected:
            raise NeutralityError(f"neutrality vector case {name!r} expectation drifted")
    return pattern


def find_violations(root: Path = REPOSITORY_ROOT) -> list[str]:
    canonical_path, published_path = _vector_paths(root)
    canonical = load_json(canonical_path, "canonical neutrality vector")
    published = load_json(published_path, "published neutrality vector")
    if canonical_path.read_bytes() != published_path.read_bytes():
        raise NeutralityError("canonical and published neutrality vector bytes drifted")
    pattern = validate_vector(canonical)
    manifest = load_json(
        root / "contracts" / RELEASE_MANIFEST_RELATIVE_PATH,
        "reference-host release manifest",
    )
    expected = {
        "vector_id": VECTOR_ID,
        "path": PUBLISHED_VECTOR_RELATIVE_PATH,
        "digest": "sha256:" + hashlib.sha256(published_path.read_bytes()).hexdigest(),
        "profiles": ["embedded", "reference-host"],
    }
    if manifest.get("neutrality_vector") != expected:
        raise NeutralityError("reference-host release manifest neutrality vector drifted")

    violations: list[str] = []
    for relative in canonical["scan_targets"]:
        path = (root / relative).resolve(strict=False)
        try:
            path.relative_to(root.resolve(strict=False))
        except ValueError as error:
            raise NeutralityError(f"{relative}: scan target escapes repository root") from error
        if not path.is_file():
            violations.append(f"{relative}: scan target is missing")
            continue
        matches = scan_text(path.read_text(encoding="utf-8"), pattern)
        violations.extend(f"{relative}: forbidden downstream reference `{match}`" for match in matches)
    return violations


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=REPOSITORY_ROOT)
    args = parser.parse_args(argv)
    try:
        violations = find_violations(args.root.resolve(strict=False))
        if violations:
            raise NeutralityError("\n  ".join(violations))
    except (NeutralityError, OSError, UnicodeError, json.JSONDecodeError) as error:
        print(f"FAIL-CLOSED: reference-host neutrality rejected: {error}", file=sys.stderr)
        return 1
    print("PASS: reference-host neutrality")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
