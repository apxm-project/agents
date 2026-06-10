"""Publish APXM release artifacts."""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
from pathlib import Path

from apxm_release.constants import REPO_ROOT
from apxm_release.dist import build_dist, release_artifacts
from apxm_release.util import release_dir, release_tag, release_version, run, stdout, venv_bin


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


def publish_pypi(args: argparse.Namespace) -> int:
    version = release_version()
    output_dir = release_dir(version, args.output_dir)
    python_artifacts = sorted((output_dir / "python").glob("apxm-*"))
    if not python_artifacts:
        print(f"error: no Python artifacts found under {output_dir / 'python'}", file=sys.stderr)
        return 1
    if not args.yes:
        print(f"PyPI upload dry run for apxm {version}:")
        for artifact in python_artifacts:
            print(f"  {artifact}")
        print("pass --yes to upload with twine")
        return 0
    with tempfile.TemporaryDirectory(prefix="apxm-pypi-") as temp:
        venv = Path(temp) / "venv"
        if run([sys.executable, "-m", "venv", str(venv)]).returncode != 0:
            return 1
        pip = venv_bin(venv, "pip")
        python = venv_bin(venv, "python")
        if run([str(pip), "install", "twine"]).returncode != 0:
            return 1
        cmd = [str(python), "-m", "twine", "upload"]
        if args.repository:
            cmd.extend(["--repository", args.repository])
        cmd.extend(str(path) for path in python_artifacts)
        return run(cmd).returncode
