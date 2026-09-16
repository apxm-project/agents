#!/usr/bin/env python3
"""Provision the service-only toolchain environment.

`dekk agents setup` builds the full environment from `[environment.packages]`
in `.dekk.toml`: MLIR 22, libclang, clang, the conda GCC pair, CMake and Ninja
on top of Python and Node. That set exists for one crate — `apxm-compiler`,
which links the native AIS dialect behind its `mlir` Cargo feature — and no
other crate in the workspace depends on it.

This provisions the same conda prefix with that MLIR half removed: Python,
Node, uv, pytest and git only. Rust comes from the host toolchain the way it
does for the full environment. What it provisions is exactly what
`dekk agents check-service` needs, which is why the service-only gate calls
this and never calls `setup`.

The package set is not a second list: it is `[environment.packages]` minus
`MLIR_ONLY_PACKAGES`, so a package added to `.dekk.toml` reaches both
environments unless it is named there.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
DEKK_MANIFEST_PATH = REPOSITORY_ROOT / ".dekk.toml"

#: Packages in `[environment.packages]` that exist only so `apxm-compiler` can
#: build the dialect with its `mlir` feature: the MLIR/LLVM runtime and headers
#: (`mlir`), the libclang bindgen parses the C API with (`libclang13`, `clang`),
#: the C++ compiler CMake configures against (`gxx_linux-64`), and the CMake and
#: Ninja that drive the build. Nothing outside `crates/compiler/pipeline` reads
#: any of them.
MLIR_ONLY_PACKAGES: frozenset[str] = frozenset(
    {"mlir", "libclang13", "clang", "gxx_linux-64", "cmake", "ninja"}
)

#: conda front-ends in preference order. `micromamba` and `mamba` solve the
#: service set in well under a minute; `conda` is the available fallback.
CONDA_EXECUTABLES: tuple[str, ...] = ("micromamba", "mamba", "conda")


def load_environment(manifest_path: Path = DEKK_MANIFEST_PATH) -> dict[str, object]:
    """Return the `[environment]` table from `.dekk.toml`."""
    with manifest_path.open("rb") as handle:
        manifest = tomllib.load(handle)
    environment = manifest.get("environment")
    if not isinstance(environment, dict):
        raise KeyError(f"{manifest_path.name} declares no [environment]")
    return environment


def service_packages(environment: dict[str, object]) -> list[str]:
    """Return the conda package specs the service environment installs."""
    packages = environment.get("packages")
    if not isinstance(packages, dict):
        raise KeyError("[environment.packages] is missing or not a table")
    specs: list[str] = []
    for name, version in packages.items():
        if name in MLIR_ONLY_PACKAGES:
            continue
        specs.append(f"{name}={version}" if version else name)
    return specs


def environment_prefix(environment: dict[str, object]) -> Path:
    """Return the conda prefix `[environment].path` resolves to."""
    path = environment.get("path")
    if not isinstance(path, str):
        raise KeyError("[environment].path is missing")
    expanded = path.replace("{project}", str(REPOSITORY_ROOT)).replace(
        "{home}", str(Path.home())
    )
    return Path(expanded).expanduser()


def channels(environment: dict[str, object]) -> list[str]:
    value = environment.get("channels")
    return list(value) if isinstance(value, list) else ["conda-forge"]


def find_conda() -> str:
    for executable in CONDA_EXECUTABLES:
        found = shutil.which(executable)
        if found:
            return found
    raise RuntimeError(
        "no conda front-end on PATH; install micromamba, mamba or conda "
        f"(looked for {', '.join(CONDA_EXECUTABLES)})"
    )


def provision_command(
    conda: str, prefix: Path, specs: list[str], channel_list: list[str], *, exists: bool
) -> list[str]:
    """Build the conda invocation that provisions the service prefix.

    An existing prefix is installed into rather than recreated: a developer who
    already carries the full environment keeps MLIR and simply gains anything
    the service set adds.
    """
    subcommand = "install" if exists else "create"
    command = [conda, subcommand, "--yes", "--prefix", str(prefix)]
    for channel in channel_list:
        command.extend(["--channel", channel])
    if Path(conda).name != "micromamba":
        command.append("--override-channels")
    command.extend(specs)
    return command


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--list",
        action="store_true",
        help="print the resolved service package set and exit",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="print the conda invocation without running it",
    )
    args = parser.parse_args(argv)

    environment = load_environment()
    specs = service_packages(environment)
    if args.list:
        for spec in specs:
            print(spec)
        return 0

    prefix = environment_prefix(environment)
    command = provision_command(
        find_conda(),
        prefix,
        specs,
        channels(environment),
        exists=prefix.exists(),
    )
    print("$ " + " ".join(command), flush=True)
    if args.dry_run:
        return 0

    prefix.parent.mkdir(parents=True, exist_ok=True)
    completed = subprocess.run(command, cwd=REPOSITORY_ROOT, env=os.environ, check=False)
    if completed.returncode != 0:
        print("install-service: provisioning failed", file=sys.stderr)
        return completed.returncode
    print(f"install-service: service toolchain ready at {prefix}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
