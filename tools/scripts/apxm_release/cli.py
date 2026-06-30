"""Argument parsing for `dekk agents release`."""

from __future__ import annotations

import argparse

from apxm_release.checks import run_checks
from apxm_release.dist import build_dist
from apxm_release.publish import publish_github, publish_pypi


def _add_check_options(subparser: argparse.ArgumentParser) -> None:
    subparser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="Allow a dirty tree for local dry runs",
    )
    subparser.add_argument(
        "--skip-fetch",
        action="store_true",
        help="Do not refresh upstream before sync checks",
    )
    subparser.add_argument("--skip-build", action="store_true", help="Skip release binary build")
    subparser.add_argument(
        "--skip-codegen",
        action="store_true",
        help="Skip frontend codegen freshness check",
    )
    subparser.add_argument(
        "--skip-python-tests",
        action="store_true",
        help="Skip Python frontend tests",
    )
    subparser.add_argument(
        "--skip-secrets",
        action="store_true",
        help="Skip detect-secrets baseline scan",
    )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Prepare and publish APXM releases through Dekk.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    check_parser = subparsers.add_parser("check", help="Run release readiness checks")
    _add_check_options(check_parser)
    check_parser.set_defaults(func=run_checks)

    dist_parser = subparsers.add_parser(
        "dist",
        help="Build release archives, Python package, and checksums",
    )
    _add_check_options(dist_parser)
    dist_parser.add_argument("--target-label", help="Override binary artifact target label")
    dist_parser.add_argument(
        "--output-dir",
        help="Release artifact directory (default: .apxm/releases/vX.Y.Z)",
    )
    dist_parser.add_argument(
        "--skip-python-dist",
        action="store_true",
        help="Skip Python package build",
    )
    dist_parser.set_defaults(func=build_dist)

    publish_parser = subparsers.add_parser("publish", help="Publish GitHub release assets")
    _add_check_options(publish_parser)
    publish_parser.add_argument("--target-label", help="Override binary artifact target label")
    publish_parser.add_argument(
        "--output-dir",
        help="Release artifact directory (default: .apxm/releases/vX.Y.Z)",
    )
    publish_parser.add_argument(
        "--skip-python-dist",
        action="store_true",
        help="Skip Python package build when building dist",
    )
    publish_parser.add_argument(
        "--skip-dist",
        action="store_true",
        help="Publish existing artifacts without rebuilding dist",
    )
    publish_parser.add_argument("--tag", help="Release tag (default: v<version>)")
    publish_parser.add_argument(
        "--update",
        action="store_true",
        help="Replace assets on an existing GitHub release",
    )
    publish_parser.add_argument(
        "--draft",
        action="store_true",
        help="Create the GitHub release as a draft",
    )
    publish_parser.add_argument(
        "--prerelease",
        action="store_true",
        help="Mark the GitHub release as a prerelease",
    )
    publish_parser.add_argument(
        "--yes",
        action="store_true",
        help="Actually publish; otherwise print a dry run",
    )
    publish_parser.set_defaults(func=publish_github)

    pypi_parser = subparsers.add_parser("pypi", help="Upload Python artifacts to PyPI with twine")
    pypi_parser.add_argument(
        "--output-dir",
        help="Release artifact directory (default: .apxm/releases/vX.Y.Z)",
    )
    pypi_parser.add_argument("--repository", help="Twine repository name, for example testpypi")
    pypi_parser.add_argument(
        "--yes",
        action="store_true",
        help="Actually upload; otherwise print a dry run",
    )
    pypi_parser.set_defaults(func=publish_pypi)

    args = parser.parse_args(argv)
    return int(args.func(args))
