"""Release readiness checks."""

from __future__ import annotations

import argparse
import shutil
import sys
from dataclasses import dataclass

from apxm_release.constants import RELEASE_BINARIES, REPO_ROOT, TARGET_RELEASE_DIR
from apxm_release.util import (
    internal_crate_name,
    last_line,
    load_toml,
    release_tag,
    run,
    stdout,
    python_version,
    workspace_version,
)


@dataclass(frozen=True)
class CheckResult:
    name: str
    ok: bool
    detail: str


def _print_result(check: CheckResult) -> None:
    print(f"[{'OK' if check.ok else 'FAIL'}] {check.name}: {check.detail}")


def _check_git_clean(allow_dirty: bool) -> CheckResult:
    dirty = stdout(["git", "status", "--porcelain"])
    if dirty and not allow_dirty:
        return CheckResult("git tree", False, "working tree has uncommitted changes")
    detail = "dirty allowed for local check" if dirty else "clean"
    return CheckResult("git tree", True, detail)


def _check_branch_sync(skip_fetch: bool) -> CheckResult:
    branch = stdout(["git", "branch", "--show-current"]) or "<detached>"
    upstream = stdout(["git", "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
    if not upstream:
        return CheckResult("git upstream", False, f"{branch} has no upstream")
    if not skip_fetch:
        fetched = run(["git", "fetch", "--quiet"])
        if fetched.returncode != 0:
            return CheckResult("git upstream", False, "git fetch failed")
    local = stdout(["git", "rev-parse", "HEAD"])
    remote = stdout(["git", "rev-parse", upstream])
    base = stdout(["git", "merge-base", "HEAD", upstream])
    if local == remote:
        return CheckResult("git upstream", True, f"{branch} matches {upstream}")
    if local == base:
        return CheckResult("git upstream", False, f"{branch} is behind {upstream}")
    if remote == base:
        return CheckResult("git upstream", False, f"{branch} has unpushed commits")
    return CheckResult("git upstream", False, f"{branch} diverged from {upstream}")


def _check_version_consistency() -> CheckResult:
    version = workspace_version()
    python = python_version()
    mismatches: list[str] = []
    if python != version:
        mismatches.append(f"python={python}")

    cargo = load_toml(REPO_ROOT / "Cargo.toml")
    for name, spec in cargo["workspace"]["dependencies"].items():
        if (
            internal_crate_name(name)
            and isinstance(spec, dict)
            and str(spec.get("version")) != version
        ):
            mismatches.append(f"workspace.dependencies.{name}={spec.get('version')}")

    lock = load_toml(REPO_ROOT / "Cargo.lock")
    for package in lock.get("package", []):
        name = str(package.get("name", ""))
        if internal_crate_name(name) and str(package.get("version")) != version:
            mismatches.append(f"Cargo.lock:{name}={package.get('version')}")

    if mismatches:
        return CheckResult("version consistency", False, ", ".join(mismatches))
    return CheckResult("version consistency", True, version)


def _check_changelog() -> CheckResult:
    version = workspace_version()
    needle = f"## [{version}] - "
    text = (REPO_ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    detail = f"{version} entry {'present' if needle in text else 'missing'}"
    return CheckResult("changelog", needle in text, detail)


def _check_release_notes() -> CheckResult:
    version = workspace_version()
    path = REPO_ROOT / "release-notes" / f"{release_tag(version)}.md"
    if not path.is_file():
        return CheckResult("release notes", False, f"missing {path.relative_to(REPO_ROOT)}")
    if not path.read_text(encoding="utf-8").strip():
        return CheckResult("release notes", False, f"{path.relative_to(REPO_ROOT)} is empty")
    return CheckResult("release notes", True, str(path.relative_to(REPO_ROOT)))


def _check_tag_available() -> CheckResult:
    tag = release_tag(workspace_version())
    head = stdout(["git", "rev-parse", "HEAD"])
    local = stdout(["git", "rev-parse", "--verify", "--quiet", tag])
    remote_lines = stdout(["git", "ls-remote", "--tags", "apxm-project", tag]).splitlines()
    remote = remote_lines[0].split()[0] if remote_lines else ""
    if local and local != head:
        detail = f"local tag {tag} points at {local[:12]}, not HEAD"
        return CheckResult("release tag", False, detail)
    if remote and remote != head:
        detail = f"remote tag {tag} points at {remote[:12]}, not HEAD"
        return CheckResult("release tag", False, detail)
    if local or remote:
        return CheckResult("release tag", True, f"{tag} points at HEAD")
    return CheckResult("release tag", True, f"{tag} is available")


def _check_rust_publish_guards() -> CheckResult:
    missing: list[str] = []
    for path in sorted((REPO_ROOT / "crates").glob("**/Cargo.toml")):
        text = path.read_text(encoding="utf-8")
        if "publish = false" not in text:
            missing.append(str(path.relative_to(REPO_ROOT)))
    if missing:
        return CheckResult("rust publish guards", False, ", ".join(missing))
    return CheckResult("rust publish guards", True, "all workspace crates are publish=false")


def _check_skillpack() -> CheckResult:
    check = run(
        [sys.executable, "tools/scripts/validate_pack.py", "crates/tools/apxm-server/skills"],
        capture=True,
    )
    detail = "builtin skill pack validates" if check.returncode == 0 else last_line(check)
    return CheckResult("skillpack", check.returncode == 0, detail)


def _check_dekk_doctor() -> CheckResult:
    if shutil.which("dekk") is None:
        return CheckResult("dekk doctor", False, "dekk not found on PATH")
    check = run(["dekk", "apxm", "doctor"], capture=True)
    detail = "environment ready" if check.returncode == 0 else last_line(check)
    return CheckResult("dekk doctor", check.returncode == 0, detail)


def _check_codegen_current(skip_codegen: bool) -> CheckResult:
    if skip_codegen:
        return CheckResult("codegen", True, "skipped")
    check = run(["dekk", "apxm", "codegen", "--check"], capture=True)
    if check.returncode != 0:
        return CheckResult("codegen", False, last_line(check))
    return CheckResult("codegen", True, "generated Python frontend is current")


def _check_python_tests(skip_tests: bool) -> CheckResult:
    if skip_tests:
        return CheckResult("python frontend tests", True, "skipped")
    check = run(["dekk", "apxm", "test-python-frontend"], capture=True)
    detail = "passed" if check.returncode == 0 else last_line(check)
    return CheckResult("python frontend tests", check.returncode == 0, detail)


def _check_secrets(skip_secrets: bool) -> CheckResult:
    if skip_secrets:
        return CheckResult("secrets scan", True, "skipped")
    if not (REPO_ROOT / ".secrets.baseline").is_file():
        return CheckResult("secrets scan", True, "no baseline configured")
    if shutil.which("detect-secrets") is None:
        return CheckResult("secrets scan", True, "detect-secrets not installed; skipped")
    check = run(["detect-secrets", "scan", "--baseline", ".secrets.baseline"], capture=True)
    detail = "baseline scan passed" if check.returncode == 0 else last_line(check)
    return CheckResult("secrets scan", check.returncode == 0, detail)


def _build_release_binaries() -> int:
    commands = (
        [
            sys.executable,
            "tools/scripts/cargo.py",
            "build",
            "-p",
            "apxm-cli",
            "--features",
            "driver,metrics",
            "--release",
        ],
        [sys.executable, "tools/scripts/cargo.py", "build", "-p", "apxm-server", "--release"],
    )
    for cmd in commands:
        check = run(cmd)
        if check.returncode != 0:
            return check.returncode
    return 0


def _check_binaries() -> CheckResult:
    missing = [name for name in RELEASE_BINARIES if not (TARGET_RELEASE_DIR / name).is_file()]
    if missing:
        return CheckResult("release binaries", False, f"missing {', '.join(missing)}")
    return CheckResult("release binaries", True, ", ".join(RELEASE_BINARIES))


def run_checks(args: argparse.Namespace) -> int:
    checks = [
        _check_git_clean(args.allow_dirty),
        _check_branch_sync(args.skip_fetch),
        _check_version_consistency(),
        _check_changelog(),
        _check_release_notes(),
        _check_tag_available(),
        _check_rust_publish_guards(),
        _check_skillpack(),
        _check_dekk_doctor(),
        _check_codegen_current(args.skip_codegen),
        _check_python_tests(args.skip_python_tests),
        _check_secrets(args.skip_secrets),
    ]
    if not args.skip_build:
        build_rc = _build_release_binaries()
        detail = "built" if build_rc == 0 else "cargo build failed"
        checks.append(CheckResult("release binary build", build_rc == 0, detail))
        if build_rc == 0:
            checks.append(_check_binaries())
    for check in checks:
        _print_result(check)
    return 0 if all(check.ok for check in checks) else 1
