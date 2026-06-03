#!/usr/bin/env python3
"""Validate a compiled-skill bundle directory.

Checks:
  * skill.toml parses and matches the apxm-core SkillManifest contract.
  * Required fields are present (skill_id, version, entry_flow).
  * SKILL.md exists and has YAML frontmatter with name + description.
  * If skill.air is present, it is non-empty.
  * If artifact_hash is set in skill.toml AND skill.apxmobj is present,
    the recomputed blake3 hash matches.

Exit code 0 on success, 1 on any failure. Designed to run locally and
in pre-commit; no network calls.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

REQUIRED_FIELDS = ("skill_id", "version", "entry_flow")
HASH_PREFIX = "blake3:"


def _fail(path: Path, msg: str) -> None:
    print(f"FAIL {path}: {msg}", file=sys.stderr)


def _parse_frontmatter(text: str) -> dict | None:
    if not text.startswith("---\n"):
        return None
    end = text.find("\n---\n", 4)
    if end == -1:
        return None
    block = text[4:end]
    result: dict[str, str] = {}
    for line in block.splitlines():
        if ":" not in line or line.lstrip().startswith("#"):
            continue
        key, _, value = line.partition(":")
        result[key.strip()] = value.strip()
    return result


def _check_artifact_hash(skill_dir: Path, declared: str) -> str | None:
    artifact = skill_dir / "skill.apxmobj"
    if not artifact.exists():
        return None
    if not declared.startswith(HASH_PREFIX):
        return f"artifact_hash must start with {HASH_PREFIX!r}"
    try:
        import blake3
    except ModuleNotFoundError:
        return None
    digest = blake3.blake3(artifact.read_bytes()).hexdigest()
    expected = declared[len(HASH_PREFIX):]
    if digest != expected:
        return f"artifact_hash mismatch: declared {expected}, computed {digest}"
    return None


def validate(skill_dir: Path) -> int:
    failures = 0

    manifest_path = skill_dir / "skill.toml"
    if not manifest_path.exists():
        _fail(skill_dir, "missing skill.toml")
        return 1
    try:
        manifest = tomllib.loads(manifest_path.read_text())
    except tomllib.TOMLDecodeError as exc:
        _fail(manifest_path, f"TOML parse error: {exc}")
        return 1

    if isinstance(manifest.get("skill"), dict):
        manifest = manifest["skill"]

    for field in REQUIRED_FIELDS:
        if not manifest.get(field):
            _fail(manifest_path, f"missing required field: {field}")
            failures += 1

    skill_md = skill_dir / "SKILL.md"
    if not skill_md.exists():
        _fail(skill_dir, "missing SKILL.md")
        failures += 1
    else:
        front = _parse_frontmatter(skill_md.read_text())
        if front is None:
            _fail(skill_md, "missing or malformed YAML frontmatter")
            failures += 1
        else:
            for key in ("name", "description"):
                if not front.get(key):
                    _fail(skill_md, f"frontmatter missing {key}")
                    failures += 1

    air = skill_dir / "skill.air"
    if air.exists() and air.stat().st_size == 0:
        _fail(air, "skill.air is empty")
        failures += 1

    declared_hash = manifest.get("artifact_hash")
    if declared_hash:
        err = _check_artifact_hash(skill_dir, declared_hash)
        if err:
            _fail(skill_dir, err)
            failures += 1

    if failures == 0:
        print(f"OK   {skill_dir}")
    return 0 if failures == 0 else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="+", type=Path,
                        help="Skill bundle directories to validate")
    args = parser.parse_args()

    rc = 0
    for path in args.paths:
        if not path.is_dir():
            print(f"FAIL {path}: not a directory", file=sys.stderr)
            rc = 1
            continue
        rc |= validate(path)
    return rc


if __name__ == "__main__":
    sys.exit(main())
