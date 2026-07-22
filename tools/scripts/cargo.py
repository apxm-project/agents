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
    SCRUB_SIGBUS_CACHE = "scrub-sigbus-cache"
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
CONFIG_FLAG = "--config"
PROFILE_FLAG = "--profile"
PACKAGE_FLAG = "-p"
FEATURES_FLAG = "--features"
DRIVER_METRICS_FEATURES = "driver,metrics"
APXM_CLI_PACKAGE = "apxm-cli"
APXM_COMPILER_PACKAGE = "apxm-compiler"
TARGET_ROOT_NAME = "apxm-cargo-targets"
PROJECT_TARGET_DIR_NAME = "target"
DEBUG_PROFILE_DIR_NAME = "debug"
RELEASE_PROFILE_DIR_NAME = "release"
FINGERPRINT_DIR_NAME = ".fingerprint"
BUILD_DIR_NAME = "build"
LIB_DIR_NAME = "lib"
CMAKE_CACHE_FILE_NAME = "CMakeCache.txt"
DEPS_DIR_NAME = "deps"
EXAMPLES_DIR_NAME = "examples"
INCREMENTAL_DIR_NAME = "incremental"
APXM_COMPILER_BUILD_GLOB = f"apxm-compiler*/out/{BUILD_DIR_NAME}/{CMAKE_CACHE_FILE_NAME}"
TABLEGEN_TARGET = "AISIRIncGen"
PROJECT_DIGEST_SIZE = 16
EXECUTABLE_SUFFIX = ".exe"
FINGERPRINT_OUTPUT_GLOB = "output-*"
RUSTC_SIGBUS_MARKER = "rustc interrupted by SIGBUS"
TEMP_ARCHIVE_GLOB = ".tmp*.temp-archive"
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


def _command_profile(command: list[str]) -> str:
    if RELEASE_FLAG in command:
        return RELEASE_PROFILE_DIR_NAME
    for idx, value in enumerate(command):
        if value == PROFILE_FLAG and idx + 1 < len(command):
            profile = command[idx + 1].lower()
            if profile == RELEASE_PROFILE_DIR_NAME:
                return RELEASE_PROFILE_DIR_NAME
    for idx, value in enumerate(command):
        if value == CONFIG_FLAG and idx + 1 < len(command):
            config = command[idx + 1].lower()
            if config == RELEASE_PROFILE_DIR_NAME:
                return RELEASE_PROFILE_DIR_NAME
    return DEBUG_PROFILE_DIR_NAME


def _prepend_env_path(env: dict[str, str], key: str, paths: list[Path]) -> None:
    existing = env.get(key, "")
    current = [str(path) for path in paths]
    if existing:
        current.extend(existing.split(os.pathsep))
    if current:
        env[key] = os.pathsep.join(current)


def _cargo_env(project_root: Path, target_dir: Path, command: list[str]) -> dict[str, str]:
    env = dict(os.environ)
    env[EnvKey.CARGO_TARGET_DIR.value] = str(target_dir)
    profile = _command_profile(command)
    project_profile_dir = project_root / PROJECT_TARGET_DIR_NAME / profile
    # The native MLIR bridge is installed into the workspace target/profile
    # directory by the compiler build script, even when Rust artifacts use the
    # machine-local CARGO_TARGET_DIR.  dekk sets LD_LIBRARY_PATH to the release
    # install for normal CLI use; prepend the active profile here so debug tests
    # never load a stale release libapxm_compiler_c.so.
    profile_paths = [project_profile_dir / LIB_DIR_NAME, project_profile_dir]
    _prepend_env_path(env, "LD_LIBRARY_PATH", profile_paths)
    _prepend_env_path(env, "DYLD_LIBRARY_PATH", profile_paths)
    return env


def _run(command: list[str], *, project_root: Path, target_dir: Path) -> int:
    result = subprocess.run(
        command,
        cwd=project_root,
        env=_cargo_env(project_root, target_dir, command),
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
    fd, temp_name = tempfile.mkstemp(
        prefix=f".{destination.name}.",
        suffix=".tmp",
        dir=destination.parent,
    )
    os.close(fd)
    temp_path = Path(temp_name)
    try:
        shutil.copy2(source, temp_path)
        os.replace(temp_path, destination)
    finally:
        try:
            temp_path.unlink()
        except FileNotFoundError:
            pass


def _stage_release_outputs(project_root: Path, target_dir: Path) -> None:
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


def _build_compiler(project_root: Path, target_dir: Path) -> int:
    return _run(
        [
            CARGO,
            CargoCommand.BUILD.value,
            PACKAGE_FLAG,
            APXM_COMPILER_PACKAGE,
            RELEASE_FLAG,
        ],
        project_root=project_root,
        target_dir=target_dir,
    )


def _build_dialect(project_root: Path, target_dir: Path) -> int:
    build_dir = _find_compiler_build_dir(target_dir)
    if build_dir is None:
        result = _build_compiler(project_root, target_dir)
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
        _stage_release_outputs(project_root, target_dir)
    return result


def _clean(project_root: Path, target_dir: Path, args: list[str]) -> int:
    result = _run(
        [CARGO, CargoCommand.CLEAN.value, *args],
        project_root=project_root,
        target_dir=target_dir,
    )
    return result


def _scrub_sigbus_cache(project_root: Path, target_dir: Path) -> int:
    target_roots = [project_root / PROJECT_TARGET_DIR_NAME, target_dir]
    removed = 0

    for root in dict.fromkeys(path.resolve() for path in target_roots):
        if not root.exists():
            continue

        fingerprint_root = root / DEBUG_PROFILE_DIR_NAME / FINGERPRINT_DIR_NAME
        if fingerprint_root.is_dir():
            for output in fingerprint_root.glob(f"*/{FINGERPRINT_OUTPUT_GLOB}"):
                if not output.is_file():
                    continue
                try:
                    content = output.read_text(errors="replace")
                except OSError:
                    continue
                if RUSTC_SIGBUS_MARKER in content:
                    output.unlink()
                    removed += 1

        deps_root = root / DEBUG_PROFILE_DIR_NAME / DEPS_DIR_NAME
        if deps_root.is_dir():
            for entry in deps_root.glob(TEMP_ARCHIVE_GLOB):
                if entry.is_dir():
                    shutil.rmtree(entry)
                    removed += 1

    suffix = "y" if removed == 1 else "ies"
    print(f"removed {removed} stale rustc SIGBUS cache entr{suffix}")
    return 0


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
    if argv[0] == CargoCommand.SCRUB_SIGBUS_CACHE.value:
        return _scrub_sigbus_cache(project_root, target_dir)

    result = _run([CARGO, *argv], project_root=project_root, target_dir=target_dir)
    if result == 0 and argv[0] == CargoCommand.BUILD.value and RELEASE_FLAG in argv:
        _stage_release_outputs(project_root, target_dir)
    return result


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
