"""Hold every checked-in agent package against its own generated integrity chain.

`agent lint` reads a package as authored and `agent build` writes its chain;
neither notices a package edited after its last build. Nothing in the standard
battery did either, which is how `examples/agents/conversational/integrity.toml`
once went stale while `test-cli`, `check`, and `test-frontend-examples` all
passed. This gate closes that: a package edit without a rebuild fails here.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]

# Every checked-in `apxm.agent` package that carries a generated integrity.toml.
AGENT_PACKAGES = (
    "examples/agents/conversational",
    "examples/agents/coder",
    "examples/agents/skilled",
    "crates/compiler/frontend/python/tests_program/fixtures/canonical_session_agent",
    "tools/tests/fixtures/python-handler-package",
)


def cargo_target_dir() -> Path:
    """Resolve the target dir the project's cargo wrapper is using."""

    completed = subprocess.run(
        [sys.executable, "tools/scripts/cargo.py", "target-dir"],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return Path(completed.stdout.strip())


def build_cli() -> Path:
    """Build the CLI this gate drives and return its path."""

    subprocess.run(
        [sys.executable, "tools/scripts/cargo.py", "build", "-p", "apxm-cli", "--bin", "apxm"],
        cwd=REPOSITORY_ROOT,
        check=True,
    )
    return cargo_target_dir() / "debug" / "apxm"


def run_action(apxm: Path, action: str, package: str) -> bool:
    """Run one `apxm agent <action>` over one package, reporting its verdict."""

    completed = subprocess.run(
        [str(apxm), "agent", action, package],
        cwd=REPOSITORY_ROOT,
        capture_output=True,
        text=True,
    )
    if completed.stdout.strip():
        print(completed.stdout.rstrip(), flush=True)
    if completed.stderr.strip():
        print(completed.stderr.rstrip(), file=sys.stderr, flush=True)
    return completed.returncode == 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--build",
        action="store_true",
        help="Rebuild each package's integrity chain instead of verifying it.",
    )
    arguments = parser.parse_args()

    apxm = build_cli()
    actions = ("build",) if arguments.build else ("lint", "verify")

    failures: list[str] = []
    for package in AGENT_PACKAGES:
        for action in actions:
            if not run_action(apxm, action, package):
                failures.append(f"{package}: agent {action}")
                break

    if failures:
        print(
            "agent package gate failed:\n  " + "\n  ".join(failures),
            file=sys.stderr,
        )
        return 1
    print(f"{len(AGENT_PACKAGES)} agent packages {'rebuilt' if arguments.build else 'verified'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
