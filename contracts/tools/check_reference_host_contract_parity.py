#!/usr/bin/env python3
"""Fail-closed parity gate for the published reference-host contract cohort.

The gate is deliberately offline and stdlib-only.  It compares the canonical
host request/response/private-transport schemas and vectors with the copies
published by the reference host, checks every manifest and owner-descriptor
digest, validates the vectors against the canonical schemas, and checks the
reference-host source for the contract's transport boundary constants.

It does not build or start the host, read credentials, inspect the network, or
assume a Unix kernel.  A missing, malformed, stale, or ambiguous input is a
failure; no result is treated as an implicit pass.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


HOST_CONTRACTS = (
    ("apxm.runtime.host-request.v1", "apxm.runtime.host-request.v1.json"),
    ("apxm.runtime.host-response.v1", "apxm.runtime.host-response.v1.json"),
    ("apxm.runtime.host-transport.v1", "apxm.runtime.host-transport.v1.json"),
)

SOURCE_CONSTANTS = {
    "HOST_SCHEMA": "apxm.runtime.host-response.v1",
    "REQUEST_SCHEMA": "apxm.runtime.host-request.v1",
    "TRANSPORT_PROTOCOL": "jsonl-stdin-stdout",
    "PRIVATE_TRANSPORT_PROTOCOL": "jsonl-unix-stream",
    "OWNER_EXECUTABLE": "apxm-reference-host",
    "RELEASE_MANIFEST_SOURCE_PATH": (
        "contracts/reference-host/manifests/"
        "apxm.reference-host-release-manifest.v1.json"
    ),
}

SOURCE_BOUNDARY_MARKERS = (
    "admission: Option<Admission>",
    "air: Option<Value>",
    "verify_invocation_admission",
    "--unix-socket",
    "peer_cred()",
    "path.is_absolute()",
    "private_transport_peer_is_owner",
    "parent_metadata.mode() & 0o022",
    "fs::set_permissions(socket_path",
    "options.mode(0o600)",
)

EXPECTED_STATUS_VALUES = {
    "readiness",
    "committed",
    "failed",
    "rejected",
    "cancelled",
    "shutdown",
    "revoked",
    "restarted",
}

REFERENCE_HOST_PROFILE_COHORT = ["embedded", "reference-host"]
REFERENCE_HOST_PARITY_VECTOR_SPECS = (
    (
        "apxm.reference-host.invoke-parity.v1",
        "reference-host/vectors/apxm.reference-host.invoke-parity.v1.json",
    ),
    (
        "apxm.reference-host.lifecycle-parity.v1",
        "reference-host/vectors/apxm.reference-host.lifecycle-parity.v1.json",
    ),
)
REFERENCE_HOST_HARNESS_PATH = "crates/tools/cli/tests/reference_host_jsonl.rs"


@dataclass(frozen=True)
class ContractPaths:
    root: Path

    @property
    def contracts(self) -> Path:
        return self.root / "contracts"

    @property
    def schemas(self) -> Path:
        return self.contracts / "schemas"

    @property
    def vectors(self) -> Path:
        return self.contracts / "vectors"

    @property
    def published_schemas(self) -> Path:
        return self.contracts / "reference-host" / "schemas"

    @property
    def published_vectors(self) -> Path:
        return self.contracts / "reference-host" / "vectors"

    @property
    def execution_manifest(self) -> Path:
        return (
            self.contracts
            / "reference-host"
            / "manifests"
            / "apxm.reference-host-execution-manifest.v1.json"
        )

    @property
    def release_manifest(self) -> Path:
        return (
            self.contracts
            / "reference-host"
            / "manifests"
            / "apxm.reference-host-release-manifest.v1.json"
        )

    @property
    def descriptor(self) -> Path:
        return (
            self.contracts
            / "descriptors"
            / "apxm.agents-owner-descriptor.v1.json"
        )

    @property
    def descriptor_sidecar(self) -> Path:
        return self.descriptor.with_suffix(".sha256")

    @property
    def reference_host_source(self) -> Path:
        return self.root / "crates" / "tools" / "cli" / "src" / "bin" / "reference_host.rs"


class GateError(Exception):
    """Raised for an input that prevents a trustworthy parity decision."""


def digest_bytes(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def load_json(path: Path, label: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise GateError(f"{label}: missing file {path}") from error
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise GateError(f"{label}: cannot load valid UTF-8 JSON from {path}: {error}") from error


def read_bytes(path: Path, label: str) -> bytes:
    try:
        return path.read_bytes()
    except (FileNotFoundError, OSError) as error:
        raise GateError(f"{label}: cannot read {path}: {error}") from error


def fail(message: str) -> None:
    raise GateError(message)


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def repo_relative(path: Path, root: Path) -> str:
    try:
        return path.resolve(strict=False).relative_to(root.resolve(strict=False)).as_posix()
    except ValueError as error:
        raise GateError(f"path escapes repository root: {path}") from error


def resolve_contract_path(paths: ContractPaths, relative: str, label: str) -> Path:
    candidate = (paths.contracts / relative).resolve(strict=False)
    try:
        candidate.relative_to(paths.contracts.resolve(strict=False))
    except ValueError as error:
        raise GateError(f"{label}: path escapes contracts/: {relative}") from error
    require(candidate.is_file(), f"{label}: referenced file is missing: {relative}")
    return candidate


def resolve_repo_path(paths: ContractPaths, relative: str, label: str) -> Path:
    candidate = (paths.root / relative).resolve(strict=False)
    try:
        candidate.relative_to(paths.root.resolve(strict=False))
    except ValueError as error:
        raise GateError(f"{label}: path escapes repository root: {relative}") from error
    require(candidate.is_file(), f"{label}: referenced file is missing: {relative}")
    return candidate


def require_digest(path: Path, recorded: Any, label: str) -> None:
    require(isinstance(recorded, str), f"{label}: digest is missing or not a string")
    require(
        re.fullmatch(r"sha256:[0-9a-f]{64}", recorded) is not None,
        f"{label}: digest is not a lowercase sha256 digest: {recorded!r}",
    )
    actual = digest_bytes(read_bytes(path, label))
    require(recorded == actual, f"{label}: digest drifted; recorded {recorded}, actual {actual}")


def mapping_entry(entries: Any, key: str, value: str, label: str) -> dict[str, Any]:
    require(isinstance(entries, list), f"{label}: expected a list")
    matches = [entry for entry in entries if isinstance(entry, dict) and entry.get(key) == value]
    require(len(matches) == 1, f"{label}: expected exactly one {key}={value!r}")
    return matches[0]


def check_copy_and_shape(paths: ContractPaths) -> dict[str, dict[str, Any]]:
    schemas: dict[str, dict[str, Any]] = {}
    for schema_id, filename in HOST_CONTRACTS:
        canonical_schema_path = paths.schemas / filename
        published_schema_path = paths.published_schemas / filename
        canonical_vector_path = paths.vectors / filename
        published_vector_path = paths.published_vectors / filename

        canonical_schema_bytes = read_bytes(canonical_schema_path, f"canonical schema {schema_id}")
        published_schema_bytes = read_bytes(published_schema_path, f"published schema {schema_id}")
        require(
            canonical_schema_bytes == published_schema_bytes,
            f"{schema_id}: canonical and reference-host schema bytes drifted",
        )
        canonical_vector_bytes = read_bytes(canonical_vector_path, f"canonical vector {schema_id}")
        published_vector_bytes = read_bytes(published_vector_path, f"published vector {schema_id}")
        require(
            canonical_vector_bytes == published_vector_bytes,
            f"{schema_id}: canonical and reference-host vector bytes drifted",
        )

        schema = load_json(canonical_schema_path, f"canonical schema {schema_id}")
        require(isinstance(schema, dict), f"{schema_id}: schema must be a JSON object")
        require(schema.get("$id") == schema_id, f"{schema_id}: schema $id drifted")
        vector = load_json(canonical_vector_path, f"canonical vector {schema_id}")
        require(isinstance(vector, list) and vector, f"{schema_id}: vector must be a non-empty list")
        names: set[str] = set()
        for index, case in enumerate(vector):
            label = f"{schema_id} vector case {index}"
            require(isinstance(case, dict), f"{label}: case must be an object")
            name = case.get("name")
            require(isinstance(name, str) and name, f"{label}: case name is required")
            require(name not in names, f"{label}: duplicate case name {name!r}")
            names.add(name)
            require(isinstance(case.get("expected_valid"), bool), f"{label}: expected_valid is required")
            require(isinstance(case.get("input"), dict), f"{label}: input must be an object")
        schemas[schema_id] = schema
    return schemas


def pointer_value(document: Any, pointer: str) -> Any:
    value = document
    for component in pointer.removeprefix("#/").split("/"):
        component = component.replace("~1", "/").replace("~0", "~")
        if isinstance(value, dict) and component in value:
            value = value[component]
        else:
            raise GateError(f"JSON Schema pointer does not resolve: {pointer}")
    return value


class SchemaSubsetValidator:
    """Small offline validator for the JSON Schema features used by these vectors."""

    def __init__(self, schemas: dict[str, dict[str, Any]]) -> None:
        self.schemas = schemas

    def validate(self, instance: Any, schema: Any, *, root: dict[str, Any], path: str) -> list[str]:
        if not isinstance(schema, dict):
            return [f"{path}: schema fragment is not an object"]
        if "$ref" in schema:
            reference = schema["$ref"]
            if not isinstance(reference, str):
                return [f"{path}: $ref is not a string"]
            if reference.startswith("#"):
                target = pointer_value(root, reference)
                target_root = root
            else:
                schema_id, separator, fragment = reference.partition("#")
                target_root = self.schemas.get(schema_id)
                if target_root is None:
                    return [f"{path}: unresolved external $ref {schema_id!r}"]
                target = pointer_value(target_root, f"#{fragment}") if separator else target_root
            return self.validate(instance, target, root=target_root, path=path)

        errors: list[str] = []
        if "oneOf" in schema:
            branches = schema["oneOf"]
            require(isinstance(branches, list), f"{path}: oneOf must be a list")
            valid_count = sum(
                not self.validate(instance, branch, root=root, path=path)
                for branch in branches
            )
            if valid_count != 1:
                errors.append(f"{path}: oneOf matched {valid_count} branches, expected exactly one")
            return errors
        if "anyOf" in schema:
            branches = schema["anyOf"]
            require(isinstance(branches, list), f"{path}: anyOf must be a list")
            if not any(not self.validate(instance, branch, root=root, path=path) for branch in branches):
                errors.append(f"{path}: anyOf matched no branches")
            return errors
        if "allOf" in schema:
            for branch in schema["allOf"]:
                errors.extend(self.validate(instance, branch, root=root, path=path))

        if "const" in schema and instance != schema["const"]:
            errors.append(f"{path}: expected const {schema['const']!r}")
        if "enum" in schema and instance not in schema["enum"]:
            errors.append(f"{path}: value is outside enum")

        expected_type = schema.get("type")
        if expected_type is not None:
            type_ok = {
                "object": isinstance(instance, dict),
                "array": isinstance(instance, list),
                "string": isinstance(instance, str),
                "integer": isinstance(instance, int) and not isinstance(instance, bool),
                "number": isinstance(instance, (int, float)) and not isinstance(instance, bool),
                "boolean": isinstance(instance, bool),
                "null": instance is None,
            }.get(expected_type)
            if type_ok is None:
                errors.append(f"{path}: unsupported schema type {expected_type!r}")
            elif not type_ok:
                return errors + [f"{path}: expected type {expected_type}"]

        if isinstance(instance, str):
            if "minLength" in schema and len(instance) < schema["minLength"]:
                errors.append(f"{path}: string is shorter than minLength")
            if "pattern" in schema and re.search(schema["pattern"], instance) is None:
                errors.append(f"{path}: string does not match pattern")
        if isinstance(instance, (int, float)) and "minimum" in schema and instance < schema["minimum"]:
            errors.append(f"{path}: number is below minimum")
        if isinstance(instance, dict):
            for required in schema.get("required", []):
                if required not in instance:
                    errors.append(f"{path}: missing required property {required!r}")
            properties = schema.get("properties", {})
            if not isinstance(properties, dict):
                errors.append(f"{path}: properties must be an object")
            else:
                for key, value in instance.items():
                    if key in properties:
                        errors.extend(self.validate(value, properties[key], root=root, path=f"{path}.{key}"))
                    elif schema.get("additionalProperties") is False:
                        errors.append(f"{path}: unknown property {key!r}")
                    elif isinstance(schema.get("additionalProperties"), dict):
                        errors.extend(
                            self.validate(value, schema["additionalProperties"], root=root, path=f"{path}.{key}")
                        )
        if isinstance(instance, list) and isinstance(schema.get("items"), dict):
            for index, value in enumerate(instance):
                errors.extend(self.validate(value, schema["items"], root=root, path=f"{path}[{index}]"))
        return errors


def load_all_schemas(paths: ContractPaths) -> dict[str, dict[str, Any]]:
    schemas: dict[str, dict[str, Any]] = {}
    for path in sorted(paths.schemas.glob("*.json")):
        schema = load_json(path, f"schema {path.name}")
        require(isinstance(schema, dict) and isinstance(schema.get("$id"), str), f"schema {path.name}: missing $id")
        schema_id = schema["$id"]
        require(schema_id not in schemas, f"duplicate canonical schema $id {schema_id!r}")
        schemas[schema_id] = schema
    return schemas


def check_target_vectors(paths: ContractPaths, schemas: dict[str, dict[str, Any]]) -> None:
    validator = SchemaSubsetValidator(schemas)
    for schema_id, filename in HOST_CONTRACTS:
        vector = load_json(paths.vectors / filename, f"vector {schema_id}")
        schema = schemas.get(schema_id)
        require(schema is not None, f"{schema_id}: canonical schema is not indexed")
        for index, case in enumerate(vector):
            errors = validator.validate(case["input"], schema, root=schema, path=f"{schema_id}[{index}]")
            if case["expected_valid"] and errors:
                fail(f"{schema_id} vector case {case['name']!r}: expected valid but failed: {errors[0]}")
            if not case["expected_valid"] and not errors:
                fail(f"{schema_id} vector case {case['name']!r}: expected invalid but validated")


def check_manifest_ref(
    paths: ContractPaths,
    entry: Any,
    *,
    schema_id: str,
    published_relative: str,
    canonical_relative: str | None,
    label: str,
) -> None:
    require(isinstance(entry, dict), f"{label}: manifest entry must be an object")
    require(entry.get("schema_id") == schema_id, f"{label}: schema_id drifted")
    require(entry.get("path") == published_relative, f"{label}: published path drifted")
    published = resolve_contract_path(paths, published_relative, label)
    require_digest(published, entry.get("digest"), label)
    if canonical_relative is not None:
        require(
            entry.get("canonical_source_path") == canonical_relative,
            f"{label}: canonical_source_path drifted",
        )
        canonical = resolve_contract_path(paths, canonical_relative, label)
        require(
            read_bytes(published, label) == read_bytes(canonical, label),
            f"{label}: canonical source and published bytes drifted",
        )


def check_manifests(paths: ContractPaths) -> None:
    execution = load_json(paths.execution_manifest, "reference-host execution manifest")
    release = load_json(paths.release_manifest, "reference-host release manifest")
    require(execution.get("schema_version") == "apxm.reference-host-execution-manifest.v1", "execution manifest schema_version drifted")
    require(release.get("schema_version") == "apxm.reference-host-release-manifest.v1", "release manifest schema_version drifted")
    for label, manifest in (("execution manifest", execution), ("release manifest", release)):
        require(manifest.get("semantic_owner") == "agents", f"{label}: semantic_owner drifted")
        require(manifest.get("owner_executable") == "apxm-reference-host", f"{label}: owner_executable drifted")
        require(manifest.get("owner_executable_path") == "crates/tools/cli/src/bin/reference_host.rs", f"{label}: owner_executable_path drifted")
    require(execution.get("transport_protocol") == "jsonl-stdin-stdout", "execution manifest: transport_protocol drifted")
    require(release.get("transport_protocol") == "jsonl-stdin-stdout", "release manifest: transport_protocol drifted")

    for field, schema_id, filename in (
        ("host_request_schema", "apxm.runtime.host-request.v1", "apxm.runtime.host-request.v1.json"),
        ("host_response_schema", "apxm.runtime.host-response.v1", "apxm.runtime.host-response.v1.json"),
        ("host_transport_schema", "apxm.runtime.host-transport.v1", "apxm.runtime.host-transport.v1.json"),
    ):
        check_manifest_ref(
            paths,
            execution.get(field),
            schema_id=schema_id,
            published_relative=f"reference-host/schemas/{filename}",
            canonical_relative=f"schemas/{filename}",
            label=f"execution manifest {field}",
        )
        check_manifest_ref(
            paths,
            mapping_entry(release.get("publication_cohort", {}).get("schemas"), "schema_id", schema_id, "release publication schemas"),
            schema_id=schema_id,
            published_relative=f"reference-host/schemas/{filename}",
            canonical_relative=None,
            label=f"release manifest publication schema {schema_id}",
        )
    for field, schema_id, filename in (
        ("host_request_vector", "apxm.runtime.host-request.v1", "apxm.runtime.host-request.v1.json"),
        ("host_response_vector", "apxm.runtime.host-response.v1", "apxm.runtime.host-response.v1.json"),
        ("host_transport_vector", "apxm.runtime.host-transport.v1", "apxm.runtime.host-transport.v1.json"),
    ):
        check_manifest_ref(
            paths,
            execution.get(field),
            schema_id=schema_id,
            published_relative=f"reference-host/vectors/{filename}",
            canonical_relative=f"vectors/{filename}",
            label=f"execution manifest {field}",
        )
        check_manifest_ref(
            paths,
            mapping_entry(release.get("publication_cohort", {}).get("vectors"), "schema_id", schema_id, "release publication vectors"),
            schema_id=schema_id,
            published_relative=f"reference-host/vectors/{filename}",
            canonical_relative=None,
            label=f"release manifest publication vector {schema_id}",
        )

    execution_ref = mapping_entry(
        release.get("publication_cohort", {}).get("manifests"),
        "schema_version",
        "apxm.reference-host-execution-manifest.v1",
        "release publication manifests",
    )
    require_digest(paths.execution_manifest, execution_ref.get("digest"), "release publication execution manifest")
    require(
        release.get("lifecycle_cohort_attestation", {}).get("execution_manifest", {}).get("digest")
        == digest_bytes(read_bytes(paths.execution_manifest, "execution manifest")),
        "release lifecycle attestation: execution manifest digest drifted",
    )
    constraints = release.get("constraints")
    require(isinstance(constraints, dict), "release manifest: constraints are missing")
    require(constraints.get("http_surface") == "absent", "release manifest: HTTP surface must remain absent")
    require(constraints.get("clic_dependency") == "absent", "release manifest: CLIC dependency must remain absent")
    check_release_evidence(paths, release)


def check_release_evidence(paths: ContractPaths, release: dict[str, Any]) -> None:
    parity_refs: list[dict[str, Any]] = []
    parity_case_names: list[str] = []
    for vector_id, relative in REFERENCE_HOST_PARITY_VECTOR_SPECS:
        vector_path = resolve_contract_path(paths, relative, f"release parity vector {vector_id}")
        vector = load_json(vector_path, f"release parity vector {vector_id}")
        require(isinstance(vector, dict), f"{vector_id}: vector must be an object")
        require(vector.get("schema_version") == vector_id, f"{vector_id}: schema_version drifted")
        require(vector.get("semantic_owner") == "agents", f"{vector_id}: semantic_owner drifted")
        require(vector.get("profiles") == REFERENCE_HOST_PROFILE_COHORT, f"{vector_id}: profiles drifted")
        cases = vector.get("cases")
        require(isinstance(cases, list) and cases, f"{vector_id}: cases are missing")
        names: list[str] = []
        for index, case in enumerate(cases):
            require(isinstance(case, dict), f"{vector_id} case {index}: case must be an object")
            name = case.get("name")
            require(isinstance(name, str) and name, f"{vector_id} case {index}: name is missing")
            require(name not in names, f"{vector_id}: duplicate case name {name!r}")
            names.append(name)
        parity_case_names.extend(names)
        parity_refs.append(
            {
                "vector_id": vector_id,
                "path": relative,
                "digest": digest_bytes(read_bytes(vector_path, f"release parity vector {vector_id}")),
                "profiles": list(REFERENCE_HOST_PROFILE_COHORT),
            }
        )

    require(
        release.get("golden_vectors") == parity_refs,
        "release manifest golden_vectors drifted from the checked-in parity vectors",
    )
    execution_ref = {
        "schema_version": "apxm.reference-host-execution-manifest.v1",
        "path": "reference-host/manifests/apxm.reference-host-execution-manifest.v1.json",
        "digest": digest_bytes(read_bytes(paths.execution_manifest, "execution manifest")),
    }
    lifecycle_vector = load_json(
        resolve_contract_path(
            paths,
            REFERENCE_HOST_PARITY_VECTOR_SPECS[1][1],
            "reference-host lifecycle parity vector",
        ),
        "reference-host lifecycle parity vector",
    )
    require(isinstance(lifecycle_vector, dict), "reference-host lifecycle parity vector: must be an object")
    require(
        isinstance(lifecycle_vector.get("shared_contracts"), dict),
        "reference-host lifecycle parity vector: shared_contracts are missing",
    )
    expected_attestation = {
        "profile_cohort": list(REFERENCE_HOST_PROFILE_COHORT),
        "execution_manifest": execution_ref,
        "lifecycle_vector": parity_refs[1],
        "shared_contracts": lifecycle_vector["shared_contracts"],
        "required_cases": [case["name"] for case in lifecycle_vector["cases"]],
    }
    require(
        release.get("lifecycle_cohort_attestation") == expected_attestation,
        "release manifest lifecycle_cohort_attestation drifted",
    )

    harness_path = resolve_repo_path(paths, REFERENCE_HOST_HARNESS_PATH, "reference-host parity harness")
    expected_evidence = {
        "owner_executable": {
            "name": "apxm-reference-host",
            "path": "crates/tools/cli/src/bin/reference_host.rs",
            "transport_protocol": "jsonl-stdin-stdout",
        },
        "harness": {
            "kind": "rust-integration-test",
            "path": REFERENCE_HOST_HARNESS_PATH,
            "digest": digest_bytes(read_bytes(harness_path, "reference-host parity harness")),
        },
        "vectors": parity_refs,
        "live_required_cases": parity_case_names,
        "unavailable_required_cases": [],
    }
    require(
        release.get("executable_parity_evidence") == expected_evidence,
        "release manifest executable_parity_evidence drifted",
    )


def check_descriptor(paths: ContractPaths) -> None:
    descriptor = load_json(paths.descriptor, "agents owner descriptor")
    for schema_id, filename in HOST_CONTRACTS:
        entry = mapping_entry(descriptor.get("owned_schemas"), "schema_id", schema_id, "descriptor owned schemas")
        require(entry.get("path") == f"schemas/{filename}", f"descriptor {schema_id}: path drifted")
        require_digest(paths.schemas / filename, entry.get("digest"), f"descriptor {schema_id}")
        vector = mapping_entry(descriptor.get("conformance_vectors"), "schema_id", schema_id, "descriptor vectors")
        require(vector.get("path") == f"vectors/{filename}", f"descriptor {schema_id} vector: path drifted")
        require_digest(paths.vectors / filename, vector.get("digest"), f"descriptor {schema_id} vector")

    require(paths.descriptor_sidecar.is_file(), "owner descriptor exact-checksum sidecar is missing")
    sidecar_lines = paths.descriptor_sidecar.read_text(encoding="utf-8").splitlines()
    require(len(sidecar_lines) == 2, "owner descriptor exact-checksum sidecar shape drifted")
    recorded_digest, separator, recorded_path = sidecar_lines[0].partition("  ")
    require(separator and recorded_path == repo_relative(paths.descriptor, paths.root), "owner descriptor sidecar path drifted")
    require(recorded_digest == digest_bytes(read_bytes(paths.descriptor, "owner descriptor")), "owner descriptor sidecar digest drifted")


def rust_constant(source: str, name: str) -> str:
    match = re.search(rf"\bconst\s+{re.escape(name)}\s*:\s*&str\s*=\s*\"([^\"]+)\"", source)
    require(match is not None, f"reference host source: missing string constant {name}")
    return match.group(1)


def check_reference_host_source(paths: ContractPaths, schemas: dict[str, dict[str, Any]]) -> None:
    source = read_bytes(paths.reference_host_source, "reference host source").decode("utf-8")
    for name, expected in SOURCE_CONSTANTS.items():
        require(rust_constant(source, name) == expected, f"reference host source: {name} drifted")
    for marker in SOURCE_BOUNDARY_MARKERS:
        require(marker in source, f"reference host source: private/public boundary marker missing: {marker}")
    require("CLIC" not in source and "clic" not in source, "reference host source must not depend on CLIC")

    response_schema = schemas["apxm.runtime.host-response.v1"]
    statuses = set(response_schema.get("properties", {}).get("status", {}).get("enum", []))
    require(EXPECTED_STATUS_VALUES <= statuses, "host-response schema is missing a status emitted by reference host")

    transport_schema = schemas["apxm.runtime.host-transport.v1"]
    properties = transport_schema.get("properties", {})
    require(properties.get("semantic_owner", {}).get("const") == "agents", "private transport semantic owner drifted")
    require(properties.get("protocol", {}).get("const") == SOURCE_CONSTANTS["PRIVATE_TRANSPORT_PROTOCOL"], "private transport protocol drifted")
    endpoint = properties.get("endpoint", {}).get("properties", {})
    require(endpoint.get("public", {}).get("const") is False, "private transport endpoint became public")
    require(endpoint.get("path_source", {}).get("const") == "injected-runtime-endpoint", "private transport endpoint path is not injected")
    require(properties.get("authority", {}).get("const") == "caller-supplied-apxm-invocation-admission", "private transport authority source drifted")


def run_gate(root: Path) -> None:
    paths = ContractPaths(root.resolve(strict=False))
    schemas = check_copy_and_shape(paths)
    all_schemas = load_all_schemas(paths)
    all_schemas.update(schemas)
    check_target_vectors(paths, all_schemas)
    check_manifests(paths)
    check_descriptor(paths)
    check_reference_host_source(paths, all_schemas)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="agents checkout root (default: inferred from this script)",
    )
    args = parser.parse_args(argv)
    try:
        run_gate(args.root)
    except (GateError, OSError, UnicodeError, json.JSONDecodeError) as error:
        print(f"FAIL-CLOSED: reference-host contract parity rejected: {error}", file=sys.stderr)
        return 1
    print("PASS: reference-host contract parity")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
