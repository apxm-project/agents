#!/usr/bin/env python3
"""Validate one or more APXM skill-pack directories.

The builtin operating skills live under crates/tools/apxm-server/skills/.
(This validator and the pack format were inherited from the former apxm-libs
repo, which was dissolved 2026-06-03; see docs/skills-migration/.)

The pack format is a single, unified shape. A pack may contain one or many
skills; the optional `agent.toml` / `hierarchy.toml` / `tools.toml` files
at the pack root declare agent identity, position in the hierarchy, and
tool manifests when the pack ships an agent rather than only skills.

Checks:
  * pack.toml parses and declares pack_id, version, license.
  * [source] block is well-formed:
      - kind ∈ {"original", "port"}.
      - When kind = "port": upstream, upstream_path, upstream_commit,
        upstream_license, attribution are required.
  * skills/ contains at least one skill directory, each with a valid
    skill bundle (delegated to validate_skill.py).
  * For port packs: skills/<skill>/upstream/SKILL.md is present.
  * Optional pack-root files validated when present:
      - agent.toml — must declare agent_id, display_name, agent_type, scope.
      - hierarchy.toml — parent_agent (string) and child_agents (list of
        strings) optional.
      - tools.toml — each [[tool]] entry declares name, capability,
        endpoint_pattern.

With --require-artifact (strict publish mode):
  * Every skill's skill.apxmobj must exist.
  * Its BLAKE3 must equal skill.toml::artifact_hash. Drift is rejected
    with the canonical (pack_id, skill_id, expected, actual) tuple so
    install-time tamper detection has a single failure shape across the
    publish/install boundary.

Exit code 0 on success, 1 on any failure.
"""
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

try:
    import blake3 as _blake3
    HAVE_BLAKE3 = True
except ModuleNotFoundError:
    _blake3 = None
    HAVE_BLAKE3 = False

HASH_PREFIX = "blake3:"

REQUIRED_TOP = ("pack_id", "version", "license")
REQUIRED_SOURCE = ("kind", "attribution")
PORT_REQUIRED = ("upstream", "upstream_path", "upstream_commit", "upstream_license")
VALID_KINDS = {"original", "port"}
REQUIRED_AGENT_FIELDS = ("agent_id", "display_name", "agent_type", "scope")
REQUIRED_TOOL_FIELDS = ("name", "capability", "endpoint_pattern")
# Backing kinds a [[tool]] may declare (endpoint_pattern only required for http).
VALID_TOOL_KINDS = ("provider", "http", "static", "mcp")


def _fail(path: Path, msg: str) -> None:
    print(f"FAIL {path}: {msg}", file=sys.stderr)


def _tagged_blake3(data: bytes) -> str:
    if not HAVE_BLAKE3:
        raise RuntimeError("blake3 not installed; `pip install blake3`")
    return f"{HASH_PREFIX}{_blake3.blake3(data).hexdigest()}"


def _check_artifact(pack_id: str, skill_id: str, skill_dir: Path) -> int:
    artifact = skill_dir / "skill.apxmobj"
    if not artifact.is_file():
        _fail(skill_dir, f"--require-artifact: missing skill.apxmobj for {skill_id}")
        return 1
    skill_toml = skill_dir / "skill.toml"
    try:
        manifest = tomllib.loads(skill_toml.read_text())
    except tomllib.TOMLDecodeError as exc:
        _fail(skill_toml, f"TOML parse error: {exc}")
        return 1
    declared = manifest.get("artifact_hash") or ""
    if not declared:
        _fail(skill_toml, f"--require-artifact: skill.toml::artifact_hash empty for {skill_id}")
        return 1
    actual = _tagged_blake3(artifact.read_bytes())
    if declared != actual:
        _fail(
            skill_dir,
            f"artifact_hash mismatch: pack={pack_id} skill={skill_id} "
            f"expected={declared} actual={actual}",
        )
        return 1
    return 0


def _validate_agent_toml(path: Path) -> int:
    if not path.is_file():
        return 0
    try:
        data = tomllib.loads(path.read_text())
    except tomllib.TOMLDecodeError as exc:
        _fail(path, f"TOML parse error: {exc}")
        return 1
    failures = 0
    for field in REQUIRED_AGENT_FIELDS:
        if not data.get(field):
            _fail(path, f"agent.toml missing required field: {field}")
            failures += 1
    return failures


def _validate_hierarchy_toml(path: Path) -> int:
    if not path.is_file():
        return 0
    try:
        data = tomllib.loads(path.read_text())
    except tomllib.TOMLDecodeError as exc:
        _fail(path, f"TOML parse error: {exc}")
        return 1
    failures = 0
    parent = data.get("parent_agent")
    if parent is not None and not isinstance(parent, str):
        _fail(path, "hierarchy.toml::parent_agent must be a string")
        failures += 1
    children = data.get("child_agents")
    if children is not None and (
        not isinstance(children, list) or not all(isinstance(c, str) for c in children)
    ):
        _fail(path, "hierarchy.toml::child_agents must be a list of strings")
        failures += 1
    return failures


def _validate_tools_toml(path: Path) -> int:
    if not path.is_file():
        return 0
    try:
        data = tomllib.loads(path.read_text())
    except tomllib.TOMLDecodeError as exc:
        _fail(path, f"TOML parse error: {exc}")
        return 1
    failures = 0
    tools = data.get("tool")
    if tools is None:
        return 0
    if not isinstance(tools, list):
        _fail(path, "tools.toml::[[tool]] must be an array of tables")
        return 1
    for index, entry in enumerate(tools):
        if not isinstance(entry, dict):
            _fail(path, f"tools.toml::[[tool]] entry #{index} must be a table")
            failures += 1
            continue
        # name + capability are always required.
        for field in ("name", "capability"):
            if not entry.get(field):
                _fail(path, f"tools.toml::[[tool]] entry #{index} missing {field}")
                failures += 1
        # The backing kind selects how the block is registered. endpoint_pattern
        # is required ONLY for kind=http (provider blocks resolve url at dispatch
        # via provider.call; mcp/static carry no endpoint).
        kind = entry.get("kind", "provider")
        if kind not in VALID_TOOL_KINDS:
            _fail(path, f"tools.toml::[[tool]] entry #{index} kind={kind!r} not in {sorted(VALID_TOOL_KINDS)}")
            failures += 1
        if kind == "http" and not entry.get("endpoint_pattern"):
            _fail(path, f"tools.toml::[[tool]] entry #{index} (kind=http) missing endpoint_pattern")
            failures += 1
        if kind == "mcp" and not entry.get("server_url"):
            _fail(path, f"tools.toml::[[tool]] entry #{index} (kind=mcp) missing server_url")
            failures += 1
    return failures


def _validate_skill(skill_dir: Path) -> int:
    validator = Path(__file__).resolve().parent / "validate_skill.py"
    res = subprocess.run(
        [sys.executable, str(validator), str(skill_dir)],
        capture_output=True, text=True,
    )
    if res.returncode != 0:
        sys.stderr.write(res.stderr)
        return 1
    return 0


def validate_pack(pack_dir: Path, require_artifact: bool = False) -> int:
    failures = 0
    manifest_path = pack_dir / "pack.toml"
    if not manifest_path.exists():
        _fail(pack_dir, "missing pack.toml")
        return 1
    try:
        manifest = tomllib.loads(manifest_path.read_text())
    except tomllib.TOMLDecodeError as exc:
        _fail(manifest_path, f"TOML parse error: {exc}")
        return 1

    for field in REQUIRED_TOP:
        if not manifest.get(field):
            _fail(manifest_path, f"missing required field: {field}")
            failures += 1

    source = manifest.get("source")
    if not isinstance(source, dict):
        _fail(manifest_path, "missing [source] block")
        failures += 1
        source = {}
    for field in REQUIRED_SOURCE:
        if not source.get(field):
            _fail(manifest_path, f"[source] missing: {field}")
            failures += 1

    kind = source.get("kind")
    if kind not in VALID_KINDS:
        _fail(manifest_path, f"[source].kind must be one of {sorted(VALID_KINDS)}; got {kind!r}")
        failures += 1
    elif kind == "port":
        for field in PORT_REQUIRED:
            if not source.get(field):
                _fail(manifest_path, f"[source] port pack missing: {field}")
                failures += 1

    pack_id = manifest.get("pack_id") or pack_dir.name

    # The catalog ships ONE content kind: skills (skills/<id>/ with AIR).
    # See docs/vision.md. The provider/connector plane ([app] + tools.toml) was
    # removed to apxm-auth (docs/migration/connectors-removed.md).
    #
    # The validator still RELAXES the skills/ requirement when a pack carries
    # root block metadata ([app] or tools.toml) — the "bundle" shape — because
    # the wire format permits it and the docs/examples/bundle/ fixture uses it.
    # No catalog pack does. --require-artifact is a skill-only check and finds no
    # skill dirs to check on a blocks-only pack.
    is_app_pack = isinstance(manifest.get("app"), dict)
    has_tools = (pack_dir / "tools.toml").is_file()
    blocks_only = is_app_pack or has_tools

    if is_app_pack:
        app = manifest["app"]
        for field in ("id", "category"):
            if not app.get(field):
                _fail(manifest_path, f"[app] missing required field: {field}")
                failures += 1

    skills_root = pack_dir / "skills"
    if not skills_root.is_dir():
        if not blocks_only:
            _fail(pack_dir, "pack must contain a skills/ directory")
            failures += 1
    else:
        skill_dirs = sorted(p for p in skills_root.iterdir() if p.is_dir())
        if not skill_dirs and not blocks_only:
            _fail(skills_root, "pack must contain at least one skill")
            failures += 1
        for skill_dir in skill_dirs:
            skill_id = skill_dir.name
            failures += _validate_skill(skill_dir)
            if kind == "port":
                upstream_md = skill_dir / "upstream" / "SKILL.md"
                if not upstream_md.exists():
                    _fail(skill_dir, "port pack missing upstream/SKILL.md")
                    failures += 1
            if require_artifact:
                failures += _check_artifact(pack_id, skill_id, skill_dir)

    failures += _validate_agent_toml(pack_dir / "agent.toml")
    failures += _validate_hierarchy_toml(pack_dir / "hierarchy.toml")
    failures += _validate_tools_toml(pack_dir / "tools.toml")

    if failures == 0:
        print(f"OK   {pack_dir}")
    return 0 if failures == 0 else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "paths",
        nargs="+",
        type=Path,
        help="Pack directories, or a catalog directory containing packs, to validate",
    )
    parser.add_argument(
        "--require-artifact",
        action="store_true",
        help=(
            "Reject packs whose skills lack a compiled .apxmobj or whose "
            "skill.toml::artifact_hash disagrees with the bytes on disk. "
            "Strict release-gate mode."
        ),
    )
    args = parser.parse_args()

    rc = 0
    for path in args.paths:
        if not path.is_dir():
            print(f"FAIL {path}: not a directory", file=sys.stderr)
            rc = 1
            continue
        if (path / "pack.toml").is_file():
            pack_dirs = [path]
        else:
            # Catalog directory: validate every immediate subdirectory that is a pack.
            pack_dirs = sorted(
                p for p in path.iterdir() if p.is_dir() and (p / "pack.toml").is_file()
            )
            if not pack_dirs:
                print(f"FAIL {path}: no pack.toml and no pack subdirectories", file=sys.stderr)
                rc = 1
                continue
        for pack_dir in pack_dirs:
            rc |= validate_pack(pack_dir, require_artifact=args.require_artifact)
    return rc


if __name__ == "__main__":
    sys.exit(main())
