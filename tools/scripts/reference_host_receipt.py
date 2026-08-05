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
import sysconfig
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


def path_evidence(
    path: Path,
    *,
    expected_path: Path | None = None,
    expected_digest: str | None = None,
) -> dict[str, Any]:
    evidence: dict[str, Any] = {
        "path": str(path),
        "exists": path.exists(),
        "is_file": path.is_file(),
    }
    if expected_path is not None:
        evidence["expected_path"] = str(expected_path)
        evidence["path_matches_expected"] = path.resolve(strict=False) == expected_path.resolve(
            strict=False
        )
    if expected_digest is not None:
        evidence["expected_digest"] = expected_digest
    if path.is_file():
        evidence["observed_digest"] = file_digest(path)
        if expected_digest is not None:
            evidence["digest_matches_expected"] = evidence["observed_digest"] == expected_digest
    elif expected_digest is not None:
        evidence["digest_matches_expected"] = False
    return evidence


def startup_input_preflight_evidence(execution_manifest: dict[str, Any]) -> dict[str, Any]:
    fixture = execution_manifest.get("startup_input_preflight_test_fixture")
    if not isinstance(fixture, dict):
        return {
            "status": "missing-declaration",
            "expected_path": None,
            "exists": False,
            "is_file": False,
            "digest_matches_expected": False,
        }
    raw_path = fixture.get("artifact_path")
    if not isinstance(raw_path, str) or not raw_path:
        return {
            "status": "invalid-declaration",
            "expected_path": raw_path if isinstance(raw_path, str) else None,
            "exists": False,
            "is_file": False,
            "digest_matches_expected": False,
        }
    artifact_path = REPOSITORY_ROOT / "contracts" / raw_path
    evidence = path_evidence(
        artifact_path,
        expected_digest=fixture.get("artifact_digest")
        if isinstance(fixture.get("artifact_digest"), str)
        else None,
    )
    evidence["scope"] = fixture.get("scope")
    evidence["artifact_path"] = raw_path
    return evidence


def provenance_evidence(validator: Any, execution_manifest: dict[str, Any]) -> dict[str, Any]:
    return {
        "owner_descriptor": path_evidence(validator.DESCRIPTOR_PATH),
        "owner_descriptor_exact_checksum": path_evidence(
            validator.DESCRIPTOR_SIDECAR_PATH
        ),
        "release_manifest": path_evidence(validator.REFERENCE_HOST_RELEASE_MANIFEST_PATH),
        "startup_input_preflight": startup_input_preflight_evidence(execution_manifest),
    }


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


def build_command_target(command: list[str]) -> str | None:
    for index, value in enumerate(command):
        if value == "--target" and index + 1 < len(command):
            return command[index + 1]
        if value.startswith("--target="):
            return value.split("=", 1)[1]
    return None


def binary_platform_evidence(command: list[str]) -> dict[str, Any]:
    target = build_command_target(command)
    return {
        "selection": "explicit-target" if target else "native-host-default",
        "target": target,
        "system": platform.system().lower() or "unknown",
        "machine": platform.machine().lower() or "unknown",
        "platform_tag": target or sysconfig.get_platform(),
    }


def committed_reference_host_release_attestation(validator: Any) -> dict[str, Any]:
    revision = git_stdout("rev-parse", "HEAD")
    if not revision:
        raise RuntimeError("owner revision is unavailable")
    status = git_stdout("status", "--porcelain")
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
    release_attestation: dict[str, Any],
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
        "build_identity": {
            "owner_revision": release_attestation["cohort"]["revision"],
            "release_manifest_path": release_attestation["release_manifest"]["path"],
            "execution_manifest_path": str(EXECUTION_MANIFEST_PATH.relative_to(REPOSITORY_ROOT)),
            "binary_platform": binary_platform_evidence(BUILD_COMMAND),
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
            "startup_input_preflight": release_attestation["startup_input_preflight"],
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
    execution_manifest = load_json(EXECUTION_MANIFEST_PATH)
    receipt["evidence"] = provenance_evidence(validator, execution_manifest)
    try:
        release_attestation = committed_reference_host_release_attestation(validator)
        receipt["reference_host_release"] = release_attestation
    except Exception as error:
        receipt["status"] = "unverifiable-release-cohort"
        receipt["error"] = {
            "code": "unverifiable_release_cohort",
            "message": f"reference-host release cohort is unverifiable: {error}",
        }
        receipt["evidence"]["owner_release_provenance_verified"] = False
        write_json(receipt_path, receipt)
        return 1, receipt
    receipt["evidence"]["owner_release_provenance_verified"] = True
    resolved_target_dir = resolve_target_dir(target_dir)
    expected_output_path = canonical_output_path(
        resolved_target_dir, execution_manifest["owner_executable"]
    )
    resolved_output_path = output_path or expected_output_path
    expected_source_path = manifest_source_path(execution_manifest)
    resolved_source_path = source_path or expected_source_path

    receipt.update(
        base_receipt(
            execution_manifest=execution_manifest,
            validator=validator,
            receipt_path=receipt_path,
            target_dir=resolved_target_dir,
            output_path=resolved_output_path,
            release_attestation=release_attestation,
        )
    )
    receipt["source"] = path_evidence(
        resolved_source_path,
        expected_path=expected_source_path,
    )
    receipt["source"]["repo_relative_path"] = execution_manifest["owner_executable_path"]

    if not receipt["source"]["path_matches_expected"]:
        receipt["status"] = "mismatched-source"
        receipt["error"] = {
            "code": "mismatched_source",
            "message": (
                "reference-host source path does not match the execution manifest; "
                "the build gate stays closed"
            ),
        }
        write_json(receipt_path, receipt)
        return 1, receipt

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

    receipt["output"] = path_evidence(
        resolved_output_path,
        expected_path=expected_output_path,
    )
    if not receipt["output"]["path_matches_expected"]:
        receipt["status"] = "mismatched-output"
        receipt["error"] = {
            "code": "mismatched_output",
            "message": (
                "reference-host output path does not match the canonical release path; "
                "the build gate stays closed"
            ),
        }
        write_json(receipt_path, receipt)
        return 1, receipt

    build_exit_code = run_build(BUILD_COMMAND)
    if build_exit_code != 0:
        receipt["status"] = "build-failed"
        receipt["error"] = {
            "code": "build_failed",
            "message": f"reference-host build exited with status {build_exit_code}",
        }
        write_json(receipt_path, receipt)
        return build_exit_code, receipt

    receipt["output"] = path_evidence(
        resolved_output_path,
        expected_path=expected_output_path,
    )
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

    if not os.access(resolved_output_path, os.X_OK):
        receipt["status"] = "non-executable-output"
        receipt["error"] = {
            "code": "non_executable_output",
            "message": (
                "reference-host build produced a non-executable file at the canonical "
                "release path"
            ),
        }
        write_json(receipt_path, receipt)
        return 1, receipt

    receipt["status"] = "built"
    receipt["output"].update({
        "path": str(resolved_output_path),
        "digest": file_digest(resolved_output_path),
    })
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
