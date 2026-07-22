"""Build APXM release artifacts."""

from __future__ import annotations

import argparse
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path

from apxm_release.checks import run_checks
from apxm_release.constants import (
    CHECKSUM_FILE,
    PYTHON_PROJECT,
    RELEASE_BINARIES,
    RELEASE_DOCS,
    REPO_ROOT,
    TARGET_RELEASE_DIR,
)
from apxm_release.util import (
    clean_python_env,
    python_publish_enabled,
    release_dir,
    release_tag,
    release_version,
    run,
    stdout,
    target_label,
    venv_bin,
    write_checksums,
)


def _write_manifest(staging: Path, *, version: str, tag: str, target: str) -> None:
    commit = stdout(["git", "rev-parse", "HEAD"])
    payload = [
        "name: apxm",
        f"version: {version}",
        f"tag: {tag}",
        f"target: {target}",
        f"commit: {commit}",
        "",
        "binaries:",
        *(f"  - bin/{name}" for name in RELEASE_BINARIES),
        "",
    ]
    (staging / "RELEASE-MANIFEST.txt").write_text("\n".join(payload), encoding="utf-8")


def _stage_binary_release(staging: Path, *, version: str, tag: str, target: str) -> None:
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
    _write_manifest(staging, version=version, tag=tag, target=target)


def _make_tarball(staging: Path, output: Path, arc_root: str) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(output, "w:gz") as archive:
        archive.add(staging, arcname=arc_root)


def _make_source_archive(output: Path, arc_root: str) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    archive = run(
        ["git", "archive", "--format=tar.gz", f"--prefix={arc_root}/", "-o", str(output), "HEAD"]
    )
    if archive.returncode != 0:
        raise SystemExit(archive.returncode)


def _build_python_dist(output_dir: Path) -> list[Path]:
    python_dir = output_dir / "python"
    python_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="apxm-python-build-") as temp:
        venv = Path(temp) / "venv"
        created = run([sys.executable, "-m", "venv", str(venv)])
        if created.returncode != 0:
            raise SystemExit(created.returncode)
        pip = venv_bin(venv, "pip")
        python = venv_bin(venv, "python")
        installed = run([str(pip), "install", "build", "twine"], cwd=PYTHON_PROJECT)
        if installed.returncode != 0:
            raise SystemExit(installed.returncode)
        built = run([str(python), "-m", "build", "--outdir", str(python_dir), str(PYTHON_PROJECT)])
        if built.returncode != 0:
            raise SystemExit(built.returncode)
        artifacts = sorted(python_dir.glob("apxm-*"))
        checked = run([str(python), "-m", "twine", "check", *(str(path) for path in artifacts)])
        if checked.returncode != 0:
            raise SystemExit(checked.returncode)
        wheel = next((path for path in artifacts if path.suffix == ".whl"), None)
        if wheel is None:
            raise SystemExit("Python wheel was not produced")
        smoke_venv = Path(temp) / "smoke"
        if run([sys.executable, "-m", "venv", str(smoke_venv)]).returncode != 0:
            raise SystemExit("failed to create Python smoke venv")
        smoke_pip = venv_bin(smoke_venv, "pip")
        smoke_python = venv_bin(smoke_venv, "python")
        smoke_env = clean_python_env()
        installed_wheel = run(
            [str(smoke_pip), "install", "--force-reinstall", str(wheel)],
            env=smoke_env,
        )
        if installed_wheel.returncode != 0:
            raise SystemExit("failed to install built Python wheel")
        smoke = (
            "import apxm_program; "
            "assert apxm_program.FIVE_OPS; "
            "print('OK')"
        )
        if run([str(smoke_python), "-c", smoke], env=smoke_env).returncode != 0:
            raise SystemExit("built Python wheel smoke import failed")
    return sorted(path for path in python_dir.glob("apxm-*") if path.is_file())


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

    version = release_version()
    tag = release_tag(version)
    current_target_label = args.target_label or target_label()
    output_dir = release_dir(version, args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    binary_root = f"apxm-{version}-{current_target_label}"
    binary_archive = output_dir / f"{binary_root}.tar.gz"
    source_archive = output_dir / f"apxm-{version}-source.tar.gz"

    with tempfile.TemporaryDirectory(prefix="apxm-release-") as temp:
        staging = Path(temp) / binary_root
        _stage_binary_release(staging, version=version, tag=tag, target=current_target_label)
        _make_tarball(staging, binary_archive, binary_root)
    _make_source_archive(source_archive, f"apxm-{version}")
    python_artifacts = (
        _build_python_dist(output_dir)
        if python_publish_enabled() and not args.skip_python_dist
        else []
    )
    checksum_path = write_checksums([binary_archive, source_archive, *python_artifacts], output_dir)

    print(f"wrote {binary_archive}")
    print(f"wrote {source_archive}")
    for artifact in python_artifacts:
        print(f"wrote {artifact}")
    print(f"wrote {checksum_path}")
    return 0


def release_artifacts(output_dir: Path) -> list[Path]:
    patterns = ["*.tar.gz", CHECKSUM_FILE]
    if python_publish_enabled():
        patterns.append("python/apxm-*")
    artifacts: list[Path] = []
    for pattern in patterns:
        artifacts.extend(sorted(output_dir.glob(pattern)))
    return [path for path in artifacts if path.is_file()]
