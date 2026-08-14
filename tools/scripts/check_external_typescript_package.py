"""Compile an external TypeScript Agent Program through the public frontend."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import tomllib
from pathlib import Path
from typing import Any


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
FRONTEND_ROOT = REPOSITORY_ROOT / "crates/compiler/frontend/typescript"
STAGING_ROOT = REPOSITORY_ROOT / ".apxm/external-typescript-package"
SEMANTIC_OPERATIONS = {
    "model.call",
    "capability.invoke",
    "program.new",
    "program.invoke",
    "await.event",
}


def fail(message: str) -> None:
    """Stop the conformance run with one actionable diagnostic."""

    raise SystemExit(message)


def load_json(path: Path) -> dict[str, Any]:
    """Load a JSON object from a package manifest."""

    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        fail(f"failed to read JSON object {path}: {error}")
    if not isinstance(value, dict):
        fail(f"expected a JSON object in {path}")
    return value


def validate_package(source_root: Path) -> None:
    """Validate the source package's public compile declarations."""

    agent_path = source_root / "agent.toml"
    package_path = source_root / "package.json"
    try:
        agent = tomllib.loads(agent_path.read_text())
    except (OSError, tomllib.TOMLDecodeError) as error:
        fail(f"failed to read {agent_path}: {error}")

    compile_declaration = agent.get("compile")
    if not isinstance(compile_declaration, dict):
        fail(f"{agent_path} must declare [compile]")
    if compile_declaration.get("frontend") != "typescript":
        fail(f"{agent_path} must declare [compile].frontend = \"typescript\"")
    entry = compile_declaration.get("entry")
    if not isinstance(entry, str):
        fail(f"{agent_path} must declare a string [compile].entry")
    entry_path = Path(entry)
    if entry_path.is_absolute() or ".." in entry_path.parts or entry_path.suffix != ".ts":
        fail(f"{agent_path} [compile].entry must be a package-relative TypeScript path")
    if not (source_root / entry_path).is_file():
        fail(f"declared TypeScript entry does not exist: {source_root / entry_path}")

    package = load_json(package_path)
    scripts = package.get("scripts")
    if not isinstance(scripts, dict):
        fail(f"{package_path} must declare scripts")
    for script in ("build", "compile", "run"):
        if not isinstance(scripts.get(script), str):
            fail(f"{package_path} must declare a {script!r} script")

    dependencies = package.get("dependencies")
    frontend_version = load_json(FRONTEND_ROOT / "package.json").get("version")
    if not isinstance(dependencies, dict) or dependencies.get("@apxm/frontend") != frontend_version:
        fail(
            f"{package_path} must depend on @apxm/frontend at the exact public version "
            f"{frontend_version!r}"
        )


def run(command: list[str], *, cwd: Path) -> str:
    """Run one package command and return its standard output."""

    completed = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if completed.returncode != 0:
        fail(
            f"command failed in {cwd}: {' '.join(command)}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed.stdout.strip()


def run_json_script(package_root: Path, script: str) -> dict[str, Any]:
    """Run a package script whose only output is one JSON object."""

    output = run(["npm", "run", "--silent", script], cwd=package_root)
    try:
        value = json.loads(output)
    except json.JSONDecodeError as error:
        fail(f"npm script {script!r} did not emit one JSON value: {error}")
    if not isinstance(value, dict):
        fail(f"npm script {script!r} must emit a JSON object")
    return value


def validate_frontend_graph(graph: dict[str, Any]) -> None:
    """Require the ordinary TypeScript FrontendGraph contract."""

    if graph.get("schema_version") != "apxm.frontend-graph":
        fail("external package did not emit apxm.frontend-graph")
    if graph.get("source_language") != "typescript":
        fail("external package FrontendGraph did not record source_language=typescript")
    call_intents = graph.get("call_intents")
    control_intents = graph.get("control_intents")
    if not isinstance(call_intents, list) or not isinstance(control_intents, list):
        fail("external package FrontendGraph is missing typed call/control intents")
    intent_kinds = {intent.get("intent_kind") for intent in call_intents if isinstance(intent, dict)}
    control_kinds = {
        intent.get("control_kind") for intent in control_intents if isinstance(intent, dict)
    }
    if not {"tool_invocation", "model_invocation"}.issubset(intent_kinds):
        fail("external package must author typed Tool and Model calls")
    if not {"loop", "yield"}.issubset(control_kinds):
        fail("external package must author its loop and yield/resume in source")


def validate_air(air: dict[str, Any]) -> None:
    """Require canonical AIR with no product-specific operation family."""

    if air.get("schema_version") != "apxm.air":
        fail("external package did not emit apxm.air")
    operations = air.get("semantic_operations")
    structural_ir = air.get("structural_ir")
    if not isinstance(operations, list) or not isinstance(structural_ir, list):
        fail("external package AIR is missing semantic_operations or structural_ir")
    operation_kinds = {
        operation.get("op") for operation in operations if isinstance(operation, dict)
    }
    unknown = operation_kinds - SEMANTIC_OPERATIONS
    if unknown:
        fail(f"external package emitted operations outside the closed family: {sorted(unknown)}")
    if not {"model.call", "capability.invoke"}.issubset(operation_kinds):
        fail("external package AIR must contain ordinary Model and Capability operations")
    structural_kinds = {
        region.get("kind") for region in structural_ir if isinstance(region, dict)
    }
    if not {"ais.loop", "yield"}.issubset(structural_kinds):
        fail("external package AIR must contain compiler-emitted loop and yield structure")
    source_map = air.get("source_map")
    if not isinstance(source_map, dict) or source_map.get("source_language") != "typescript":
        fail("external package AIR source map did not preserve TypeScript provenance")


def stage_source(source_root: Path) -> Path:
    """Copy source inputs into the repository-local generated-artifact root."""

    if STAGING_ROOT.exists():
        shutil.rmtree(STAGING_ROOT)
    shutil.copytree(
        source_root,
        STAGING_ROOT,
        ignore=shutil.ignore_patterns(".git", "dist", "node_modules", "*.tsbuildinfo"),
    )
    return STAGING_ROOT


def main() -> None:
    """Run external-source frontend and compiler conformance."""

    parser = argparse.ArgumentParser()
    parser.add_argument("package", type=Path)
    args = parser.parse_args()
    source_root = args.package.resolve()
    if not source_root.is_dir():
        fail(f"external TypeScript package does not exist: {source_root}")
    validate_package(source_root)

    package_root = stage_source(source_root)
    run(
        [
            "npm",
            "install",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--no-package-lock",
            "--no-save",
            str(FRONTEND_ROOT),
        ],
        cwd=package_root,
    )
    run(["npm", "run", "--silent", "build"], cwd=package_root)

    first_graph = run_json_script(package_root, "run")
    second_graph = run_json_script(package_root, "run")
    if first_graph != second_graph:
        fail("external package FrontendGraph output is not deterministic")
    validate_frontend_graph(first_graph)

    first_air = run_json_script(package_root, "compile")
    second_air = run_json_script(package_root, "compile")
    if first_air != second_air:
        fail("external package AIR output is not deterministic")
    validate_air(first_air)


if __name__ == "__main__":
    main()
