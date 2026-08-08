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
HOST_REQUEST_SCHEMA_ID = "apxm.runtime.host-request.v1"
HOST_REQUEST_SCHEMA_PATH = "reference-host/schemas/apxm.runtime.host-request.v1.json"
HOST_REQUEST_SCHEMA_SOURCE_PATH = "schemas/apxm.runtime.host-request.v1.json"
HOST_REQUEST_VECTOR_PATH = "reference-host/vectors/apxm.runtime.host-request.v1.json"
HOST_REQUEST_VECTOR_SOURCE_PATH = "vectors/apxm.runtime.host-request.v1.json"
HOST_RESPONSE_SCHEMA_ID = "apxm.runtime.host-response.v1"
HOST_RESPONSE_SCHEMA_PATH = "reference-host/schemas/apxm.runtime.host-response.v1.json"
HOST_RESPONSE_SCHEMA_SOURCE_PATH = "schemas/apxm.runtime.host-response.v1.json"
HOST_RESPONSE_VECTOR_PATH = "reference-host/vectors/apxm.runtime.host-response.v1.json"
HOST_RESPONSE_VECTOR_SOURCE_PATH = "vectors/apxm.runtime.host-response.v1.json"
HOST_TRANSPORT_SCHEMA_ID = "apxm.runtime.host-transport.v1"
HOST_TRANSPORT_SCHEMA_PATH = "reference-host/schemas/apxm.runtime.host-transport.v1.json"
HOST_TRANSPORT_SCHEMA_SOURCE_PATH = "schemas/apxm.runtime.host-transport.v1.json"
HOST_TRANSPORT_VECTOR_PATH = "reference-host/vectors/apxm.runtime.host-transport.v1.json"
HOST_TRANSPORT_VECTOR_SOURCE_PATH = "vectors/apxm.runtime.host-transport.v1.json"
EXECUTION_MANIFEST_RELATIVE_PATH = (
    "reference-host/manifests/apxm.reference-host-execution-manifest.v1.json"
)
RELEASE_MANIFEST_RELATIVE_PATH = (
    "contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json"
)


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


def _contract_file(relative_path: str, label: str) -> Path:
    path = REPOSITORY_ROOT / "contracts" / relative_path
    if not path.is_file():
        raise RuntimeError(f"{label} is missing: {relative_path}")
    return path


def _published_contract_entry(
    *,
    manifest: dict[str, Any],
    field: str,
    schema_id: str,
    published_path: str,
    canonical_source_path: str,
    semantic_owner: bool,
) -> dict[str, Any]:
    published_file = _contract_file(published_path, f"reference-host {field} publication")
    source_file = _contract_file(canonical_source_path, f"reference-host {field} source")
    if published_file.read_bytes() != source_file.read_bytes():
        raise RuntimeError(
            f"reference-host {field} published bytes differ from canonical source"
        )
    expected: dict[str, Any] = {
        "schema_id": schema_id,
        "path": published_path,
        "digest": file_digest(published_file),
        "canonical_source_path": canonical_source_path,
    }
    if semantic_owner:
        expected["semantic_owner"] = "agents"
    if manifest.get(field) != expected:
        raise RuntimeError(f"reference-host execution manifest field {field} drifted")
    return expected


def validate_reference_host_publication(
    execution_manifest: dict[str, Any], release_manifest: dict[str, Any]
) -> dict[str, str]:
    """Validate the exact owner-local host-request publication cohort."""
    _published_contract_entry(
        manifest=execution_manifest,
        field="host_request_schema",
        schema_id=HOST_REQUEST_SCHEMA_ID,
        published_path=HOST_REQUEST_SCHEMA_PATH,
        canonical_source_path=HOST_REQUEST_SCHEMA_SOURCE_PATH,
        semantic_owner=True,
    )
    _published_contract_entry(
        manifest=execution_manifest,
        field="host_request_vector",
        schema_id=HOST_REQUEST_SCHEMA_ID,
        published_path=HOST_REQUEST_VECTOR_PATH,
        canonical_source_path=HOST_REQUEST_VECTOR_SOURCE_PATH,
        semantic_owner=False,
    )
    _published_contract_entry(
        manifest=execution_manifest,
        field="host_response_schema",
        schema_id=HOST_RESPONSE_SCHEMA_ID,
        published_path=HOST_RESPONSE_SCHEMA_PATH,
        canonical_source_path=HOST_RESPONSE_SCHEMA_SOURCE_PATH,
        semantic_owner=True,
    )
    _published_contract_entry(
        manifest=execution_manifest,
        field="host_response_vector",
        schema_id=HOST_RESPONSE_SCHEMA_ID,
        published_path=HOST_RESPONSE_VECTOR_PATH,
        canonical_source_path=HOST_RESPONSE_VECTOR_SOURCE_PATH,
        semantic_owner=False,
    )
    _published_contract_entry(
        manifest=execution_manifest,
        field="host_transport_schema",
        schema_id=HOST_TRANSPORT_SCHEMA_ID,
        published_path=HOST_TRANSPORT_SCHEMA_PATH,
        canonical_source_path=HOST_TRANSPORT_SCHEMA_SOURCE_PATH,
        semantic_owner=True,
    )
    _published_contract_entry(
        manifest=execution_manifest,
        field="host_transport_vector",
        schema_id=HOST_TRANSPORT_SCHEMA_ID,
        published_path=HOST_TRANSPORT_VECTOR_PATH,
        canonical_source_path=HOST_TRANSPORT_VECTOR_SOURCE_PATH,
        semantic_owner=False,
    )

    publication_cohort = release_manifest.get("publication_cohort")
    if not isinstance(publication_cohort, dict):
        raise RuntimeError("reference-host release manifest publication_cohort is missing")
    expected_execution_ref = {
        "schema_version": execution_manifest.get("schema_version"),
        "path": EXECUTION_MANIFEST_RELATIVE_PATH,
        "digest": file_digest(EXECUTION_MANIFEST_PATH),
    }
    if publication_cohort.get("manifests") != [expected_execution_ref]:
        raise RuntimeError(
            "reference-host release manifest execution-manifest digest drifted"
        )

    for group, identifier_field, field in (
        ("schemas", "schema_id", "host_request_schema"),
        ("vectors", "schema_id", "host_request_vector"),
        ("schemas", "schema_id", "host_response_schema"),
        ("vectors", "schema_id", "host_response_vector"),
        ("schemas", "schema_id", "host_transport_schema"),
        ("vectors", "schema_id", "host_transport_vector"),
    ):
        entries = publication_cohort.get(group)
        if not isinstance(entries, list):
            raise RuntimeError(
                f"reference-host release manifest publication_cohort {group} is missing"
            )
        schema_id = execution_manifest[field][identifier_field]
        expected_entry = {
            identifier_field: schema_id,
            "path": execution_manifest[field]["path"],
            "digest": execution_manifest[field]["digest"],
        }
        matches = [
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get(identifier_field) == schema_id
        ]
        if matches != [expected_entry]:
            label = "host-request" if schema_id == HOST_REQUEST_SCHEMA_ID else schema_id
            raise RuntimeError(
                f"reference-host release manifest {group} {label} publication drifted"
            )

    release_manifest_digest = file_digest(RELEASE_MANIFEST_PATH)
    fixture = execution_manifest.get("startup_input_preflight_test_fixture")
    if not isinstance(fixture, dict):
        raise RuntimeError("reference-host startup-input preflight declaration is missing")
    raw_fixture_path = fixture.get("artifact_path")
    declared_fixture_digest = fixture.get("artifact_digest")
    if not isinstance(raw_fixture_path, str) or not isinstance(declared_fixture_digest, str):
        raise RuntimeError("reference-host startup-input preflight digest declaration drifted")
    fixture_file = _contract_file(raw_fixture_path, "reference-host startup-input preflight fixture")
    if declared_fixture_digest != file_digest(fixture_file):
        raise RuntimeError("reference-host startup-input preflight fixture digest drifted")
    return {
        "execution_manifest_digest": expected_execution_ref["digest"],
        "release_manifest_digest": release_manifest_digest,
    }


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


def validate_reference_host_release_attestation(
    validator: Any,
    release_attestation: Any,
    *,
    execution_manifest: dict[str, Any] | None = None,
) -> None:
    """Reject release evidence that does not match the canonical runtime inputs."""
    if execution_manifest is None:
        execution_manifest = load_json(EXECUTION_MANIFEST_PATH)
    release_manifest = load_json(RELEASE_MANIFEST_PATH)
    try:
        manifest_digests = validate_reference_host_publication(
            execution_manifest, release_manifest
        )
    except Exception as error:
        raise RuntimeError(
            "reference-host release attestation is inconsistent with canonical "
            f"release/runtime evidence: {error}"
        ) from error
    if not isinstance(release_attestation, dict):
        raise RuntimeError("reference-host release attestation must be an object")
    cohort = release_attestation.get("cohort")
    if not isinstance(cohort, dict):
        raise RuntimeError("reference-host release attestation cohort is missing")
    owner_revision = cohort.get("revision")
    if (
        not isinstance(owner_revision, str)
        or len(owner_revision) != 40
        or any(character not in "0123456789abcdef" for character in owner_revision)
    ):
        raise RuntimeError("reference-host release attestation owner revision is invalid")

    descriptor_digest = validator.descriptor_exact_checksum()
    if cohort.get("descriptor_digest") != descriptor_digest:
        raise RuntimeError(
            "reference-host release attestation descriptor digest is inconsistent"
        )
    expected_attestation = validator.reference_host_release_attestation(
        owner_revision=owner_revision,
        owner_descriptor_digest=descriptor_digest,
    )
    if release_attestation != expected_attestation:
        raise RuntimeError(
            "reference-host release attestation is inconsistent with canonical "
            "release/runtime evidence"
        )

    if cohort.get("manifest_digest") != manifest_digests["release_manifest_digest"]:
        raise RuntimeError(
            "reference-host release attestation release manifest digest is inconsistent"
        )
    release_reference = release_attestation.get("release_manifest")
    expected_release_reference = {
        "path": RELEASE_MANIFEST_RELATIVE_PATH,
        "exact_bytes_digest": manifest_digests["release_manifest_digest"],
    }
    if release_reference != expected_release_reference:
        raise RuntimeError(
            "reference-host release attestation release manifest reference drifted"
        )
    startup_preflight = release_attestation.get("startup_input_preflight")
    fixture = execution_manifest["startup_input_preflight_test_fixture"]
    fixture_file = _contract_file(
        fixture["artifact_path"], "reference-host startup-input preflight fixture"
    )
    expected_startup_preflight_digest = file_digest(fixture_file)
    if not isinstance(startup_preflight, dict):
        raise RuntimeError("reference-host release attestation startup-input preflight is missing")
    if startup_preflight.get("artifact_digest") != fixture["artifact_digest"]:
        raise RuntimeError(
            "reference-host release attestation startup-input artifact digest is inconsistent"
        )
    if startup_preflight.get("exact_bytes_digest") != expected_startup_preflight_digest:
        raise RuntimeError(
            "reference-host release attestation startup-input exact bytes digest is inconsistent"
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
        validate_reference_host_release_attestation(
            validator,
            release_attestation,
            execution_manifest=execution_manifest,
        )
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
