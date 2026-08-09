"""Build deterministic executable artifacts from repository example sources."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_ROOT = REPOSITORY_ROOT / "crates/machine/program/tests/fixtures/example-artifacts"
CONVERSATIONAL_PYTHON_OUTPUT = FIXTURE_ROOT / "conversational-python.v2.json"
CONVERSATIONAL_TYPESCRIPT_OUTPUT = FIXTURE_ROOT / "conversational-typescript.v2.json"


def run_json(command: list[str], *, cwd: Path, env: dict[str, str]) -> dict[str, Any]:
    """Run one installed frontend consumer and decode its JSON artifact."""

    completed = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "example compiler failed:\n"
            f"stdout:\n{completed.stdout}\n"
            f"stderr:\n{completed.stderr}"
        )
    value = json.loads(completed.stdout)
    if value.get("schema_version") != "apxm.executable-artifact.v1":
        raise ValueError("example compiler did not emit apxm.executable-artifact.v1")
    return value


def build_python_artifact(package_root: Path) -> dict[str, Any]:
    """Compile the Python example through the installed generic wheel."""

    example_root = REPOSITORY_ROOT / "examples/agents/conversational/python"
    env = os.environ.copy()
    env["PYTHONPATH"] = os.pathsep.join((str(package_root), str(example_root)))
    program = (
        "import json, pathlib; import apxm_program; "
        "from agent import ConversationalExample; "
        f"root=pathlib.Path({str(package_root)!r}).resolve(); "
        "loaded=pathlib.Path(apxm_program.__file__).resolve(); "
        "assert loaded.is_relative_to(root), (loaded, root); "
        "print(json.dumps(ConversationalExample.artifact(), sort_keys=True, separators=(',', ':')))"
    )
    return run_json([sys.executable, "-c", program], cwd=REPOSITORY_ROOT, env=env)


def build_typescript_artifact() -> dict[str, Any]:
    """Compile the TypeScript example through its installed generic dependency."""

    example_root = REPOSITORY_ROOT / "examples/agents/conversational"
    installed_frontend = example_root / "node_modules/@apxm/frontend/package.json"
    if not installed_frontend.is_file():
        raise FileNotFoundError(f"missing installed TypeScript frontend: {installed_frontend}")
    program = (
        "import { buildConversational } from './dist/index.js';"
        "console.log(JSON.stringify(buildConversational().artifact()));"
    )
    return run_json(
        ["node", "--input-type=module", "-e", program],
        cwd=example_root,
        env=os.environ.copy(),
    )


def canonical_bytes(value: dict[str, Any]) -> bytes:
    """Encode a stable runtime-proof fixture."""

    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def publish(path: Path, value: dict[str, Any], *, check: bool) -> None:
    """Write or drift-check one source-derived runtime-proof fixture."""

    expected = canonical_bytes(value)
    if check:
        if not path.is_file() or path.read_bytes() != expected:
            raise SystemExit(f"example artifact drift: {path.relative_to(REPOSITORY_ROOT)}")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(expected)


def main() -> None:
    """Build or drift-check the example-derived runtime-proof fixtures."""

    parser = argparse.ArgumentParser()
    parser.add_argument("--python-package-root", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    package_root = args.python_package_root.resolve()
    if not package_root.is_dir():
        raise SystemExit(f"installed Python package root is missing: {package_root}")

    publish(
        CONVERSATIONAL_PYTHON_OUTPUT,
        build_python_artifact(package_root),
        check=args.check,
    )
    publish(
        CONVERSATIONAL_TYPESCRIPT_OUTPUT,
        build_typescript_artifact(),
        check=args.check,
    )


if __name__ == "__main__":
    main()
