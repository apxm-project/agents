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
CONVERSATIONAL_OUTPUT = (
    REPOSITORY_ROOT
    / "examples/agents/conversational/artifacts/executable-artifact.v1.json"
)
GAO_OUTPUT = REPOSITORY_ROOT / "examples/agents/gao/artifacts/executable-artifact.v1.json"


def run_json(command: list[str], *, cwd: Path, env: dict[str, str]) -> dict[str, Any]:
    """Run one installed frontend consumer and decode its JSON artifact."""

    completed = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=True,
        capture_output=True,
        text=True,
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
        "import json, pathlib; "
        "import apxm_program; "
        "from agent import build_conversational; "
        f"root=pathlib.Path({str(package_root)!r}).resolve(); "
        "loaded=pathlib.Path(apxm_program.__file__).resolve(); "
        "assert loaded.is_relative_to(root), (loaded, root); "
        "print(json.dumps(apxm_program.compile_artifact("
        "build_conversational().build_graph()), sort_keys=True, separators=(',', ':')))"
    )
    return run_json([sys.executable, "-c", program], cwd=REPOSITORY_ROOT, env=env)


def build_typescript_artifact() -> dict[str, Any]:
    """Compile the Gao example through its installed generic npm dependency."""

    example_root = REPOSITORY_ROOT / "examples/agents/gao"
    installed_frontend = example_root / "node_modules/@apxm/frontend/package.json"
    if not installed_frontend.is_file():
        raise FileNotFoundError(f"missing installed TypeScript frontend: {installed_frontend}")
    program = (
        "import { compileArtifact } from '@apxm/frontend';"
        "import { buildGao } from './dist/index.js';"
        "console.log(JSON.stringify(compileArtifact(buildGao().buildGraph())));"
    )
    return run_json(
        ["node", "--input-type=module", "-e", program],
        cwd=example_root,
        env=os.environ.copy(),
    )


def canonical_bytes(value: dict[str, Any]) -> bytes:
    """Encode a stable repository artifact."""

    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def publish(path: Path, value: dict[str, Any], *, check: bool) -> None:
    """Write or drift-check one generated example artifact."""

    expected = canonical_bytes(value)
    if check:
        if not path.is_file() or path.read_bytes() != expected:
            raise SystemExit(f"example artifact drift: {path.relative_to(REPOSITORY_ROOT)}")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(expected)


def main() -> None:
    """Build or drift-check both example artifacts."""

    parser = argparse.ArgumentParser()
    parser.add_argument("--python-package-root", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    package_root = args.python_package_root.resolve()
    if not package_root.is_dir():
        raise SystemExit(f"installed Python package root is missing: {package_root}")

    publish(
        CONVERSATIONAL_OUTPUT,
        build_python_artifact(package_root),
        check=args.check,
    )
    publish(GAO_OUTPUT, build_typescript_artifact(), check=args.check)


if __name__ == "__main__":
    main()
