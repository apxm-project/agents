#!/usr/bin/env python3
"""Build the canonical reference-host executable and emit an exact receipt."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import platform
import subprocess
import sys
from pathlib import Path
from typing import Any, Callable


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
VALIDATOR_PATH = REPOSITORY_ROOT / "contracts" / "tools" / "validate_owner_descriptor.py"
EXECUTION_MANIFEST_PATH = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "manifests"
    / "apxm.reference-host-execution-manifest.v1.json"
)
RELEASE_MANIFEST_PATH = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "manifests"
    / "apxm.reference-host-release-manifest.v1.json"
)
CARGO_WRAPPER_PATH = REPOSITORY_ROOT / "tools" / "scripts" / "cargo.py"
DEFAULT_RECEIPT_PATH = (
    REPOSITORY_ROOT
    / ".apxm"
    / "reference-host"
    / "receipts"
    / "apxm.reference-host-build-receipt.v1.json"
)
RECEIPT_SCHEMA_VERSION = "apxm.reference-host-build-receipt.v1"
BUILD_COMMAND = [
    "python",
    "tools/scripts/cargo.py",
    "build",
    "-p",
    "apxm-cli",
    "--bin",
    "apxm-reference-host",
    "--release",
]
TARGET_DIR_COMMAND = ["python", "tools/scripts/cargo.py", "target-dir"]
STARTUP_INPUT_SCHEMA = "apxm.reference-host-startup-input.v1"
STARTUP_INPUT_FLAG = "--startup-input"


def load_validator_module():
    spec = importlib.util.spec_from_file_location("validate_owner_descriptor", VALIDATOR_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load validate_owner_descriptor.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def file_digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def canonical_output_path(target_dir: Path, executable: str) -> Path:
    return target_dir / "release" / executable


def manifest_source_path(execution_manifest: dict[str, Any]) -> Path:
    return REPOSITORY_ROOT / execution_manifest["owner_executable_path"]


def resolve_target_dir(explicit_target_dir: Path | None) -> Path:
    if explicit_target_dir is not None:
        return explicit_target_dir
    output = subprocess.check_output(
        [sys.executable, str(CARGO_WRAPPER_PATH), "target-dir"],
        cwd=REPOSITORY_ROOT,
        text=True,
    ).strip()
    return Path(output)


def build_runner(command: list[str]) -> int:
    result = subprocess.run(
        [sys.executable, str(CARGO_WRAPPER_PATH), *command[2:]],
        cwd=REPOSITORY_ROOT,
        check=False,
    )
    return int(result.returncode)


def git_stdout(*args: str) -> str:
    env = os.environ.copy()
    if platform.system() == "Darwin":
        env.pop("DYLD_LIBRARY_PATH", None)
        env.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
        env.pop("LD_LIBRARY_PATH", None)
    result = subprocess.run(
        ["git", *args],
        cwd=REPOSITORY_ROOT,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or str(result.returncode)
        raise RuntimeError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout.strip()


def committed_reference_host_release_attestation(validator: Any) -> dict[str, Any]:
    revision = git_stdout("rev-parse", "HEAD")
    if not revision:
        raise RuntimeError("owner revision is unavailable")
    status = git_stdout("status", "--porcelain", "--ignored=matching")
    if status:
        raise RuntimeError(
            "owner checkout is dirty; committed reference-host release inputs are unavailable"
        )
    descriptor_digest = validator.descriptor_exact_checksum()
    return validator.reference_host_release_attestation(
        owner_revision=revision,
        owner_descriptor_digest=descriptor_digest,
    )


def base_receipt(
    *,
    execution_manifest: dict[str, Any],
    validator: Any,
    receipt_path: Path,
    target_dir: Path,
    output_path: Path,
) -> dict[str, Any]:
    return {
        "schema_version": RECEIPT_SCHEMA_VERSION,
        "semantic_owner": "agents",
        "owner_executable": execution_manifest["owner_executable"],
        "owner_executable_path": execution_manifest["owner_executable_path"],
        "execution_manifest": {
            "path": str(EXECUTION_MANIFEST_PATH.relative_to(REPOSITORY_ROOT)),
            "digest": file_digest(EXECUTION_MANIFEST_PATH),
        },
        "referenced_host_sdk": {
            "name": validator.REFERENCE_HOST_DEPENDENCY_NAME,
            "git": validator.REFERENCE_HOST_DEPENDENCY_GIT,
            "source_revision": validator.REFERENCE_HOST_DESCRIPTOR["source_revision"],
            "descriptor_semantic_digest": validator.REFERENCE_HOST_DESCRIPTOR[
                "descriptor_semantic_digest"
            ],
            "descriptor_exact_checksum": validator.REFERENCE_HOST_DESCRIPTOR[
                "descriptor_exact_checksum"
            ],
        },
        "build": {
            "command": BUILD_COMMAND,
            "target_dir_command": TARGET_DIR_COMMAND,
            "target_dir": str(target_dir),
            "output_path": str(output_path),
            "receipt_path": str(receipt_path),
        },
        "runtime_startup": {
            "required_argv": [STARTUP_INPUT_FLAG, "<path>"],
            "startup_input_schema": STARTUP_INPUT_SCHEMA,
            "reference_host_release_manifest": {
                "path": str(RELEASE_MANIFEST_PATH.relative_to(REPOSITORY_ROOT)),
                "digest": file_digest(RELEASE_MANIFEST_PATH),
            },
        },
    }


def write_reference_host_receipt(
    *,
    receipt_path: Path = DEFAULT_RECEIPT_PATH,
    source_path: Path | None = None,
    target_dir: Path | None = None,
    output_path: Path | None = None,
    run_build: Callable[[list[str]], int] = build_runner,
) -> tuple[int, dict[str, Any]]:
    receipt: dict[str, Any] = {
        "schema_version": RECEIPT_SCHEMA_VERSION,
        "semantic_owner": "agents",
    }
    validator = load_validator_module()
    try:
        receipt["reference_host_release"] = committed_reference_host_release_attestation(
            validator
        )
    except Exception as error:
        receipt["status"] = "unverifiable-release-cohort"
        receipt["error"] = {
            "code": "unverifiable_release_cohort",
            "message": f"reference-host release cohort is unverifiable: {error}",
        }
        write_json(receipt_path, receipt)
        return 1, receipt
    execution_manifest = load_json(EXECUTION_MANIFEST_PATH)
    resolved_target_dir = resolve_target_dir(target_dir)
    resolved_output_path = output_path or canonical_output_path(
        resolved_target_dir, execution_manifest["owner_executable"]
    )
    resolved_source_path = source_path or manifest_source_path(execution_manifest)

    receipt.update(
        base_receipt(
            execution_manifest=execution_manifest,
            validator=validator,
            receipt_path=receipt_path,
            target_dir=resolved_target_dir,
            output_path=resolved_output_path,
        )
    )
    receipt["source"] = {
        "path": str(resolved_source_path),
        "repo_relative_path": execution_manifest["owner_executable_path"],
        "exists": resolved_source_path.is_file(),
    }

    if not resolved_source_path.is_file():
        receipt["status"] = "missing-source"
        receipt["error"] = {
            "code": "missing_source",
            "message": (
                "canonical reference-host source is missing; the build gate stays closed"
            ),
        }
        write_json(receipt_path, receipt)
        return 1, receipt

    receipt["source"]["digest"] = file_digest(resolved_source_path)

    build_exit_code = run_build(BUILD_COMMAND)
    if build_exit_code != 0:
        receipt["status"] = "build-failed"
        receipt["error"] = {
            "code": "build_failed",
            "message": f"reference-host build exited with status {build_exit_code}",
        }
        write_json(receipt_path, receipt)
        return build_exit_code, receipt

    if not resolved_output_path.is_file():
        receipt["status"] = "missing-output"
        receipt["error"] = {
            "code": "missing_output",
            "message": (
                "reference-host build completed without producing the expected executable path"
            ),
        }
        write_json(receipt_path, receipt)
        return 1, receipt

    receipt["status"] = "built"
    receipt["output"] = {
        "path": str(resolved_output_path),
        "digest": file_digest(resolved_output_path),
    }
    write_json(receipt_path, receipt)
    return 0, receipt


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt-path", type=Path, default=DEFAULT_RECEIPT_PATH)
    parser.add_argument("--source-path", type=Path)
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--output-path", type=Path)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    exit_code, receipt = write_reference_host_receipt(
        receipt_path=args.receipt_path,
        source_path=args.source_path,
        target_dir=args.target_dir,
        output_path=args.output_path,
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
