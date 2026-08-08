#!/usr/bin/env python3
"""Emit a deterministic exact startup-input artifact for the reference host."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import platform
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
VALIDATOR_PATH = REPOSITORY_ROOT / "contracts" / "tools" / "validate_owner_descriptor.py"
RELEASE_MANIFEST_PATH = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "manifests"
    / "apxm.reference-host-release-manifest.v1.json"
)
EXECUTION_MANIFEST_PATH = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "manifests"
    / "apxm.reference-host-execution-manifest.v1.json"
)
DEFAULT_OUTPUT_PATH = (
    REPOSITORY_ROOT
    / ".apxm"
    / "reference-host"
    / "startup-inputs"
    / "apxm.reference-host-startup-input.v1.json"
)
STARTUP_INPUT_SCHEMA = "apxm.reference-host-startup-input.v1"
OWNER_EXECUTABLE = "apxm-reference-host"
OWNER_EXECUTABLE_PATH = "crates/tools/cli/src/bin/reference_host.rs"
TRANSPORT_PROTOCOL = "jsonl-stdin-stdout"
PRIVATE_TRANSPORT_PROTOCOL = "jsonl-unix-stream"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^sha256:[0-9a-f]{64}$")
FAIL_CLOSED_ON = ["missing", "placeholder", "dirty", "mismatched", "implicit-default"]
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


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def load_validator_module():
    spec = importlib.util.spec_from_file_location("validate_owner_descriptor", VALIDATOR_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load validate_owner_descriptor.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def file_digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


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


def is_placeholder_digest(digest: str) -> bool:
    hex_value = digest.removeprefix("sha256:")
    return len(set(hex_value)) == 1


def validate_revision(owner_revision: str) -> None:
    if not owner_revision:
        raise ValueError("owner_revision must be explicit; implicit/default values are rejected")
    if HEX40.fullmatch(owner_revision) is None:
        raise ValueError("owner_revision must be a lowercase 40-hex git revision")


def validate_exact_digest(name: str, value: str) -> None:
    if not value:
        raise ValueError(f"{name} must be explicit; implicit/default values are rejected")
    if HEX64.fullmatch(value) is None:
        raise ValueError(f"{name} must be a lowercase sha256:<64 hex> digest")
    if is_placeholder_digest(value):
        raise ValueError(f"{name} must not be a placeholder digest")


def _contract_file(relative_path: str, label: str) -> Path:
    path = REPOSITORY_ROOT / "contracts" / relative_path
    if not path.is_file():
        raise RuntimeError(f"{label} is missing: {relative_path}")
    return path


def _published_contract_entry(
    *,
    execution_manifest: dict[str, Any],
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
    if execution_manifest.get(field) != expected:
        raise RuntimeError(f"reference-host execution manifest field {field} drifted")
    return expected


def validate_reference_host_publication() -> dict[str, str]:
    """Validate owner-local host-request publication and manifest digests."""
    execution_manifest = load_json(EXECUTION_MANIFEST_PATH)
    release_manifest = load_json(RELEASE_MANIFEST_PATH)
    _published_contract_entry(
        execution_manifest=execution_manifest,
        field="host_request_schema",
        schema_id=HOST_REQUEST_SCHEMA_ID,
        published_path=HOST_REQUEST_SCHEMA_PATH,
        canonical_source_path=HOST_REQUEST_SCHEMA_SOURCE_PATH,
        semantic_owner=True,
    )
    _published_contract_entry(
        execution_manifest=execution_manifest,
        field="host_request_vector",
        schema_id=HOST_REQUEST_SCHEMA_ID,
        published_path=HOST_REQUEST_VECTOR_PATH,
        canonical_source_path=HOST_REQUEST_VECTOR_SOURCE_PATH,
        semantic_owner=False,
    )
    _published_contract_entry(
        execution_manifest=execution_manifest,
        field="host_response_schema",
        schema_id=HOST_RESPONSE_SCHEMA_ID,
        published_path=HOST_RESPONSE_SCHEMA_PATH,
        canonical_source_path=HOST_RESPONSE_SCHEMA_SOURCE_PATH,
        semantic_owner=True,
    )
    _published_contract_entry(
        execution_manifest=execution_manifest,
        field="host_response_vector",
        schema_id=HOST_RESPONSE_SCHEMA_ID,
        published_path=HOST_RESPONSE_VECTOR_PATH,
        canonical_source_path=HOST_RESPONSE_VECTOR_SOURCE_PATH,
        semantic_owner=False,
    )
    _published_contract_entry(
        execution_manifest=execution_manifest,
        field="host_transport_schema",
        schema_id=HOST_TRANSPORT_SCHEMA_ID,
        published_path=HOST_TRANSPORT_SCHEMA_PATH,
        canonical_source_path=HOST_TRANSPORT_SCHEMA_SOURCE_PATH,
        semantic_owner=True,
    )
    _published_contract_entry(
        execution_manifest=execution_manifest,
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
    for group, field in (
        ("schemas", "host_request_schema"),
        ("vectors", "host_request_vector"),
        ("schemas", "host_response_schema"),
        ("vectors", "host_response_vector"),
        ("schemas", "host_transport_schema"),
        ("vectors", "host_transport_vector"),
    ):
        entries = publication_cohort.get(group)
        if not isinstance(entries, list):
            raise RuntimeError(
                f"reference-host release manifest publication_cohort {group} is missing"
            )
        schema_id = execution_manifest[field]["schema_id"]
        expected_entry = {
            "schema_id": schema_id,
            "path": execution_manifest[field]["path"],
            "digest": execution_manifest[field]["digest"],
        }
        matches = [
            entry
            for entry in entries
            if isinstance(entry, dict) and entry.get("schema_id") == schema_id
        ]
        if matches != [expected_entry]:
            raise RuntimeError(
                f"reference-host release manifest {group} host-request publication drifted"
            )
    return {
        "execution_manifest_digest": expected_execution_ref["digest"],
        "release_manifest_digest": file_digest(RELEASE_MANIFEST_PATH),
    }


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def build_startup_input(
    *,
    owner_revision: str,
    release_digest: str,
    port_bindings_digest: str,
    resource_ceiling_digest: str,
    transport_protocol: str = TRANSPORT_PROTOCOL,
    output_path: Path = DEFAULT_OUTPUT_PATH,
) -> dict[str, Any]:
    validate_revision(owner_revision)
    validate_exact_digest("release_digest", release_digest)
    validate_exact_digest("port_bindings_digest", port_bindings_digest)
    validate_exact_digest("resource_ceiling_digest", resource_ceiling_digest)
    if transport_protocol not in {TRANSPORT_PROTOCOL, PRIVATE_TRANSPORT_PROTOCOL}:
        raise RuntimeError(f"unsupported reference-host transport protocol: {transport_protocol}")

    checkout_revision = git_stdout("rev-parse", "HEAD")
    if owner_revision != checkout_revision:
        raise RuntimeError(
            "owner revision mismatch: "
            f"expected {owner_revision}, checkout is {checkout_revision}"
        )
    status = git_stdout("status", "--porcelain")
    if status:
        raise RuntimeError(
            "owner checkout is dirty; exact reference-host startup input requires committed bytes"
        )

    manifest_digests = validate_reference_host_publication()
    validator = load_validator_module()
    descriptor_exact_checksum = validator.descriptor_exact_checksum()
    payload = {
        "schema_version": STARTUP_INPUT_SCHEMA,
        "semantic_owner": "agents",
        "owner_executable": OWNER_EXECUTABLE,
        "owner_executable_path": OWNER_EXECUTABLE_PATH,
        "transport_protocol": transport_protocol,
        "reference_host_release_manifest": {
            "path": str(RELEASE_MANIFEST_PATH.resolve()),
            "digest": manifest_digests["release_manifest_digest"],
        },
        "release_digest": release_digest,
        "port_bindings_digest": port_bindings_digest,
        "resource_ceiling_digest": resource_ceiling_digest,
        "provenance": {
            "owner_revision": owner_revision,
            "descriptor_semantic_digest": validator.REFERENCE_HOST_DESCRIPTOR[
                "descriptor_semantic_digest"
            ],
            "descriptor_exact_checksum": descriptor_exact_checksum,
            "dirty": False,
        },
        "fail_closed_on": FAIL_CLOSED_ON,
    }
    write_json(output_path, payload)
    return payload


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--owner-revision", required=True)
    parser.add_argument("--release-digest", required=True)
    parser.add_argument("--port-bindings-digest", required=True)
    parser.add_argument("--resource-ceiling-digest", required=True)
    parser.add_argument(
        "--transport-protocol",
        choices=(TRANSPORT_PROTOCOL, PRIVATE_TRANSPORT_PROTOCOL),
        default=TRANSPORT_PROTOCOL,
    )
    parser.add_argument("--output-path", type=Path, default=DEFAULT_OUTPUT_PATH)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    payload = build_startup_input(
        owner_revision=args.owner_revision,
        release_digest=args.release_digest,
        port_bindings_digest=args.port_bindings_digest,
        resource_ceiling_digest=args.resource_ceiling_digest,
        transport_protocol=args.transport_protocol,
        output_path=args.output_path,
    )
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
