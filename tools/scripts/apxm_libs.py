#!/usr/bin/env python3
"""APXM pack registry client.

Manages packs from the apxm-project/apxm-libs catalog. Packs are the
install unit; each pack contains exactly one skill plus optional
shared resources.

Subcommands:

  list                                 List installed packs and their skills.
  install <pack-id>[==<version>]       Install a pack into ~/.apxm/libs/.
  uninstall <pack-id>                  Remove a pack from ~/.apxm/libs/.
  build <pack-id>                      Compile every skill in a pack at the
                                       pack's declared opt level, embed each
                                       skill.toml as an
                                       apxm.skill_manifest.v1 section, and
                                       write the canonical artifact_hash and
                                       pack_hash entries in place.
  publish <pack-id>                    Build the pack, then print the
                                       `gh release create` invocation the
                                       maintainer must run to upload it.
                                       Does NOT call the network.
  pack <pack-dir>                      Build a release tarball + compute
                                       pack_hash (maintainer command).
  verify <pack-id>                     Recompute pack_hash and compare
                                       against pack.toml [integrity].
  search <query>                       Substring-match pack id, display
                                       name, description across the
                                       current sibling clone.

Transport for ``install``:

  - sibling clone (default, MVP): copies from a sibling apxm-libs/packs/
    checkout next to the apxm repo, or any directory passed as
    --source <path>.
  - github releases: downloads ``<pack-id>-<version>.tar.gz`` from
    https://github.com/apxm-project/apxm-libs/releases/download/<pack-id>-<version>/
    when --source github is passed.

Install target: ~/.apxm/libs/<pack-id>/ (overridable via APXM_LIBS_ROOT).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

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
DEFAULT_LIBS_ROOT = Path(os.environ.get("APXM_LIBS_ROOT", str(Path.home() / ".apxm" / "libs")))
DEFAULT_RELEASES_BASE = "https://github.com/apxm-project/apxm-libs/releases/download"


@dataclass
class PackManifest:
    pack_id: str
    version: str
    skill_id: str
    description: str
    pack_hash: str | None
    source_kind: str | None
    upstream: str | None
    path: Path


def _tagged_blake3(data: bytes) -> str:
    if not HAVE_BLAKE3:
        raise RuntimeError(
            "blake3 not installed; `pip install blake3` to enable pack_hash "
            "computation"
        )
    return f"{HASH_PREFIX}{_blake3.blake3(data).hexdigest()}"


def _load_manifest(pack_dir: Path) -> PackManifest | None:
    manifest_path = pack_dir / "pack.toml"
    if not manifest_path.is_file():
        return None
    data = tomllib.loads(manifest_path.read_text())
    source = data.get("source") or {}
    integrity = data.get("integrity") or {}
    pack_hash = integrity.get("pack_hash") or None
    if pack_hash == "":
        pack_hash = None
    return PackManifest(
        pack_id=data.get("pack_id", pack_dir.name),
        version=data.get("version", "0.0.0"),
        skill_id=data.get("skill", ""),
        description=data.get("description", ""),
        pack_hash=pack_hash,
        source_kind=source.get("kind"),
        upstream=source.get("upstream"),
        path=pack_dir,
    )


def _iter_installed(libs_root: Path) -> Iterable[PackManifest]:
    if not libs_root.is_dir():
        return
    for entry in sorted(libs_root.iterdir()):
        if not entry.is_dir():
            continue
        manifest = _load_manifest(entry)
        if manifest is not None:
            yield manifest


def _resolve_sibling_source(explicit: Path | None) -> Path:
    if explicit is not None:
        if not explicit.is_dir():
            sys.exit(f"--source path is not a directory: {explicit}")
        return explicit
    here = Path(__file__).resolve()
    apxm_root = here.parents[2]
    candidate = apxm_root.parent / "apxm-libs" / "packs"
    if candidate.is_dir():
        return candidate
    sys.exit(
        "no sibling apxm-libs/packs/ found; pass --source <path> or "
        "clone https://github.com/apxm-project/apxm-libs next to apxm"
    )


def _make_tarball(pack_dir: Path, out_path: Path) -> bytes:
    """Build a reproducible ``.tar.gz`` of ``pack_dir``.

    The pack_hash in ``pack.toml [integrity]`` only carries meaning if
    two consecutive builds of the same source tree produce the same
    bytes. Plain ``tarfile.add(pack_dir, arcname=pack_dir.name)`` walks
    the directory in filesystem order (non-deterministic on most
    Linuxes) and embeds the on-disk mtime/uid/gid of every entry — both
    drift across rebuilds.

    To make pack_hash a real integrity field we (a) walk entries in
    sorted order, (b) zero out mtime/uid/gid/uname/gname, and (c) ask
    gzip to omit its own mtime header (``mtime=0``).
    """

    def _norm(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.mtime = 0
        info.uid = 0
        info.gid = 0
        info.uname = ""
        info.gname = ""
        # Normalize permission bits: directories 0o755, files 0o644.
        if info.isdir():
            info.mode = 0o755
        else:
            info.mode = 0o644
        return info

    entries: list[Path] = []
    for current_root, dirs, files in __import__("os").walk(pack_dir):
        dirs.sort()
        files.sort()
        current = Path(current_root)
        if current != pack_dir:
            entries.append(current)
        for name in files:
            entries.append(current / name)
    entries.sort()

    import gzip

    with open(out_path, "wb") as raw:
        with gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode="w") as tar:
                tar.add(pack_dir, arcname=pack_dir.name, recursive=False, filter=_norm)
                for entry in entries:
                    arcname = f"{pack_dir.name}/{entry.relative_to(pack_dir).as_posix()}"
                    tar.add(entry, arcname=arcname, recursive=False, filter=_norm)
    return out_path.read_bytes()


def cmd_list(args: argparse.Namespace) -> int:
    libs_root = Path(args.libs_root)
    manifests = list(_iter_installed(libs_root))
    if args.json:
        payload = [
            {
                "pack_id": m.pack_id,
                "version": m.version,
                "skill_id": m.skill_id,
                "source_kind": m.source_kind,
                "upstream": m.upstream,
                "path": str(m.path),
            }
            for m in manifests
        ]
        print(json.dumps(payload, indent=2))
        return 0
    if not manifests:
        print(f"no packs installed under {libs_root}")
        return 0
    width = max(len(m.pack_id) for m in manifests)
    for m in manifests:
        src = m.source_kind or "?"
        print(f"  {m.pack_id:<{width}}  {m.version:<8}  source={src:<8}  skill={m.skill_id}")
    return 0


def _split_versioned(pack_ref: str) -> tuple[str, str | None]:
    if "==" in pack_ref:
        pid, ver = pack_ref.split("==", 1)
        return pid, ver
    return pack_ref, None


def cmd_install(args: argparse.Namespace) -> int:
    libs_root = Path(args.libs_root)
    libs_root.mkdir(parents=True, exist_ok=True)
    pack_id, requested_version = _split_versioned(args.pack)
    target = libs_root / pack_id
    if target.exists() and not args.force:
        sys.exit(f"{pack_id} already installed at {target}; pass --force to replace")

    if args.source == "github":
        return _install_from_github(pack_id, requested_version, target)
    src_dir = _resolve_sibling_source(
        Path(args.source) if args.source and args.source != "sibling" else None
    )
    pack_src = src_dir / pack_id
    if not pack_src.is_dir():
        sys.exit(f"pack {pack_id} not found under {src_dir}")
    manifest = _load_manifest(pack_src)
    if manifest is None:
        sys.exit(f"{pack_src}/pack.toml missing or invalid")
    if requested_version and manifest.version != requested_version:
        sys.exit(
            f"requested {pack_id}=={requested_version} but source has {manifest.version}"
        )
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(pack_src, target, symlinks=False)
    print(f"installed {pack_id}=={manifest.version} -> {target}")
    if manifest.pack_hash and HAVE_BLAKE3:
        _verify_install(target, manifest)
    elif manifest.pack_hash and not HAVE_BLAKE3:
        print("  (skipped pack_hash verification; install blake3 to enable)")
    return 0


def _install_from_github(pack_id: str, version: str | None, target: Path) -> int:
    if not version:
        sys.exit("--source github requires <pack-id>==<version>")
    asset_url = (
        f"{DEFAULT_RELEASES_BASE}/{pack_id}-{version}/{pack_id}-{version}.tar.gz"
    )
    with tempfile.TemporaryDirectory() as tmp:
        tarball = Path(tmp) / f"{pack_id}-{version}.tar.gz"
        print(f"downloading {asset_url}")
        try:
            urllib.request.urlretrieve(asset_url, tarball)
        except Exception as exc:
            sys.exit(f"download failed: {exc}")
        tar_bytes = tarball.read_bytes()
        if HAVE_BLAKE3:
            computed = _tagged_blake3(tar_bytes)
            print(f"  computed pack_hash = {computed}")
        else:
            print("  (skipped pack_hash computation; install blake3 to enable)")
        with tarfile.open(tarball, "r:gz") as tar:
            tar.extractall(tmp)
        extracted = Path(tmp) / pack_id
        if not extracted.is_dir():
            sys.exit(
                f"tarball did not contain top-level dir {pack_id}/; "
                "publisher must build it with apxm libs pack"
            )
        manifest = _load_manifest(extracted)
        if manifest is None:
            sys.exit(f"extracted pack has no pack.toml")
        if manifest.pack_hash and HAVE_BLAKE3:
            declared = manifest.pack_hash
            if declared != computed:
                sys.exit(
                    f"pack_hash mismatch: declared {declared} vs computed {computed}"
                )
        if target.exists():
            shutil.rmtree(target)
        shutil.copytree(extracted, target, symlinks=False)
    print(f"installed {pack_id}=={version} -> {target}")
    return 0


def _verify_install(pack_dir: Path, manifest: PackManifest) -> None:
    if not manifest.pack_hash:
        return
    with tempfile.TemporaryDirectory() as tmp:
        tarball = Path(tmp) / f"{manifest.pack_id}.tar.gz"
        data = _make_tarball(pack_dir, tarball)
        computed = _tagged_blake3(data)
    if computed != manifest.pack_hash:
        sys.exit(
            f"pack_hash mismatch after install: declared {manifest.pack_hash} "
            f"vs recomputed {computed}"
        )
    print(f"  pack_hash verified: {computed}")
    _verify_skill_artifact_hashes(pack_dir, manifest.pack_id)


def _verify_skill_artifact_hashes(pack_dir: Path, pack_id: str) -> None:
    """Recompute blake3 of every skill.apxmobj under ``pack_dir`` and
    compare it to the ``artifact_hash`` declared in the sibling
    ``skill.toml``. Fail clean with ``(pack_id, skill_id, expected,
    actual)`` on mismatch. A skill with no declared artifact_hash or no
    compiled artifact is reported and skipped; the caller decides whether
    that is acceptable for the install transport in use.
    """
    if not HAVE_BLAKE3:
        print("  (skipped per-skill artifact_hash verification; install blake3 to enable)")
        return
    skills_root = pack_dir / "skills"
    if not skills_root.is_dir():
        return
    for skill_dir in sorted(p for p in skills_root.iterdir() if p.is_dir()):
        skill_toml = skill_dir / "skill.toml"
        artifact = skill_dir / "skill.apxmobj"
        if not skill_toml.is_file():
            continue
        data = tomllib.loads(skill_toml.read_text())
        skill_id = data.get("skill_id") or data.get("skill", {}).get("skill_id") or skill_dir.name
        declared = data.get("artifact_hash") or data.get("skill", {}).get("artifact_hash")
        if not declared:
            print(f"  {skill_id}: no artifact_hash declared (skipped)")
            continue
        if not artifact.is_file():
            sys.exit(
                f"skill artifact missing: pack={pack_id} skill={skill_id} "
                f"expected at {artifact}"
            )
        computed = _tagged_blake3(artifact.read_bytes())
        if computed != declared:
            sys.exit(
                f"artifact_hash mismatch: pack={pack_id} skill={skill_id} "
                f"expected={declared} actual={computed}"
            )
        print(f"  {skill_id}: artifact_hash verified: {computed}")


def cmd_uninstall(args: argparse.Namespace) -> int:
    libs_root = Path(args.libs_root)
    target = libs_root / args.pack
    if not target.exists():
        sys.exit(f"{args.pack} is not installed")
    shutil.rmtree(target)
    print(f"removed {target}")
    return 0


def cmd_pack(args: argparse.Namespace) -> int:
    pack_dir = Path(args.pack_dir).resolve()
    manifest = _load_manifest(pack_dir)
    if manifest is None:
        sys.exit(f"{pack_dir}/pack.toml not found")
    if not HAVE_BLAKE3:
        sys.exit("blake3 required: pip install blake3")
    out_dir = Path(args.out_dir) if args.out_dir else pack_dir.parent
    out_dir.mkdir(parents=True, exist_ok=True)
    tarball = out_dir / f"{manifest.pack_id}-{manifest.version}.tar.gz"
    data = _make_tarball(pack_dir, tarball)
    pack_hash = _tagged_blake3(data)
    print(f"wrote   {tarball}")
    print(f"sha256  {hashlib.sha256(data).hexdigest()}")
    print(f"pack_hash {pack_hash}")
    print()
    print("Update pack.toml [integrity] before tagging:")
    print(f'  pack_hash = "{pack_hash}"')
    return 0


def cmd_verify(args: argparse.Namespace) -> int:
    libs_root = Path(args.libs_root)
    target = libs_root / args.pack
    manifest = _load_manifest(target)
    if manifest is None:
        sys.exit(f"{args.pack} is not installed at {libs_root}")
    if not manifest.pack_hash:
        print(f"{args.pack}: no pack_hash declared (cannot verify)")
        return 0
    if not HAVE_BLAKE3:
        sys.exit("blake3 required: pip install blake3")
    _verify_install(target, manifest)
    return 0


_OPT_LEVELS = {"O0": 0, "O1": 1, "O2": 2, "O3": 3}


def _resolve_pack_source(pack_id: str, source: str | None) -> Path:
    """Return the source directory for a pack-build invocation. ``source``
    accepts ``sibling`` (default), an absolute path to the packs root, or
    an absolute path to the pack itself.
    """
    if source and source != "sibling":
        candidate = Path(source)
        if not candidate.is_dir():
            sys.exit(f"--source path is not a directory: {candidate}")
        # Accept either packs/ root or the specific pack dir.
        if (candidate / "pack.toml").is_file():
            if candidate.name != pack_id:
                sys.exit(
                    f"--source {candidate} points at pack '{candidate.name}', "
                    f"not '{pack_id}'"
                )
            return candidate
        if (candidate / pack_id).is_dir():
            return candidate / pack_id
        sys.exit(f"pack {pack_id} not found under {candidate}")
    packs_root = _resolve_sibling_source(None)
    pack_dir = packs_root / pack_id
    if not pack_dir.is_dir():
        sys.exit(f"pack {pack_id} not found under {packs_root}")
    return pack_dir


def _parse_opt_level(raw: str | None) -> int:
    if raw is None:
        return 2
    text = str(raw).strip()
    if text in _OPT_LEVELS:
        return _OPT_LEVELS[text]
    if text.isdigit():
        value = int(text)
        if 0 <= value <= 3:
            return value
    sys.exit(f"invalid [compile].opt_level: {raw!r} (expected O0|O1|O2|O3 or 0-3)")


def _strip_pack_integrity(pack_toml_text: str) -> str:
    """Return ``pack_toml_text`` with any ``[integrity]`` block removed.

    Used by ``cmd_build`` to compute pack_hash over the source content
    only, not over the previous run's pack_hash. Otherwise pack_hash
    would self-reference and drift on every rebuild.
    """
    out: list[str] = []
    in_integrity = False
    for line in pack_toml_text.splitlines():
        stripped = line.strip()
        if stripped == "[integrity]":
            in_integrity = True
            continue
        if in_integrity and stripped.startswith("[") and stripped.endswith("]"):
            in_integrity = False
            out.append(line)
            continue
        if in_integrity:
            continue
        out.append(line)
    text = "\n".join(out)
    if pack_toml_text.endswith("\n") and not text.endswith("\n"):
        text += "\n"
    return text


def _write_toml_field(path: Path, table: str | None, key: str, value: str) -> None:
    """Write ``key = "value"`` into the given top-level or single-nested
    table inside ``path``. The writer is purposely small — pack.toml and
    skill.toml are short, hand-maintained, and have stable shapes. It
    preserves comments and whitespace by line-scanning rather than
    re-emitting the TOML.
    """
    text = path.read_text()
    lines = text.splitlines()
    new_line = f'{key} = "{value}"'
    in_target = table is None
    target_header = f"[{table}]" if table else None
    inserted = False
    out: list[str] = []
    seen_target = False
    table_start_idx: int | None = None
    for idx, line in enumerate(lines):
        stripped = line.strip()
        if target_header is not None and stripped == target_header:
            in_target = True
            seen_target = True
            table_start_idx = idx
            out.append(line)
            continue
        if in_target and stripped.startswith("[") and stripped != target_header:
            if not inserted:
                out.append(new_line)
                inserted = True
            in_target = table is None and table is None  # exiting our table
            out.append(line)
            continue
        if in_target and stripped.startswith(f"{key} ") or stripped.startswith(f"{key}="):
            out.append(new_line)
            inserted = True
            continue
        out.append(line)
    if not inserted:
        if target_header is not None and not seen_target:
            if out and out[-1].strip() != "":
                out.append("")
            out.append(target_header)
        out.append(new_line)
    path.write_text("\n".join(out) + "\n")


def _compile_skill_via_dekk(
    skill_dir: Path,
    opt_level: int,
) -> tuple[Path, Path]:
    """Compile one skill via `dekk apxm compile -O <level> --embed-manifest
    skill.toml skill.air -o skill.apxmobj`. Returns the (air, apxmobj)
    paths. Exits on any compile failure.
    """
    air = skill_dir / "skill.air"
    skill_toml = skill_dir / "skill.toml"
    apxmobj = skill_dir / "skill.apxmobj"
    if not air.is_file():
        sys.exit(f"skill.air missing under {skill_dir}")
    if not skill_toml.is_file():
        sys.exit(f"skill.toml missing under {skill_dir}")
    cmd = [
        "dekk",
        "apxm",
        "compile",
        "-O",
        str(opt_level),
        "--embed-manifest",
        str(skill_toml),
        str(air),
        "-o",
        str(apxmobj),
    ]
    try:
        subprocess.run(cmd, check=True)
    except FileNotFoundError:
        sys.exit("`dekk` not on PATH; cannot compile skill")
    except subprocess.CalledProcessError as exc:
        sys.exit(f"compile failed for {skill_dir}: exit {exc.returncode}")
    if not apxmobj.is_file():
        sys.exit(f"compile produced no artifact at {apxmobj}")
    return air, apxmobj


def cmd_build(args: argparse.Namespace) -> int:
    """Compile every skill in the pack at the declared opt_level, write
    canonical artifact_hash / pack_hash entries, and run validate_pack
    --require-artifact (when reachable in the sibling apxm-libs tree).
    """
    if not HAVE_BLAKE3:
        sys.exit("blake3 required: pip install blake3")
    pack_dir = _resolve_pack_source(args.pack, args.source)
    manifest = _load_manifest(pack_dir)
    if manifest is None:
        sys.exit(f"{pack_dir}/pack.toml not found or invalid")

    pack_toml = pack_dir / "pack.toml"
    raw_pack = tomllib.loads(pack_toml.read_text())
    opt_level = _parse_opt_level((raw_pack.get("compile") or {}).get("opt_level"))

    skills_root = pack_dir / "skills"
    if not skills_root.is_dir():
        sys.exit(f"no skills/ directory under {pack_dir}")

    skill_dirs = sorted(p for p in skills_root.iterdir() if p.is_dir())
    if not skill_dirs:
        sys.exit(f"no skills found under {skills_root}")

    for skill_dir in skill_dirs:
        print(f"compiling {skill_dir.name} (O{opt_level})")
        _, apxmobj = _compile_skill_via_dekk(skill_dir, opt_level)
        artifact_hash = _tagged_blake3(apxmobj.read_bytes())
        _write_toml_field(skill_dir / "skill.toml", None, "artifact_hash", artifact_hash)
        print(f"  artifact_hash = {artifact_hash}")

    # Recompute the pack tarball's blake3 and write into pack.toml
    # [integrity]. The hashed tarball must NOT contain the previous
    # run's [integrity] block (chicken-and-egg — pack_hash would
    # otherwise hash the prior pack_hash, drifting on every run). We
    # snapshot pack.toml without [integrity] for the tarball, then
    # restore + write the new pack_hash afterwards.
    pack_toml_backup = pack_toml.read_text()
    stripped_pack = _strip_pack_integrity(pack_toml_backup)
    pack_toml.write_text(stripped_pack)
    try:
        with tempfile.TemporaryDirectory() as tmp:
            tarball = Path(tmp) / f"{manifest.pack_id}.tar.gz"
            data = _make_tarball(pack_dir, tarball)
        pack_hash = _tagged_blake3(data)
    finally:
        pack_toml.write_text(pack_toml_backup)
    _write_toml_field(pack_toml, "integrity", "pack_hash", pack_hash)
    print(f"pack_hash = {pack_hash}")

    # Run validate_pack --require-artifact if the sibling validator is
    # reachable; otherwise warn but do not fail (validator lives in
    # apxm-libs, not in apxm).
    validator = _find_sibling_validator(pack_dir)
    if validator is not None:
        try:
            subprocess.run(
                [sys.executable, str(validator), "--require-artifact", str(pack_dir)],
                check=True,
            )
        except subprocess.CalledProcessError as exc:
            sys.exit(f"validate_pack --require-artifact failed: exit {exc.returncode}")
    else:
        print(
            "  (validate_pack.py not found in sibling apxm-libs; skipping; "
            "publish gate runs it via apxm-libs)"
        )
    return 0


def _find_sibling_validator(pack_dir: Path) -> Path | None:
    cur: Path | None = pack_dir
    for _ in range(6):
        if cur is None:
            break
        candidate = cur / "tools" / "validate_pack.py"
        if candidate.is_file():
            return candidate
        cur = cur.parent
    return None


def cmd_publish(args: argparse.Namespace) -> int:
    """Build the pack, then print the `gh release create` invocation the
    maintainer must run. Does NOT call the network: the CLAUDE.md §11
    boundary requires per-action user approval for any gh mutation.
    """
    pack_dir = _resolve_pack_source(args.pack, args.source)
    manifest = _load_manifest(pack_dir)
    if manifest is None:
        sys.exit(f"{pack_dir}/pack.toml not found or invalid")

    rc = cmd_build(args)
    if rc != 0:
        return rc

    out_dir = Path(args.out_dir) if args.out_dir else pack_dir.parent
    out_dir.mkdir(parents=True, exist_ok=True)
    tarball = out_dir / f"{manifest.pack_id}-{manifest.version}.tar.gz"
    _make_tarball(pack_dir, tarball)
    print()
    print(f"built tarball: {tarball}")
    tag = f"{manifest.pack_id}-v{manifest.version}"
    title = f"{manifest.pack_id} {manifest.version}"
    print()
    print("Next step (run manually after review):")
    print(
        f'  gh release create {tag} --title "{title}" {tarball}'
    )
    print("apxm libs publish does not invoke gh; per CLAUDE.md §11 the")
    print("user authorizes gh release create per action.")
    return 0


def cmd_search(args: argparse.Namespace) -> int:
    src_dir = _resolve_sibling_source(
        Path(args.source) if args.source else None
    )
    needle = args.query.lower()
    hits = []
    for entry in sorted(src_dir.iterdir()):
        if not entry.is_dir():
            continue
        manifest = _load_manifest(entry)
        if manifest is None:
            continue
        haystack = " ".join([manifest.pack_id, manifest.skill_id, manifest.description]).lower()
        if needle in haystack:
            hits.append(manifest)
    if not hits:
        print(f"no packs match {args.query!r}")
        return 0
    for m in hits:
        print(f"  {m.pack_id}  {m.version}  {m.description}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--libs-root",
        default=str(DEFAULT_LIBS_ROOT),
        help=f"install root (default: {DEFAULT_LIBS_ROOT})",
    )
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_list = sub.add_parser("list", help="list installed packs")
    p_list.add_argument("--json", action="store_true")
    p_list.set_defaults(func=cmd_list)

    p_install = sub.add_parser("install", help="install a pack")
    p_install.add_argument("pack", help="pack-id or pack-id==version")
    p_install.add_argument("--source", default="sibling", help="'sibling' | 'github' | absolute path")
    p_install.add_argument("--force", action="store_true")
    p_install.set_defaults(func=cmd_install)

    p_uninstall = sub.add_parser("uninstall", help="remove an installed pack")
    p_uninstall.add_argument("pack")
    p_uninstall.set_defaults(func=cmd_uninstall)

    p_pack = sub.add_parser("pack", help="build a release tarball + pack_hash (maintainer)")
    p_pack.add_argument("pack_dir")
    p_pack.add_argument("--out-dir", default=None)
    p_pack.set_defaults(func=cmd_pack)

    p_verify = sub.add_parser("verify", help="verify an installed pack's hash")
    p_verify.add_argument("pack")
    p_verify.set_defaults(func=cmd_verify)

    p_search = sub.add_parser("search", help="substring match across sibling catalog")
    p_search.add_argument("query")
    p_search.add_argument("--source", default=None)
    p_search.set_defaults(func=cmd_search)

    p_build = sub.add_parser(
        "build",
        help="compile every skill in a pack at the declared opt level + "
        "canonicalize artifact_hash / pack_hash (maintainer)",
    )
    p_build.add_argument("pack", help="pack-id")
    p_build.add_argument(
        "--source",
        default="sibling",
        help="'sibling' | absolute path to a packs root or to the pack directory",
    )
    p_build.set_defaults(func=cmd_build)

    p_publish = sub.add_parser(
        "publish",
        help="build the pack, then print the gh release create invocation "
        "the maintainer must run (no network calls).",
    )
    p_publish.add_argument("pack", help="pack-id")
    p_publish.add_argument("--source", default="sibling")
    p_publish.add_argument("--out-dir", default=None)
    p_publish.set_defaults(func=cmd_publish)

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
