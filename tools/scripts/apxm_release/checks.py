"""Release readiness checks."""

from __future__ import annotations

import argparse
import shutil
import sys

from apxm_release.constants import REPO_ROOT, TARGET_RELEASE_DIR, RELEASE_BINARIES, CheckResult
from apxm_release.util import (
    internal_crate_name,
    last_line,
    load_toml,
    print_result,
    release_tag,
    result,
    run,
    stdout,
    workspace_version,
    python_version,
)


def _check_git_clean(allow_dirty: bool) -> CheckResult:
    dirty = stdout(["git", "status", "--porcelain"])
    if dirty and not allow_dirty:
        return result("git tree", False, "working tree has uncommitted changes")
    return result("git tree", True, "dirty allowed for local check" if dirty else "clean")


def _check_branch_sync(skip_fetch: bool) -> CheckResult:
    branch = stdout(["git", "branch", "--show-current"]) or "<detached>"
    upstream = stdout(["git", "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
    if not upstream:
        return result("git upstream", False, f"{branch} has no upstream")
    if not skip_fetch:
        fetched = run(["git", "fetch", "--quiet"])
        if fetched.returncode != 0:
            return result("git upstream", False, "git fetch failed")
    local = stdout(["git", "rev-parse", "HEAD"])
    remote = stdout(["git", "rev-parse", upstream])
    base = stdout(["git", "merge-base", "HEAD", upstream])
    if local == remote:
        return result("git upstream", True, f"{branch} matches {upstream}")
    if local == base:
        return result("git upstream", False, f"{branch} is behind {upstream}")
    if remote == base:
        return result("git upstream", False, f"{branch} has unpushed commits")
    return result("git upstream", False, f"{branch} diverged from {upstream}")


def _check_version_consistency() -> CheckResult:
    version = workspace_version()
    python = python_version()
    mismatches: list[str] = []
    if python != version:
        mismatches.append(f"python={python}")

    cargo = load_toml(REPO_ROOT / "Cargo.toml")
    for name, spec in cargo["workspace"]["dependencies"].items():
        if internal_crate_name(name) and isinstance(spec, dict) and str(spec.get("version")) != version:
            mismatches.append(f"workspace.dependencies.{name}={spec.get('version')}")

    lock = load_toml(REPO_ROOT / "Cargo.lock")
    for package in lock.get("package", []):
        name = str(package.get("name", ""))
        if internal_crate_name(name) and str(package.get("version")) != version:
            mismatches.append(f"Cargo.lock:{name}={package.get('version')}")

    if mismatches:
        return result("version consistency", False, ", ".join(mismatches))
    return result("version consistency", True, version)


def _check_changelog() -> CheckResult:
    version = workspace_version()
    needle = f"## [{version}] - "
    text = (REPO_ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    return result("changelog", needle in text, f"{version} entry {'present' if needle in text else 'missing'}")


def _check_release_notes() -> CheckResult:
    version = workspace_version()
    path = REPO_ROOT / "release-notes" / f"{release_tag(version)}.md"
    if not path.is_file():
        return result("release notes", False, f"missing {path.relative_to(REPO_ROOT)}")
    if not path.read_text(encoding="utf-8").strip():
        return result("release notes", False, f"{path.relative_to(REPO_ROOT)} is empty")
    return result("release notes", True, str(path.relative_to(REPO_ROOT)))


def _check_tag_available() -> CheckResult:
    tag = release_tag(workspace_version())
    head = stdout(["git", "rev-parse", "HEAD"])
    local = stdout(["git", "rev-parse", "--verify", "--quiet", tag])
    remote_lines = stdout(["git", "ls-remote", "--tags", "apxm-project", tag]).splitlines()
    remote = remote_lines[0].split()[0] if remote_lines else ""
    if local and local != head:
        return result("release tag", False, f"local tag {tag} points at {local[:12]}, not HEAD")
    if remote and remote != head:
        return result("release tag", False, f"remote tag {tag} points at {remote[:12]}, not HEAD")
    if local or remote:
        return result("release tag", True, f"{tag} points at HEAD")
    return result("release tag", True, f"{tag} is available")


def _check_rust_publish_guards() -> CheckResult:
    missing: list[str] = []
    for path in sorted((REPO_ROOT / "crates").glob("**/Cargo.toml")):
        text = path.read_text(encoding="utf-8")
        if "publish = false" not in text:
            missing.append(str(path.relative_to(REPO_ROOT)))
    if missing:
        return result("rust publish guards", False, ", ".join(missing))
    return result("rust publish guards", True, "all workspace crates are publish=false")


def _check_no_legacy() -> CheckResult:
    check = run([sys.executable, "tools/scripts/check_no_legacy_vllm.py", "--strict"], capture=True)
    return result("no legacy vLLM", check.returncode == 0, "strict lint passed" if check.returncode == 0 else last_line(check))


def _check_skillpack() -> CheckResult:
    check = run([sys.executable, "tools/scripts/validate_pack.py", "crates/tools/apxm-server/skills"], capture=True)
    return result("skillpack", check.returncode == 0, "builtin skill pack validates" if check.returncode == 0 else last_line(check))


def _check_dekk_doctor() -> CheckResult:
    if shutil.which("dekk") is None:
        return result("dekk doctor", False, "dekk not found on PATH")
    check = run(["dekk", "apxm", "doctor"], capture=True)
    return result("dekk doctor", check.returncode == 0, "environment ready" if check.returncode == 0 else last_line(check))


def _check_codegen_current(skip_codegen: bool) -> CheckResult:
    if skip_codegen:
        return result("codegen", True, "skipped")
    check = run(["dekk", "apxm", "codegen", "--check"], capture=True)
    if check.returncode != 0:
        return result("codegen", False, last_line(check))
    return result("codegen", True, "generated Python frontend is current")


def _check_python_tests(skip_tests: bool) -> CheckResult:
    if skip_tests:
        return result("python frontend tests", True, "skipped")
    check = run(["dekk", "apxm", "test-python-frontend"], capture=True)
    return result("python frontend tests", check.returncode == 0, "passed" if check.returncode == 0 else last_line(check))


def _check_secrets(skip_secrets: bool) -> CheckResult:
    if skip_secrets:
        return result("secrets scan", True, "skipped")
    if not (REPO_ROOT / ".secrets.baseline").is_file():
        return result("secrets scan", True, "no baseline configured")
    if shutil.which("detect-secrets") is None:
        return result("secrets scan", True, "detect-secrets not installed; skipped")
    check = run(["detect-secrets", "scan", "--baseline", ".secrets.baseline"], capture=True)
    return result("secrets scan", check.returncode == 0, "baseline scan passed" if check.returncode == 0 else last_line(check))


def _build_release_binaries() -> int:
    commands = (
        [sys.executable, "tools/scripts/cargo.py", "build", "-p", "apxm-cli", "--features", "driver,metrics", "--release"],
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
        return result("release binaries", False, f"missing {', '.join(missing)}")
    return result("release binaries", True, ", ".join(RELEASE_BINARIES))


def run_checks(args: argparse.Namespace) -> int:
    checks = [
        _check_git_clean(args.allow_dirty),
        _check_branch_sync(args.skip_fetch),
        _check_version_consistency(),
        _check_changelog(),
        _check_release_notes(),
        _check_tag_available(),
        _check_rust_publish_guards(),
        _check_no_legacy(),
        _check_skillpack(),
        _check_dekk_doctor(),
        _check_codegen_current(args.skip_codegen),
        _check_python_tests(args.skip_python_tests),
        _check_secrets(args.skip_secrets),
    ]
    if not args.skip_build:
        build_rc = _build_release_binaries()
        checks.append(result("release binary build", build_rc == 0, "built" if build_rc == 0 else "cargo build failed"))
        if build_rc == 0:
            checks.append(_check_binaries())
    for check in checks:
        print_result(check)
    return 0 if all(check.ok for check in checks) else 1
