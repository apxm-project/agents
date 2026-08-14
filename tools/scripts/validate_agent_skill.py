#!/usr/bin/env python3
"""Validate one or more instruction-only Agent Skill directories.

An Agent Skill is trusted context, not executable deployment. This validator
therefore accepts a ``SKILL.md`` instruction document plus ordinary resources
and rejects every executable-package artifact or manifest.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


INSTRUCTIONS = "SKILL.md"
MAX_INSTRUCTION_BYTES = 128 * 1024
FORBIDDEN_ENTRIES = {"skill" + suffix for suffix in (".toml", ".apxmobj", ".air")}
# Any compiled object or serialized AIR module is a deployment unit, wherever it
# sits and whatever it is called. `apxm_program::is_executable_skill_resource`
# states the same rule for the contract reader; the two must agree, because a
# directory this gate accepts is a directory the runtime index will publish.
FORBIDDEN_SUFFIXES = (".apxmobj", ".air")
SKILL_ID = re.compile(r"^[a-z0-9][a-z0-9-]*$")


def fail(path: Path, message: str) -> None:
    print(f"FAIL {path}: {message}", file=sys.stderr)


def frontmatter(text: str) -> dict[str, str] | None:
    if not text.startswith("---\n"):
        return None
    end = text.find("\n---\n", 4)
    if end < 0:
        return None
    values: dict[str, str] = {}
    for line in text[4:end].splitlines():
        if not line or line.lstrip().startswith("#"):
            continue
        key, separator, value = line.partition(":")
        if not separator:
            return None
        values[key.strip()] = value.strip()
    return values


def validate(path: Path) -> int:
    failures = 0
    instructions = path / INSTRUCTIONS
    if not instructions.is_file() or instructions.is_symlink():
        fail(path, f"missing regular {INSTRUCTIONS}")
        return 1
    if instructions.stat().st_size > MAX_INSTRUCTION_BYTES:
        fail(instructions, f"exceeds {MAX_INSTRUCTION_BYTES} byte context limit")
        failures += 1
    text = instructions.read_text(encoding="utf-8")
    metadata = frontmatter(text)
    if metadata is None:
        fail(instructions, "missing or malformed YAML frontmatter")
        failures += 1
    else:
        name = metadata.get("name", "")
        if not SKILL_ID.fullmatch(name):
            fail(instructions, "frontmatter name must be a lowercase Agent Skill id")
            failures += 1
        if not metadata.get("description"):
            fail(instructions, "frontmatter description is required")
            failures += 1

    for entry in path.rglob("*"):
        if entry.is_symlink():
            fail(entry, "symlinked Agent Skill resources are not permitted")
            failures += 1
        elif entry.is_file() and (
            entry.name.lower() in FORBIDDEN_ENTRIES
            or entry.name.lower().endswith(FORBIDDEN_SUFFIXES)
        ):
            fail(entry, "executable skill package files are forbidden; use a separate ProgramPackage")
            failures += 1

    if failures == 0:
        print(f"OK   {path}")
    return int(failures > 0)


def discover(root: Path) -> list[Path]:
    """Every skill directory in a discovery root.

    A root holds one ``<skill_id>/SKILL.md`` per skill. Directories without one
    are not skills and are not this gate's business — that is the same rule the
    runtime index applies, so the gate and the index see the same set.
    """

    if not root.is_dir():
        fail(root, "not a directory")
        return []
    return sorted(child for child in root.iterdir() if (child / INSTRUCTIONS).is_file())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", type=Path, help="Agent Skill directories to validate")
    parser.add_argument(
        "--root",
        action="append",
        type=Path,
        default=[],
        help="Discovery root to validate every skill directory in",
    )
    args = parser.parse_args()

    targets = list(args.paths)
    for root in args.root:
        targets.extend(discover(root))
    if not targets:
        parser.error("no Agent Skill directories to validate")
    return max(
        (validate(path) if path.is_dir() else (fail(path, "not a directory") or 1) for path in targets),
        default=0,
    )


if __name__ == "__main__":
    raise SystemExit(main())
