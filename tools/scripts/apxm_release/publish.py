"""Publish APXM release artifacts."""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path

from apxm_release.constants import REPO_ROOT
from apxm_release.dist import build_dist, release_artifacts
from apxm_release.privacy import (
    load_private_python_registry,
    require_publishable_python_distribution,
)
from apxm_release.util import release_dir, release_tag, release_version, run, stdout


def _require_private_github_repository() -> bool:
    visibility = stdout(["gh", "repo", "view", "--json", "visibility", "--jq", ".visibility"])
    if visibility != "PRIVATE":
        detail = visibility or "unavailable"
        print(
            f"error: GitHub repository visibility must be PRIVATE, got {detail}",
            file=sys.stderr,
        )
        return False
    return True


def publish_github(args: argparse.Namespace) -> int:
    if shutil.which("gh") is None:
        print("error: gh is required for GitHub release publishing", file=sys.stderr)
        return 2
    version = release_version()
    tag = args.tag or release_tag(version)
    output_dir = release_dir(version, args.output_dir)
    if not args.skip_dist:
        dist_rc = build_dist(args)
        if dist_rc != 0:
            return dist_rc
    artifacts = release_artifacts(output_dir)
    if not artifacts:
        print(f"error: no release artifacts found under {output_dir}", file=sys.stderr)
        return 1
    notes_file = REPO_ROOT / "release-notes" / f"{tag}.md"
    if not notes_file.is_file():
        print(f"error: release notes missing: {notes_file.relative_to(REPO_ROOT)}", file=sys.stderr)
        return 1

    existing = run(["gh", "release", "view", tag], capture=True)
    if existing.returncode == 0 and not args.update:
        print(f"error: GitHub release {tag} already exists; pass --update", file=sys.stderr)
        return 1
    if not args.yes:
        print(f"GitHub release dry run for {tag}:")
        for artifact in artifacts:
            print(f"  {artifact}")
        print("pass --yes to publish")
        return 0

    if not _require_private_github_repository():
        return 2

    if existing.returncode == 0:
        cmd = ["gh", "release", "upload", tag, "--clobber", *(str(path) for path in artifacts)]
    else:
        cmd = [
            "gh",
            "release",
            "create",
            tag,
            *(str(path) for path in artifacts),
            "--target",
            stdout(["git", "rev-parse", "HEAD"]),
            "--title",
            f"apxm {version}",
            "--notes-file",
            str(notes_file),
        ]
        if args.draft:
            cmd.append("--draft")
        if args.prerelease:
            cmd.append("--prerelease")
    return run(cmd).returncode


def publish_python(args: argparse.Namespace) -> int:
    version = release_version()
    output_dir = release_dir(version, args.output_dir)
    try:
        registry = load_private_python_registry(
            Path(args.registry_manifest).expanduser().resolve(),
            Path(args.registry_signature).expanduser().resolve(),
            Path(args.allowed_signers).expanduser().resolve(),
            args.signer,
        )
        distribution = require_publishable_python_distribution(registry)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    normalized = distribution.replace("-", "_")
    python_artifacts = sorted(
        path
        for path in (output_dir / "python").iterdir()
        if path.is_file()
        and (path.name.startswith(f"{distribution}-") or path.name.startswith(f"{normalized}-"))
    ) if (output_dir / "python").is_dir() else []
    if not python_artifacts:
        print(f"error: no Python artifacts found under {output_dir / 'python'}", file=sys.stderr)
        return 1
    if not args.yes:
        print(
            f"private Python registry dry run for {distribution} {version} "
            f"(manifest sha256:{registry.manifest_sha256}):"
        )
        for artifact in python_artifacts:
            print(f"  {artifact}")
        print("pass --yes to upload to the signed exact endpoint")
        return 0
    twine = shutil.which("twine")
    if twine is None:
        print("error: twine must be installed before private Python publishing", file=sys.stderr)
        return 2
    cmd = [twine, "upload", "--repository-url", registry.repository_url]
    cmd.extend(str(path) for path in python_artifacts)
    return run(cmd).returncode
