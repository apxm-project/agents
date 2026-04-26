#!/usr/bin/env python3
"""Run Cargo for APXM with a machine-local target directory."""

from __future__ import annotations

import hashlib
import os
import shutil
import stat
import subprocess
import sys
import tempfile
from enum import StrEnum
from pathlib import Path


class CargoCommand(StrEnum):
    BUILD = "build"
    BUILD_DIALECT = "build-dialect"
    CLEAN = "clean"
    TARGET_DIR = "target-dir"
    TEST = "test"


class EnvKey(StrEnum):
    APXM_CARGO_TARGET_DIR = "APXM_CARGO_TARGET_DIR"
    CARGO_TARGET_DIR = "CARGO_TARGET_DIR"


REPO_MARKER = "Cargo.toml"
DEKK_MARKER = ".dekk.toml"
CARGO = "cargo"
CMAKE = "cmake"
BUILD_FLAG = "--build"
TARGET_FLAG = "--target"
RELEASE_FLAG = "--release"
PACKAGE_FLAG = "-p"
FEATURES_FLAG = "--features"
DRIVER_METRICS_FEATURES = "driver,metrics"
APXM_CLI_PACKAGE = "apxm-cli"
TARGET_ROOT_NAME = "apxm-cargo-targets"
PROJECT_TARGET_DIR_NAME = "target"
RELEASE_PROFILE_DIR_NAME = "release"
BUILD_DIR_NAME = "build"
LIB_DIR_NAME = "lib"
MAKEFILE_NAME = "Makefile"
DEPS_DIR_NAME = "deps"
EXAMPLES_DIR_NAME = "examples"
INCREMENTAL_DIR_NAME = "incremental"
APXM_COMPILER_BUILD_GLOB = f"*/apxm-compiler*/out/{BUILD_DIR_NAME}/{MAKEFILE_NAME}"
TABLEGEN_TARGET = "AISIRIncGen"
PROJECT_DIGEST_SIZE = 16
EXECUTABLE_SUFFIX = ".exe"
LIBRARY_SUFFIXES = frozenset({".a", ".dylib", ".dll", ".so"})
SKIP_RELEASE_ENTRIES = frozenset({
    BUILD_DIR_NAME,
    DEPS_DIR_NAME,
    EXAMPLES_DIR_NAME,
    INCREMENTAL_DIR_NAME,
})


def _repo_root(start: Path) -> Path:
    for candidate in (start.resolve(), *start.resolve().parents):
        if (candidate / REPO_MARKER).is_file() and (candidate / DEKK_MARKER).is_file():
            return candidate
    raise SystemExit("error: unable to locate APXM repository root")


def _project_digest(project_root: Path) -> str:
    payload = str(project_root.resolve()).encode()
    return hashlib.sha256(payload).hexdigest()[:PROJECT_DIGEST_SIZE]


def _target_dir(project_root: Path) -> Path:
    explicit = os.environ.get(EnvKey.APXM_CARGO_TARGET_DIR.value)
    if explicit:
        return Path(explicit).expanduser().resolve()
    return (
        Path(tempfile.gettempdir())
        / TARGET_ROOT_NAME
        / f"{project_root.name}-{_project_digest(project_root)}"
    )


def _cargo_env(target_dir: Path) -> dict[str, str]:
    env = dict(os.environ)
    env[EnvKey.CARGO_TARGET_DIR.value] = str(target_dir)
    return env


def _run(command: list[str], *, project_root: Path, target_dir: Path) -> int:
    result = subprocess.run(
        command,
        cwd=project_root,
        env=_cargo_env(target_dir),
        check=False,
    )
    return int(result.returncode)


def _is_executable(path: Path) -> bool:
    if path.suffix == EXECUTABLE_SUFFIX:
        return True
    try:
        mode = path.stat().st_mode
    except OSError:
        return False
    return bool(mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH))


def _is_release_artifact(path: Path) -> bool:
    if not path.is_file():
        return False
    return _is_executable(path) or path.suffix in LIBRARY_SUFFIXES


def _copy_file(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def _mirror_release_outputs(project_root: Path, target_dir: Path) -> None:
    source = target_dir / RELEASE_PROFILE_DIR_NAME
    if not source.is_dir():
        return

    destination = project_root / PROJECT_TARGET_DIR_NAME / RELEASE_PROFILE_DIR_NAME
    destination.mkdir(parents=True, exist_ok=True)

    for entry in source.iterdir():
        if entry.name in SKIP_RELEASE_ENTRIES:
            continue
        if _is_release_artifact(entry):
            _copy_file(entry, destination / entry.name)

    source_lib = source / LIB_DIR_NAME
    if source_lib.is_dir():
        destination_lib = destination / LIB_DIR_NAME
        for entry in source_lib.iterdir():
            if entry.is_file():
                _copy_file(entry, destination_lib / entry.name)


def _find_compiler_build_dir(target_dir: Path) -> Path | None:
    build_root = target_dir / RELEASE_PROFILE_DIR_NAME / BUILD_DIR_NAME
    matches = sorted(build_root.glob(APXM_COMPILER_BUILD_GLOB))
    if not matches:
        return None
    return matches[0].parent


def _build_cli(project_root: Path, target_dir: Path) -> int:
    return _run(
        [
            CARGO,
            CargoCommand.BUILD.value,
            PACKAGE_FLAG,
            APXM_CLI_PACKAGE,
            FEATURES_FLAG,
            DRIVER_METRICS_FEATURES,
            RELEASE_FLAG,
        ],
        project_root=project_root,
        target_dir=target_dir,
    )


def _build_dialect(project_root: Path, target_dir: Path) -> int:
    build_dir = _find_compiler_build_dir(target_dir)
    if build_dir is None:
        result = _build_cli(project_root, target_dir)
        if result != 0:
            return result
        build_dir = _find_compiler_build_dir(target_dir)
    if build_dir is None:
        print("error: unable to locate APXM compiler CMake build directory", file=sys.stderr)
        return 1

    result = _run(
        [CMAKE, BUILD_FLAG, str(build_dir), TARGET_FLAG, TABLEGEN_TARGET],
        project_root=project_root,
        target_dir=target_dir,
    )
    if result != 0:
        return result

    result = _run(
        [CMAKE, BUILD_FLAG, str(build_dir)],
        project_root=project_root,
        target_dir=target_dir,
    )
    if result != 0:
        return result

    result = _build_cli(project_root, target_dir)
    if result == 0:
        _mirror_release_outputs(project_root, target_dir)
    return result


def _clean(project_root: Path, target_dir: Path, args: list[str]) -> int:
    result = _run(
        [CARGO, CargoCommand.CLEAN.value, *args],
        project_root=project_root,
        target_dir=target_dir,
    )
    return result


def main(argv: list[str]) -> int:
    project_root = _repo_root(Path(__file__))
    target_dir = _target_dir(project_root)
    target_dir.mkdir(parents=True, exist_ok=True)

    if argv == [CargoCommand.TARGET_DIR.value]:
        print(target_dir)
        return 0
    if not argv:
        print("error: expected cargo subcommand", file=sys.stderr)
        return 2
    if argv[0] == CargoCommand.BUILD_DIALECT.value:
        return _build_dialect(project_root, target_dir)
    if argv[0] == CargoCommand.CLEAN.value:
        return _clean(project_root, target_dir, argv[1:])

    result = _run([CARGO, *argv], project_root=project_root, target_dir=target_dir)
    if result == 0 and argv[0] == CargoCommand.BUILD.value and RELEASE_FLAG in argv:
        _mirror_release_outputs(project_root, target_dir)
    return result


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
