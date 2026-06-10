#!/usr/bin/env python3
"""Prepare and publish APXM releases through Dekk."""

from __future__ import annotations

import argparse
import hashlib
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
PYTHON_PROJECT = REPO_ROOT / "crates" / "compiler" / "apxm-frontend" / "python"
RELEASE_ROOT = REPO_ROOT / ".apxm" / "releases"
TARGET_RELEASE_DIR = REPO_ROOT / "target" / "release"
CHECKSUM_FILE = "SHA256SUMS"
INTERNAL_PREFIX = "apxm"
RELEASE_BINARIES = ("apxm", "apxm-server", "apxm-mcp-server")
RELEASE_DOCS = (
    "README.md",
    "CHANGELOG.md",
    "CONTRIBUTING.md",
    "RELEASING.md",
    "LICENSE",
    "SECURITY.md",
    ".dekk.toml",
)


@dataclass(frozen=True)
class CheckResult:
    name: str
    ok: bool
    detail: str


def _run(
    cmd: list[str],
    *,
    capture: bool = False,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=cwd or REPO_ROOT,
        check=False,
        text=True,
        env=env,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
    )


def _stdout(cmd: list[str], *, cwd: Path | None = None) -> str:
    result = _run(cmd, capture=True, cwd=cwd)
    return result.stdout.strip() if result.returncode == 0 else ""


def _load_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def _workspace_version() -> str:
    cargo = _load_toml(REPO_ROOT / "Cargo.toml")
    return str(cargo["workspace"]["package"]["version"])


def _python_version() -> str:
    pyproject = _load_toml(PYTHON_PROJECT / "pyproject.toml")
    return str(pyproject["project"]["version"])


def _release_version() -> str:
    workspace = _workspace_version()
    python = _python_version()
    if workspace != python:
        raise SystemExit(f"workspace version {workspace} does not match Python package version {python}")
    return workspace


def _release_tag(version: str) -> str:
    return f"v{version}"


def _release_dir(version: str, output_dir: str | None = None) -> Path:
    if output_dir:
        return Path(output_dir).expanduser().resolve()
    return (RELEASE_ROOT / _release_tag(version)).resolve()


def _target_label() -> str:
    machine = platform.machine() or "unknown"
    system = platform.system().lower() or "unknown"
    libc = "gnu" if system == "linux" else system
    return f"{machine}-{system}-{libc}"


def _result(name: str, ok: bool, detail: str) -> CheckResult:
    return CheckResult(name, ok, detail)


def _print_result(result: CheckResult) -> None:
    print(f"[{'OK' if result.ok else 'FAIL'}] {result.name}: {result.detail}")


def _last_line(result: subprocess.CompletedProcess[str]) -> str:
    lines = (result.stdout + result.stderr).strip().splitlines()
    return lines[-1] if lines else f"exit {result.returncode}"


def _check_git_clean(allow_dirty: bool) -> CheckResult:
    dirty = _stdout(["git", "status", "--porcelain"])
    if dirty and not allow_dirty:
        return _result("git tree", False, "working tree has uncommitted changes")
    return _result("git tree", True, "dirty allowed for local check" if dirty else "clean")


def _check_branch_sync(skip_fetch: bool) -> CheckResult:
    branch = _stdout(["git", "branch", "--show-current"]) or "<detached>"
    upstream = _stdout(["git", "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
    if not upstream:
        return _result("git upstream", False, f"{branch} has no upstream")
    if not skip_fetch:
        fetched = _run(["git", "fetch", "--quiet"])
        if fetched.returncode != 0:
            return _result("git upstream", False, "git fetch failed")
    local = _stdout(["git", "rev-parse", "HEAD"])
    remote = _stdout(["git", "rev-parse", upstream])
    base = _stdout(["git", "merge-base", "HEAD", upstream])
    if local == remote:
        return _result("git upstream", True, f"{branch} matches {upstream}")
    if local == base:
        return _result("git upstream", False, f"{branch} is behind {upstream}")
    if remote == base:
        return _result("git upstream", False, f"{branch} has unpushed commits")
    return _result("git upstream", False, f"{branch} diverged from {upstream}")


def _check_version_consistency() -> CheckResult:
    version = _workspace_version()
    python = _python_version()
    mismatches: list[str] = []
    if python != version:
        mismatches.append(f"python={python}")

    cargo = _load_toml(REPO_ROOT / "Cargo.toml")
    for name, spec in cargo["workspace"]["dependencies"].items():
        if name.startswith(INTERNAL_PREFIX) and isinstance(spec, dict) and str(spec.get("version")) != version:
            mismatches.append(f"workspace.dependencies.{name}={spec.get('version')}")

    lock = _load_toml(REPO_ROOT / "Cargo.lock")
    for package in lock.get("package", []):
        name = str(package.get("name", ""))
        if name.startswith(INTERNAL_PREFIX) and str(package.get("version")) != version:
            mismatches.append(f"Cargo.lock:{name}={package.get('version')}")

    if mismatches:
        return _result("version consistency", False, ", ".join(mismatches))
    return _result("version consistency", True, version)


def _check_changelog() -> CheckResult:
    version = _workspace_version()
    needle = f"## [{version}] - "
    text = (REPO_ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    return _result("changelog", needle in text, f"{version} entry {'present' if needle in text else 'missing'}")


def _check_release_notes() -> CheckResult:
    version = _workspace_version()
    path = REPO_ROOT / "release-notes" / f"{_release_tag(version)}.md"
    if not path.is_file():
        return _result("release notes", False, f"missing {path.relative_to(REPO_ROOT)}")
    if not path.read_text(encoding="utf-8").strip():
        return _result("release notes", False, f"{path.relative_to(REPO_ROOT)} is empty")
    return _result("release notes", True, str(path.relative_to(REPO_ROOT)))


def _check_tag_available() -> CheckResult:
    tag = _release_tag(_workspace_version())
    head = _stdout(["git", "rev-parse", "HEAD"])
    local = _stdout(["git", "rev-parse", "--verify", "--quiet", tag])
    remote_lines = _stdout(["git", "ls-remote", "--tags", "apxm-project", tag]).splitlines()
    remote = remote_lines[0].split()[0] if remote_lines else ""
    if local and local != head:
        return _result("release tag", False, f"local tag {tag} points at {local[:12]}, not HEAD")
    if remote and remote != head:
        return _result("release tag", False, f"remote tag {tag} points at {remote[:12]}, not HEAD")
    if local or remote:
        return _result("release tag", True, f"{tag} points at HEAD")
    return _result("release tag", True, f"{tag} is available")


def _check_rust_publish_guards() -> CheckResult:
    missing: list[str] = []
    for path in sorted((REPO_ROOT / "crates").glob("**/Cargo.toml")):
        text = path.read_text(encoding="utf-8")
        if "publish = false" not in text:
            missing.append(str(path.relative_to(REPO_ROOT)))
    if missing:
        return _result("rust publish guards", False, ", ".join(missing))
    return _result("rust publish guards", True, "all workspace crates are publish=false")


def _check_no_legacy() -> CheckResult:
    result = _run([sys.executable, "tools/scripts/check_no_legacy_vllm.py", "--strict"], capture=True)
    return _result("no legacy vLLM", result.returncode == 0, "strict lint passed" if result.returncode == 0 else _last_line(result))


def _check_skillpack() -> CheckResult:
    result = _run([sys.executable, "tools/scripts/validate_pack.py", "crates/tools/apxm-server/skills"], capture=True)
    return _result("skillpack", result.returncode == 0, "builtin skill pack validates" if result.returncode == 0 else _last_line(result))


def _check_dekk_doctor() -> CheckResult:
    if shutil.which("dekk") is None:
        return _result("dekk doctor", False, "dekk not found on PATH")
    result = _run(["dekk", "apxm", "doctor"], capture=True)
    return _result("dekk doctor", result.returncode == 0, "environment ready" if result.returncode == 0 else _last_line(result))


def _check_codegen_current(skip_codegen: bool) -> CheckResult:
    if skip_codegen:
        return _result("codegen", True, "skipped")
    result = _run(["dekk", "apxm", "codegen", "--check"], capture=True)
    if result.returncode != 0:
        return _result("codegen", False, _last_line(result))
    return _result("codegen", True, "generated Python frontend is current")


def _check_python_tests(skip_tests: bool) -> CheckResult:
    if skip_tests:
        return _result("python frontend tests", True, "skipped")
    result = _run(["dekk", "apxm", "test-python-frontend"], capture=True)
    return _result("python frontend tests", result.returncode == 0, "passed" if result.returncode == 0 else _last_line(result))


def _check_secrets(skip_secrets: bool) -> CheckResult:
    if skip_secrets:
        return _result("secrets scan", True, "skipped")
    if not (REPO_ROOT / ".secrets.baseline").is_file():
        return _result("secrets scan", True, "no baseline configured")
    if shutil.which("detect-secrets") is None:
        return _result("secrets scan", True, "detect-secrets not installed; skipped")
    result = _run(["detect-secrets", "scan", "--baseline", ".secrets.baseline"], capture=True)
    return _result("secrets scan", result.returncode == 0, "baseline scan passed" if result.returncode == 0 else _last_line(result))


def _build_release_binaries() -> int:
    commands = (
        [sys.executable, "tools/scripts/cargo.py", "build", "-p", "apxm-cli", "--features", "driver,metrics", "--release"],
        [sys.executable, "tools/scripts/cargo.py", "build", "-p", "apxm-server", "--release"],
    )
    for cmd in commands:
        result = _run(cmd)
        if result.returncode != 0:
            return result.returncode
    return 0


def _check_binaries() -> CheckResult:
    missing = [name for name in RELEASE_BINARIES if not (TARGET_RELEASE_DIR / name).is_file()]
    if missing:
        return _result("release binaries", False, f"missing {', '.join(missing)}")
    return _result("release binaries", True, ", ".join(RELEASE_BINARIES))


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
        checks.append(_result("release binary build", build_rc == 0, "built" if build_rc == 0 else "cargo build failed"))
        if build_rc == 0:
            checks.append(_check_binaries())
    for check in checks:
        _print_result(check)
    return 0 if all(check.ok for check in checks) else 1


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _write_manifest(staging: Path, *, version: str, tag: str, target_label: str) -> None:
    commit = _stdout(["git", "rev-parse", "HEAD"])
    payload = [
        "name: apxm",
        f"version: {version}",
        f"tag: {tag}",
        f"target: {target_label}",
        f"commit: {commit}",
        "",
        "binaries:",
        *(f"  - bin/{name}" for name in RELEASE_BINARIES),
        "",
    ]
    (staging / "RELEASE-MANIFEST.txt").write_text("\n".join(payload), encoding="utf-8")


def _stage_binary_release(staging: Path, *, version: str, tag: str, target_label: str) -> None:
    bin_dir = staging / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    for name in RELEASE_BINARIES:
        shutil.copy2(TARGET_RELEASE_DIR / name, bin_dir / name)
    lib_dir = TARGET_RELEASE_DIR / "lib"
    if lib_dir.is_dir():
        shutil.copytree(lib_dir, staging / "lib", dirs_exist_ok=True)
    for doc in RELEASE_DOCS:
        source = REPO_ROOT / doc
        if source.is_file():
            destination = staging / doc
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
    _write_manifest(staging, version=version, tag=tag, target_label=target_label)


def _make_tarball(staging: Path, output: Path, arc_root: str) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(output, "w:gz") as archive:
        archive.add(staging, arcname=arc_root)


def _make_source_archive(output: Path, arc_root: str) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    result = _run(["git", "archive", "--format=tar.gz", f"--prefix={arc_root}/", "-o", str(output), "HEAD"])
    if result.returncode != 0:
        raise SystemExit(result.returncode)


def _venv_bin(venv: Path, name: str) -> Path:
    return venv / ("Scripts" if os.name == "nt" else "bin") / name


def _clean_python_env() -> dict[str, str]:
    env = dict(os.environ)
    env.pop("PYTHONPATH", None)
    env.pop("PYTHONHOME", None)
    return env


def _build_python_dist(output_dir: Path) -> list[Path]:
    python_dir = output_dir / "python"
    python_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="apxm-python-build-") as temp:
        venv = Path(temp) / "venv"
        created = _run([sys.executable, "-m", "venv", str(venv)])
        if created.returncode != 0:
            raise SystemExit(created.returncode)
        pip = _venv_bin(venv, "pip")
        python = _venv_bin(venv, "python")
        installed = _run([str(pip), "install", "build", "twine"], cwd=PYTHON_PROJECT)
        if installed.returncode != 0:
            raise SystemExit(installed.returncode)
        built = _run([str(python), "-m", "build", "--outdir", str(python_dir), str(PYTHON_PROJECT)])
        if built.returncode != 0:
            raise SystemExit(built.returncode)
        artifacts = sorted(python_dir.glob("apxm-*"))
        checked = _run([str(python), "-m", "twine", "check", *(str(path) for path in artifacts)])
        if checked.returncode != 0:
            raise SystemExit(checked.returncode)
        wheel = next((path for path in artifacts if path.suffix == ".whl"), None)
        if wheel is None:
            raise SystemExit("Python wheel was not produced")
        smoke_venv = Path(temp) / "smoke"
        if _run([sys.executable, "-m", "venv", str(smoke_venv)]).returncode != 0:
            raise SystemExit("failed to create Python smoke venv")
        smoke_pip = _venv_bin(smoke_venv, "pip")
        smoke_python = _venv_bin(smoke_venv, "python")
        smoke_env = _clean_python_env()
        if _run([str(smoke_pip), "install", "--force-reinstall", str(wheel)], env=smoke_env).returncode != 0:
            raise SystemExit("failed to install built Python wheel")
        smoke = (
            "from apxm.contract import RepoLayout, build_layout; "
            "from apxm import GraphRecorder, compile; "
            "print('OK')"
        )
        if _run([str(smoke_python), "-c", smoke], env=smoke_env).returncode != 0:
            raise SystemExit("built Python wheel smoke import failed")
    return sorted(path for path in python_dir.glob("apxm-*") if path.is_file())


def _write_checksums(paths: list[Path], output_dir: Path) -> Path:
    checksum_path = output_dir / CHECKSUM_FILE
    checksum_path.write_text(
        "".join(f"{_sha256(path)}  {path.relative_to(output_dir)}\n" for path in sorted(paths)),
        encoding="utf-8",
    )
    return checksum_path


def build_dist(args: argparse.Namespace) -> int:
    check_args = argparse.Namespace(
        allow_dirty=args.allow_dirty,
        skip_fetch=args.skip_fetch,
        skip_build=args.skip_build,
        skip_codegen=args.skip_codegen,
        skip_python_tests=args.skip_python_tests,
        skip_secrets=args.skip_secrets,
    )
    check_rc = run_checks(check_args)
    if check_rc != 0:
        return check_rc

    version = _release_version()
    tag = _release_tag(version)
    target_label = args.target_label or _target_label()
    output_dir = _release_dir(version, args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    binary_root = f"apxm-{version}-{target_label}"
    binary_archive = output_dir / f"{binary_root}.tar.gz"
    source_archive = output_dir / f"apxm-{version}-source.tar.gz"

    with tempfile.TemporaryDirectory(prefix="apxm-release-") as temp:
        staging = Path(temp) / binary_root
        _stage_binary_release(staging, version=version, tag=tag, target_label=target_label)
        _make_tarball(staging, binary_archive, binary_root)
    _make_source_archive(source_archive, f"apxm-{version}")
    python_artifacts = _build_python_dist(output_dir) if not args.skip_python_dist else []
    checksum_path = _write_checksums([binary_archive, source_archive, *python_artifacts], output_dir)

    print(f"wrote {binary_archive}")
    print(f"wrote {source_archive}")
    for artifact in python_artifacts:
        print(f"wrote {artifact}")
    print(f"wrote {checksum_path}")
    return 0


def _release_artifacts(output_dir: Path) -> list[Path]:
    patterns = ("*.tar.gz", "python/apxm-*", CHECKSUM_FILE)
    artifacts: list[Path] = []
    for pattern in patterns:
        artifacts.extend(sorted(output_dir.glob(pattern)))
    return [path for path in artifacts if path.is_file()]


def publish_github(args: argparse.Namespace) -> int:
    if shutil.which("gh") is None:
        print("error: gh is required for GitHub release publishing", file=sys.stderr)
        return 2
    version = _release_version()
    tag = args.tag or _release_tag(version)
    output_dir = _release_dir(version, args.output_dir)
    if not args.skip_dist:
        dist_rc = build_dist(args)
        if dist_rc != 0:
            return dist_rc
    artifacts = _release_artifacts(output_dir)
    if not artifacts:
        print(f"error: no release artifacts found under {output_dir}", file=sys.stderr)
        return 1
    notes_file = REPO_ROOT / "release-notes" / f"{tag}.md"
    if not notes_file.is_file():
        print(f"error: release notes missing: {notes_file.relative_to(REPO_ROOT)}", file=sys.stderr)
        return 1

    existing = _run(["gh", "release", "view", tag], capture=True)
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
            _stdout(["git", "rev-parse", "HEAD"]),
            "--title",
            f"apxm {version}",
            "--notes-file",
            str(notes_file),
        ]
        if args.draft:
            cmd.append("--draft")
        if args.prerelease:
            cmd.append("--prerelease")
    return _run(cmd).returncode


def publish_pypi(args: argparse.Namespace) -> int:
    version = _release_version()
    output_dir = _release_dir(version, args.output_dir)
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
        if _run([sys.executable, "-m", "venv", str(venv)]).returncode != 0:
            return 1
        pip = _venv_bin(venv, "pip")
        python = _venv_bin(venv, "python")
        if _run([str(pip), "install", "twine"]).returncode != 0:
            return 1
        cmd = [str(python), "-m", "twine", "upload"]
        if args.repository:
            cmd.extend(["--repository", args.repository])
        cmd.extend(str(path) for path in python_artifacts)
        return _run(cmd).returncode


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    def add_check_options(subparser: argparse.ArgumentParser) -> None:
        subparser.add_argument("--allow-dirty", action="store_true", help="Allow a dirty tree for local dry runs")
        subparser.add_argument("--skip-fetch", action="store_true", help="Do not refresh upstream before sync checks")
        subparser.add_argument("--skip-build", action="store_true", help="Skip release binary build")
        subparser.add_argument("--skip-codegen", action="store_true", help="Skip frontend codegen freshness check")
        subparser.add_argument("--skip-python-tests", action="store_true", help="Skip Python frontend tests")
        subparser.add_argument("--skip-secrets", action="store_true", help="Skip detect-secrets baseline scan")

    check_parser = subparsers.add_parser("check", help="Run release readiness checks")
    add_check_options(check_parser)
    check_parser.set_defaults(func=run_checks)

    dist_parser = subparsers.add_parser("dist", help="Build release archives, Python package, and checksums")
    add_check_options(dist_parser)
    dist_parser.add_argument("--target-label", help="Override binary artifact target label")
    dist_parser.add_argument("--output-dir", help="Release artifact directory (default: .apxm/releases/vX.Y.Z)")
    dist_parser.add_argument("--skip-python-dist", action="store_true", help="Skip Python package build")
    dist_parser.set_defaults(func=build_dist)

    publish_parser = subparsers.add_parser("publish", help="Publish GitHub release assets")
    add_check_options(publish_parser)
    publish_parser.add_argument("--target-label", help="Override binary artifact target label")
    publish_parser.add_argument("--output-dir", help="Release artifact directory (default: .apxm/releases/vX.Y.Z)")
    publish_parser.add_argument("--skip-python-dist", action="store_true", help="Skip Python package build when building dist")
    publish_parser.add_argument("--skip-dist", action="store_true", help="Publish existing artifacts without rebuilding dist")
    publish_parser.add_argument("--tag", help="Release tag (default: v<version>)")
    publish_parser.add_argument("--update", action="store_true", help="Replace assets on an existing GitHub release")
    publish_parser.add_argument("--draft", action="store_true", help="Create the GitHub release as a draft")
    publish_parser.add_argument("--prerelease", action="store_true", help="Mark the GitHub release as a prerelease")
    publish_parser.add_argument("--yes", action="store_true", help="Actually publish; otherwise print a dry run")
    publish_parser.set_defaults(func=publish_github)

    pypi_parser = subparsers.add_parser("pypi", help="Upload Python artifacts to PyPI with twine")
    pypi_parser.add_argument("--output-dir", help="Release artifact directory (default: .apxm/releases/vX.Y.Z)")
    pypi_parser.add_argument("--repository", help="Twine repository name, for example testpypi")
    pypi_parser.add_argument("--yes", action="store_true", help="Actually upload; otherwise print a dry run")
    pypi_parser.set_defaults(func=publish_pypi)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
