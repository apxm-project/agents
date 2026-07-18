"""Small shared helpers for APXM release commands."""

from __future__ import annotations

import hashlib
import os
import platform
import subprocess
import tomllib
from pathlib import Path

from apxm_release.constants import (
    CHECKSUM_FILE,
    INTERNAL_PREFIX,
    PYTHON_PROJECT,
    RELEASE_ROOT,
    REPO_ROOT,
)


def run(
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


def stdout(cmd: list[str], *, cwd: Path | None = None) -> str:
    result = run(cmd, capture=True, cwd=cwd)
    return result.stdout.strip() if result.returncode == 0 else ""


def load_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def workspace_version() -> str:
    cargo = load_toml(REPO_ROOT / "Cargo.toml")
    return str(cargo["workspace"]["package"]["version"])


def python_version() -> str:
    pyproject = load_toml(PYTHON_PROJECT / "pyproject.toml")
    return str(pyproject["project"]["version"])


def python_distribution_name() -> str:
    pyproject = load_toml(PYTHON_PROJECT / "pyproject.toml")
    return str(pyproject["project"]["name"])


def python_publish_enabled() -> bool:
    pyproject = load_toml(PYTHON_PROJECT / "pyproject.toml")
    return pyproject["tool"]["apxm"]["release"]["publish"] is True


def release_version() -> str:
    workspace = workspace_version()
    python = python_version()
    if workspace != python:
        raise SystemExit(
            f"workspace version {workspace} does not match Python package version {python}"
        )
    return workspace


def release_tag(version: str) -> str:
    return f"v{version}"


def release_dir(version: str, output_dir: str | None = None) -> Path:
    if output_dir:
        return Path(output_dir).expanduser().resolve()
    return (RELEASE_ROOT / release_tag(version)).resolve()


def target_label() -> str:
    machine = platform.machine() or "unknown"
    system = platform.system().lower() or "unknown"
    libc = "gnu" if system == "linux" else system
    return f"{machine}-{system}-{libc}"


def last_line(process: subprocess.CompletedProcess[str]) -> str:
    lines = (process.stdout + process.stderr).strip().splitlines()
    return lines[-1] if lines else f"exit {process.returncode}"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_checksums(paths: list[Path], output_dir: Path) -> Path:
    checksum_path = output_dir / CHECKSUM_FILE
    checksum_path.write_text(
        "".join(f"{sha256(path)}  {path.relative_to(output_dir)}\n" for path in sorted(paths)),
        encoding="utf-8",
    )
    return checksum_path


def venv_bin(venv: Path, name: str) -> Path:
    return venv / ("Scripts" if os.name == "nt" else "bin") / name


def clean_python_env() -> dict[str, str]:
    env = dict(os.environ)
    env.pop("PYTHONPATH", None)
    env.pop("PYTHONHOME", None)
    return env


def internal_crate_name(name: str) -> bool:
    return name.startswith(INTERNAL_PREFIX)
