#!/usr/bin/env python3
"""APXM pack registry client.

Manages packs from the apxm-project/apxm-libs catalog. Packs are the
install unit; each pack contains exactly one skill plus optional
shared resources.

Subcommands:

  list                                 List installed packs and their skills.
  install <pack-id>[==<version>]       Install a pack into ~/.apxm/libs/.
  uninstall <pack-id>                  Remove a pack from ~/.apxm/libs/.
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
    with tarfile.open(out_path, "w:gz") as tar:
        tar.add(pack_dir, arcname=pack_dir.name)
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

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
