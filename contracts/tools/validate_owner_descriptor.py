#!/usr/bin/env python3
"""Validate and content-address the agents owner descriptor.

The descriptor publishes the closed Agent Program semantic schemas, state
machines, typed errors, source maps, the atomic execution-commit port contract,
and their conformance vectors against the already-published contract
constitution layer. This tool references the constitution envelopes by `$id`
and file digest; it never copies or redefines them.

Modes:
  --check          verify every recorded digest, validate every vector against
                   its schema, enforce abstraction/atomicity/evidence rules and
                   the plan-free authoring rule (default; used as the gate).
  --write-digests  recompute and write content-addressed digests into the
                   descriptor and owned port contract instances.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any


def git_common_repo_root(root: Path) -> Path:
    resolved = root.resolve(strict=False)
    result = subprocess.run(
        [
            "git",
            "-C",
            str(resolved),
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0 or not result.stdout.strip():
        return resolved
    common_dir = Path(result.stdout.strip()).resolve(strict=False)
    if common_dir.name != ".git":
        return resolved
    return common_dir.parent


SCRIPT_DIR = Path(__file__).resolve().parent
CONTRACTS_DIR = SCRIPT_DIR.parent
CHECKOUT_ROOT = CONTRACTS_DIR.parent.resolve(strict=False)
AGENTS_ROOT = git_common_repo_root(CHECKOUT_ROOT)
WORKSPACE_DIR = AGENTS_ROOT.parent
CONSTITUTION_SCHEMAS_DIR = WORKSPACE_DIR / "contracts" / "schemas"

SCHEMAS_DIR = CONTRACTS_DIR / "schemas"
VECTORS_DIR = CONTRACTS_DIR / "vectors"
PORT_CONTRACTS_DIR = CONTRACTS_DIR / "port-contracts"
DESCRIPTOR_PATH = CONTRACTS_DIR / "descriptors" / "apxm.agents-owner-descriptor.v1.json"
DESCRIPTOR_SIDECAR_PATH = DESCRIPTOR_PATH.with_suffix(".sha256")
REFERENCE_HOST_RELEASE_MANIFEST_PATH = (
    CONTRACTS_DIR / "reference-host" / "manifests" / "apxm.reference-host-release-manifest.v1.json"
)
REFERENCE_HOST_EXECUTION_MANIFEST_PATH = (
    CONTRACTS_DIR / "reference-host" / "manifests" / "apxm.reference-host-execution-manifest.v1.json"
)
REFERENCE_HOST_INVOKE_PARITY_VECTOR_PATH = (
    CONTRACTS_DIR / "reference-host" / "vectors" / "apxm.reference-host.invoke-parity.v1.json"
)
REFERENCE_HOST_LIFECYCLE_PARITY_VECTOR_PATH = (
    CONTRACTS_DIR / "reference-host" / "vectors" / "apxm.reference-host.lifecycle-parity.v1.json"
)
WORKSPACE_MANIFEST = AGENTS_ROOT / "Cargo.toml"
EXECUTION_COMMIT_PORT_CONTRACT_PATH = (
    PORT_CONTRACTS_DIR / "apxm.execution-commit.port-contract.v1.json"
)
CAPABILITY_PORT_CONTRACT_PATH = (
    PORT_CONTRACTS_DIR / "apxm.capability-invocation.port-contract.v1.json"
)

EXECUTION_COMMIT_ENVELOPE = CONSTITUTION_SCHEMAS_DIR / "execution-commit.v1.json"
EXECUTION_COMMIT_VECTORS = VECTORS_DIR / "apxm.execution-commit.v1.json"
CAPABILITY_INVOCATION_SCHEMA = SCHEMAS_DIR / "apxm.capability-invocation.v1.json"
CAPABILITY_OUTCOME_SCHEMA = SCHEMAS_DIR / "apxm.capability-outcome.v1.json"
CAPABILITY_INVOCATION_VECTORS = VECTORS_DIR / "apxm.capability-invocation.v1.json"
CAPABILITY_OUTCOME_VECTORS = VECTORS_DIR / "apxm.capability-outcome.v1.json"
MODEL_INFERENCE_PORT_CONTRACT_PATH = (
    PORT_CONTRACTS_DIR / "apxm.model-inference.port-contract.v1.json"
)
MODEL_INFERENCE_REQUEST_SCHEMA = SCHEMAS_DIR / "apxm.model-inference-request.v1.json"
MODEL_INFERENCE_OUTCOME_SCHEMA = SCHEMAS_DIR / "apxm.model-inference-outcome.v1.json"
MODEL_INFERENCE_REQUEST_VECTORS = VECTORS_DIR / "apxm.model-inference-request.v1.json"
MODEL_INFERENCE_OUTCOME_VECTORS = VECTORS_DIR / "apxm.model-inference-outcome.v1.json"
EXTERNAL_AGENT_PORT_CONTRACT_PATH = (
    PORT_CONTRACTS_DIR / "apxm.external-agent.port-contract.v1.json"
)
EXTERNAL_AGENT_REQUEST_SCHEMA = SCHEMAS_DIR / "apxm.external-agent-request.v1.json"
EXTERNAL_AGENT_OUTCOME_SCHEMA = SCHEMAS_DIR / "apxm.external-agent-outcome.v1.json"
EXTERNAL_AGENT_REQUEST_VECTORS = VECTORS_DIR / "apxm.external-agent-request.v1.json"
EXTERNAL_AGENT_OUTCOME_VECTORS = VECTORS_DIR / "apxm.external-agent-outcome.v1.json"
DURABLE_EVENT_PORT_CONTRACT_PATH = (
    PORT_CONTRACTS_DIR / "apxm.durable-event.port-contract.v1.json"
)
DURABLE_EVENT_REQUEST_SCHEMA = SCHEMAS_DIR / "apxm.durable-event-request.v1.json"
DURABLE_EVENT_OUTCOME_SCHEMA = SCHEMAS_DIR / "apxm.durable-event-outcome.v1.json"
DURABLE_EVENT_REQUEST_VECTORS = VECTORS_DIR / "apxm.durable-event-request.v1.json"
DURABLE_EVENT_OUTCOME_VECTORS = VECTORS_DIR / "apxm.durable-event-outcome.v1.json"
PROGRAM_COMPOSITION_PORT_CONTRACT_PATH = (
    PORT_CONTRACTS_DIR / "apxm.program-composition.port-contract.v1.json"
)
PROGRAM_COMPOSITION_REQUEST_SCHEMA = SCHEMAS_DIR / "apxm.program-composition-request.v1.json"
PROGRAM_COMPOSITION_OUTCOME_SCHEMA = SCHEMAS_DIR / "apxm.program-composition-outcome.v1.json"
PROGRAM_COMPOSITION_REQUEST_VECTORS = VECTORS_DIR / "apxm.program-composition-request.v1.json"
PROGRAM_COMPOSITION_OUTCOME_VECTORS = VECTORS_DIR / "apxm.program-composition-outcome.v1.json"

PORT_CONTRACT_SPECS = (
    (
        EXECUTION_COMMIT_PORT_CONTRACT_PATH,
        "port_boundary",
        EXECUTION_COMMIT_ENVELOPE,
        EXECUTION_COMMIT_ENVELOPE,
        EXECUTION_COMMIT_ENVELOPE,
        (EXECUTION_COMMIT_VECTORS,),
    ),
    (
        CAPABILITY_PORT_CONTRACT_PATH,
        "capability_port_boundary",
        CAPABILITY_INVOCATION_SCHEMA,
        CAPABILITY_OUTCOME_SCHEMA,
        CAPABILITY_OUTCOME_SCHEMA,
        (CAPABILITY_INVOCATION_VECTORS, CAPABILITY_OUTCOME_VECTORS),
    ),
    (
        MODEL_INFERENCE_PORT_CONTRACT_PATH,
        "model_inference_port_boundary",
        MODEL_INFERENCE_REQUEST_SCHEMA,
        MODEL_INFERENCE_OUTCOME_SCHEMA,
        MODEL_INFERENCE_OUTCOME_SCHEMA,
        (MODEL_INFERENCE_REQUEST_VECTORS, MODEL_INFERENCE_OUTCOME_VECTORS),
    ),
    (
        EXTERNAL_AGENT_PORT_CONTRACT_PATH,
        "external_agent_port_boundary",
        EXTERNAL_AGENT_REQUEST_SCHEMA,
        EXTERNAL_AGENT_OUTCOME_SCHEMA,
        EXTERNAL_AGENT_OUTCOME_SCHEMA,
        (EXTERNAL_AGENT_REQUEST_VECTORS, EXTERNAL_AGENT_OUTCOME_VECTORS),
    ),
    (
        DURABLE_EVENT_PORT_CONTRACT_PATH,
        "durable_event_port_boundary",
        DURABLE_EVENT_REQUEST_SCHEMA,
        DURABLE_EVENT_OUTCOME_SCHEMA,
        DURABLE_EVENT_OUTCOME_SCHEMA,
        (DURABLE_EVENT_REQUEST_VECTORS, DURABLE_EVENT_OUTCOME_VECTORS),
    ),
    (
        PROGRAM_COMPOSITION_PORT_CONTRACT_PATH,
        "program_composition_port_boundary",
        PROGRAM_COMPOSITION_REQUEST_SCHEMA,
        PROGRAM_COMPOSITION_OUTCOME_SCHEMA,
        PROGRAM_COMPOSITION_OUTCOME_SCHEMA,
        (PROGRAM_COMPOSITION_REQUEST_VECTORS, PROGRAM_COMPOSITION_OUTCOME_VECTORS),
    ),
)

# Vector file -> schema `$id` it is validated against. Constitution-owned
# envelopes are resolved from the published constitution layer.
VECTOR_SCHEMA = {
    "apxm.frontend-graph.v1.json": "apxm.frontend-graph.v1",
    "apxm.air.v1.json": "apxm.air.v1",
    "apxm.executable-artifact.v1.json": "apxm.executable-artifact.v1",
    "apxm.runtime-evidence.v1.json": "apxm.runtime-evidence.v1",
    "apxm.committed-native-model-usage.v1.json": "apxm.committed-native-model-usage.v1",
    "apxm.source-map.v1.json": "apxm.source-map.v1",
    "apxm.execution-commit.v1.json": "apxm.execution-commit.v1",
    "apxm.external-agent-session.v1.json": "apxm.external-agent-session.v1",
    "apxm.external-agent-evidence.v1.json": "apxm.external-agent-evidence.v1",
    "apxm.port-contract.v1.json": "apxm.port-contract.v1",
    "apxm.model-context-envelope.v1.json": "apxm.model-context-envelope.v1",
    "apxm.model-target.v1.json": "apxm.model-target.v1",
    "apxm.model-binding.v1.json": "apxm.model-binding.v1",
    "apxm.handler-manifest.v1.json": "apxm.handler-manifest.v1",
    "apxm.capability-invocation.v1.json": "apxm.capability-invocation.v1",
    "apxm.capability-outcome.v1.json": "apxm.capability-outcome.v1",
    "apxm.model-inference-request.v1.json": "apxm.model-inference-request.v1",
    "apxm.model-inference-outcome.v1.json": "apxm.model-inference-outcome.v1",
    "apxm.external-agent-request.v1.json": "apxm.external-agent-request.v1",
    "apxm.external-agent-outcome.v1.json": "apxm.external-agent-outcome.v1",
    "apxm.durable-event-request.v1.json": "apxm.durable-event-request.v1",
    "apxm.durable-event-outcome.v1.json": "apxm.durable-event-outcome.v1",
    "apxm.program-composition-request.v1.json": "apxm.program-composition-request.v1",
    "apxm.program-composition-outcome.v1.json": "apxm.program-composition-outcome.v1",
    "apxm.runtime-readiness.v1.json": "apxm.runtime-readiness.v1",
    "apxm.runtime-drain-quiescence.v1.json": "apxm.runtime-drain-quiescence.v1",
    "apxm.invocation-admission.v1.json": "apxm.invocation-admission.v1",
}

# Authoring rule: no product surface may cite the delivery plan or its
# vocabulary. Contract `$id`s and `schema_version` consts are legitimate.
FORBIDDEN_TEXT = [
    r"master[\s_-]?plan",
    r"apxm-v1",
    r"\bwave\s*\d+\b",
    r"\bcampaign\b",
    r"\bADR[\s-]?\d+\b",
    r"\blane\b",
]
FORBIDDEN_LANE_IDS = re.compile(r"\b(?:A|C|S|O|H|T|P|K|D|V|E|M|R)\d[a-z]?\b")

HEX64 = re.compile(r"^[0-9a-f]{64}$")

REFERENCE_HOST_DEPENDENCY_NAME = "apxm-host-sdk"
REFERENCE_HOST_DEPENDENCY_GIT = "https://github.com/apxm-project/host-sdk.git"
REFERENCE_HOST_PROFILE_COHORT = ["embedded", "reference-host"]
REFERENCE_HOST_EXECUTION_MANIFEST_SCHEMA_VERSION = (
    "apxm.reference-host-execution-manifest.v1"
)
REFERENCE_HOST_INVOKE_VECTOR_ID = "apxm.reference-host.invoke-parity.v1"
REFERENCE_HOST_LIFECYCLE_VECTOR_ID = "apxm.reference-host.lifecycle-parity.v1"
RETIRED_REFERENCE_HOST_ADMISSION_ALIAS = "apxm.execution-admission.v1"
REFERENCE_HOST_EXECUTION_SCHEMA_KEYS = (
    "host_execution_manifest_schema",
    "runtime_readiness_schema",
    "runtime_drain_quiescence_schema",
    "invocation_admission_schema",
)
REFERENCE_HOST_EXECUTION_VECTOR_KEYS = (
    "runtime_readiness_vector",
    "runtime_drain_quiescence_vector",
    "invocation_admission_vector",
)
REFERENCE_HOST_DESCRIPTOR = {
    "semantic_owner": "host-sdk",
    "schema_version": "apxm.host-sdk-owner-descriptor.v1",
    "source_revision": "ff48332f2ce6af8a45eb38a15f4510a134223f5f",
    "descriptor_semantic_digest": "sha256:a007bb8daee44cfc5156a358bd4c0f4665adefc0d731738ead9c50a31734e14b",
    "descriptor_exact_checksum": "sha256:d771d3f2c4a50fdeee0c57475c4c00fc6b4121b1e4bf5147a57a76bf11ad7a88",
}
RETIRED_REFERENCE_HOST_OWNERS = frozenset({"coordinator", "clic", "host", "hostsdk"})
RETIRED_REFERENCE_HOST_SCHEMAS = frozenset(
    {
        "apxm.coordinator-owner-descriptor.v1",
        "apxm.clic-owner-descriptor.v1",
        "apxm.host-owner-descriptor.v1",
        "apxm.hostsdk-owner-descriptor.v1",
        "apxm.host_sdk-owner-descriptor.v1",
    }
)

class ValidationError(Exception):
    pass


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def load_toml(path: Path) -> dict[str, Any]:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def content_digest(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical(value).encode("utf-8")).hexdigest()


def file_digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def repo_relative_path(path: Path) -> str:
    return path.relative_to(CHECKOUT_ROOT).as_posix()


def resolve_contract_relative_path(raw_path: str, *, label: str) -> Path:
    candidate = (CONTRACTS_DIR / raw_path).resolve(strict=False)
    contracts_root = CONTRACTS_DIR.resolve(strict=False)
    try:
        candidate.relative_to(contracts_root)
    except ValueError as error:
        raise ValidationError(f"{label} path escapes contracts/: {raw_path}") from error
    if not candidate.is_file():
        raise ValidationError(f"{label} file is missing: {raw_path}")
    return candidate


def descriptor_exact_checksum() -> str:
    if not DESCRIPTOR_SIDECAR_PATH.is_file():
        raise ValidationError("owner descriptor exact-checksum sidecar is missing")
    lines = DESCRIPTOR_SIDECAR_PATH.read_text(encoding="utf-8").splitlines()
    if len(lines) != 2:
        raise ValidationError("owner descriptor exact-checksum sidecar shape drifted")
    recorded_digest, separator, recorded_path = lines[0].partition("  ")
    if separator != "  ":
        raise ValidationError("owner descriptor exact-checksum sidecar must use git checksum spacing")
    if recorded_path != f"contracts/descriptors/{DESCRIPTOR_PATH.name}":
        raise ValidationError("owner descriptor exact-checksum sidecar path drifted")
    if lines[1] != "digest_scope: exact repository bytes":
        raise ValidationError("owner descriptor exact-checksum sidecar scope drifted")
    actual_digest = file_digest(DESCRIPTOR_PATH)
    if recorded_digest != actual_digest:
        raise ValidationError("owner descriptor exact-checksum sidecar is stale")
    return actual_digest


def reference_host_execution_manifest_ref() -> dict[str, str]:
    return {
        "schema_version": REFERENCE_HOST_EXECUTION_MANIFEST_SCHEMA_VERSION,
        "path": "reference-host/manifests/apxm.reference-host-execution-manifest.v1.json",
        "digest": file_digest(REFERENCE_HOST_EXECUTION_MANIFEST_PATH),
    }


def reference_host_publication_cohort(
    execution_manifest: dict[str, Any],
) -> dict[str, list[dict[str, str]]]:
    def publication_entry(field: str, *, key: str) -> dict[str, str]:
        entry = execution_manifest.get(field)
        if not isinstance(entry, dict):
            raise ValidationError(
                f"reference-host execution manifest field {field} must be an object"
            )
        value = entry.get(key)
        path = entry.get("path")
        digest = entry.get("digest")
        if not all(isinstance(item, str) for item in (value, path, digest)):
            raise ValidationError(
                f"reference-host execution manifest field {field} must publish exact string coordinates"
            )
        return {
            key: value,
            "path": path,
            "digest": digest,
        }

    return {
        "manifests": [reference_host_execution_manifest_ref()],
        "schemas": [
            publication_entry(field, key="schema_id")
            for field in REFERENCE_HOST_EXECUTION_SCHEMA_KEYS
        ],
        "vectors": [
            publication_entry(field, key="schema_id")
            for field in REFERENCE_HOST_EXECUTION_VECTOR_KEYS
        ],
    }


def reference_host_lifecycle_vector_ref(lifecycle_vector: dict[str, Any]) -> dict[str, Any]:
    return {
        "vector_id": REFERENCE_HOST_LIFECYCLE_VECTOR_ID,
        "path": "reference-host/vectors/apxm.reference-host.lifecycle-parity.v1.json",
        "digest": file_digest(REFERENCE_HOST_LIFECYCLE_PARITY_VECTOR_PATH),
        "profiles": REFERENCE_HOST_PROFILE_COHORT,
    }


def reference_host_invoke_vector_ref(invoke_vector: dict[str, Any]) -> dict[str, Any]:
    return {
        "vector_id": REFERENCE_HOST_INVOKE_VECTOR_ID,
        "path": "reference-host/vectors/apxm.reference-host.invoke-parity.v1.json",
        "digest": file_digest(REFERENCE_HOST_INVOKE_PARITY_VECTOR_PATH),
        "profiles": REFERENCE_HOST_PROFILE_COHORT,
    }


def attested_release_entry(
    raw_entry: dict[str, Any], *, identifier_field: str, label: str
) -> dict[str, str]:
    identifier = raw_entry.get(identifier_field)
    raw_path = raw_entry.get("path")
    if not isinstance(identifier, str) or not identifier:
        raise ValidationError(f"{label} {identifier_field} is missing")
    if not isinstance(raw_path, str) or not raw_path:
        raise ValidationError(f"{label} path is missing")
    path = resolve_contract_relative_path(raw_path, label=label)
    return {
        identifier_field: identifier,
        "path": repo_relative_path(path),
        "exact_bytes_digest": file_digest(path),
    }


def owner_source_contract_attestations(descriptor: dict[str, Any]) -> list[dict[str, str]]:
    attestations: list[dict[str, str]] = []
    for entry in descriptor.get("owned_schemas", []):
        if not isinstance(entry, dict):
            raise ValidationError("owned_schemas entries must be objects")
        contract_id = entry.get("schema_id")
        raw_path = entry.get("path")
        if not isinstance(contract_id, str) or not isinstance(raw_path, str):
            raise ValidationError("owned_schemas entries must carry schema_id and path")
        path = resolve_contract_relative_path(raw_path, label=f"owned schema {contract_id}")
        attestations.append(
            {
                "kind": "owned-schema",
                "contract_id": contract_id,
                "path": repo_relative_path(path),
                "exact_bytes_digest": file_digest(path),
            }
        )
    for entry in descriptor.get("owned_port_contracts", []):
        if not isinstance(entry, dict):
            raise ValidationError("owned_port_contracts entries must be objects")
        contract_id = entry.get("port_contract_id")
        raw_path = entry.get("path")
        if not isinstance(contract_id, str) or not isinstance(raw_path, str):
            raise ValidationError("owned_port_contracts entries must carry port_contract_id and path")
        path = resolve_contract_relative_path(raw_path, label=f"owned port contract {contract_id}")
        attestations.append(
            {
                "kind": "owned-port-contract",
                "contract_id": contract_id,
                "path": repo_relative_path(path),
                "exact_bytes_digest": file_digest(path),
            }
        )
    return attestations


def reference_host_release_attestation(
    *, owner_revision: str, owner_descriptor_digest: str
) -> dict[str, Any]:
    if not owner_revision:
        raise ValidationError("reference-host release attestation requires owner_revision")
    if not owner_descriptor_digest:
        raise ValidationError("reference-host release attestation requires owner_descriptor_digest")
    descriptor = load_json(DESCRIPTOR_PATH)
    check_reference_host_boundary(descriptor)
    check_reference_host_release_evidence()
    release_manifest = load_json(REFERENCE_HOST_RELEASE_MANIFEST_PATH)
    attestation = {
        "cohort": {
            "revision": owner_revision,
            "descriptor_digest": owner_descriptor_digest,
            "manifest_digest": file_digest(REFERENCE_HOST_RELEASE_MANIFEST_PATH),
        },
        "release_manifest": {
            "path": repo_relative_path(REFERENCE_HOST_RELEASE_MANIFEST_PATH),
            "exact_bytes_digest": file_digest(REFERENCE_HOST_RELEASE_MANIFEST_PATH),
        },
        "golden_vectors": [],
        "publication_cohort": {
            "manifests": [],
            "schemas": [],
            "vectors": [],
        },
        "owner_source_contracts": owner_source_contract_attestations(descriptor),
    }
    golden_vectors = release_manifest.get("golden_vectors")
    if not isinstance(golden_vectors, list):
        raise ValidationError("reference-host release manifest golden_vectors must be an array")
    for entry in golden_vectors:
        if not isinstance(entry, dict):
            raise ValidationError("reference-host release manifest golden_vectors entry must be an object")
        raw_path = entry.get("path")
        if not isinstance(raw_path, str) or not raw_path:
            raise ValidationError("reference-host release manifest golden vector path is missing")
        path = resolve_contract_relative_path(raw_path, label="reference-host golden vector")
        attestation["golden_vectors"].append(
            {
                "path": repo_relative_path(path),
                "exact_bytes_digest": file_digest(path),
            }
        )
    publication_cohort = release_manifest.get("publication_cohort")
    if not isinstance(publication_cohort, dict):
        raise ValidationError("reference-host release manifest publication_cohort must be an object")
    for group, identifier_field in (
        ("manifests", "schema_version"),
        ("schemas", "schema_id"),
        ("vectors", "schema_id"),
    ):
        entries = publication_cohort.get(group)
        if not isinstance(entries, list):
            raise ValidationError(f"reference-host release manifest publication_cohort {group} must be an array")
        attestation["publication_cohort"][group] = [
            attested_release_entry(
                entry,
                identifier_field=identifier_field,
                label=f"reference-host publication cohort {group[:-1]}",
            )
            for entry in entries
            if isinstance(entry, dict)
        ]
        if len(attestation["publication_cohort"][group]) != len(entries):
            raise ValidationError(
                f"reference-host release manifest publication_cohort {group} entries must be objects"
            )
    return attestation


def _matches_json_type(instance: Any, name: str) -> bool:
    if name == "object":
        return isinstance(instance, dict)
    if name == "array":
        return isinstance(instance, list)
    if name == "string":
        return isinstance(instance, str)
    if name == "integer":
        return isinstance(instance, int) and not isinstance(instance, bool)
    if name == "number":
        return isinstance(instance, (int, float)) and not isinstance(instance, bool)
    if name == "boolean":
        return isinstance(instance, bool)
    if name == "null":
        return instance is None
    return False


def resolve_schema_ref(
    ref: object,
    schema_ids: dict[str, dict[str, Any]],
    root_schema: dict[str, Any],
) -> tuple[dict[str, Any], dict[str, Any]] | None:
    if not isinstance(ref, str):
        return None
    if ref.startswith("#/$defs/"):
        target = root_schema.get("$defs", {}).get(ref.removeprefix("#/$defs/"))
        return (target, root_schema) if isinstance(target, dict) else None
    schema_id, marker, pointer = ref.partition("#/$defs/")
    if not marker:
        # Bare `<id>` root reference to a whole published schema.
        external_root = schema_ids.get(ref)
        return (external_root, external_root) if isinstance(external_root, dict) else None
    external_root = schema_ids.get(schema_id)
    if external_root is None:
        return None
    target = external_root.get("$defs", {}).get(pointer)
    return (target, external_root) if isinstance(target, dict) else None


def schema_instance_errors(
    schema: object,
    instance: object,
    schema_ids: dict[str, dict[str, Any]],
    root_schema: dict[str, Any],
) -> list[str]:
    """Evaluate the closed structural JSON Schema subset used by these contracts."""
    if not isinstance(schema, dict):
        return ["schema must be an object"]
    if "$ref" in schema:
        resolved = resolve_schema_ref(schema["$ref"], schema_ids, root_schema)
        if resolved is None:
            return [f"unresolved $ref {schema['$ref']!r}"]
        target, target_root = resolved
        return schema_instance_errors(target, instance, schema_ids, target_root)
    if "oneOf" in schema:
        branches = schema["oneOf"]
        if not isinstance(branches, list):
            return ["oneOf must be an array"]
        matching = [
            branch
            for branch in branches
            if not schema_instance_errors(branch, instance, schema_ids, root_schema)
        ]
        if len(matching) != 1:
            return [f"expected exactly one oneOf branch, matched {len(matching)}"]
        return []

    errors: list[str] = []
    if "const" in schema and instance != schema["const"]:
        return [f"expected const {schema['const']!r}, got {instance!r}"]
    if "enum" in schema and instance not in schema["enum"]:
        errors.append(f"{instance!r} is not one of {schema['enum']}")

    declared_type = schema.get("type")
    if declared_type is not None:
        allowed = declared_type if isinstance(declared_type, list) else [declared_type]
        if not any(_matches_json_type(instance, value) for value in allowed):
            return [f"expected type {allowed}, got {type(instance).__name__}"]

    if isinstance(instance, str):
        if len(instance) < schema.get("minLength", 0):
            errors.append("string is shorter than minLength")
        max_length = schema.get("maxLength")
        if isinstance(max_length, int) and len(instance) > max_length:
            errors.append("string exceeds maxLength")
        pattern = schema.get("pattern")
        if isinstance(pattern, str) and re.fullmatch(pattern, instance) is None:
            errors.append(f"string does not match pattern {pattern!r}")

    if isinstance(instance, (int, float)) and not isinstance(instance, bool):
        minimum = schema.get("minimum")
        if isinstance(minimum, (int, float)) and instance < minimum:
            errors.append(f"{instance!r} is less than minimum {minimum}")

    if isinstance(instance, list):
        min_items = schema.get("minItems")
        if isinstance(min_items, int) and len(instance) < min_items:
            errors.append("array has fewer than minItems")
        if schema.get("uniqueItems") is True:
            canonical_items = [canonical(value) for value in instance]
            if len(set(canonical_items)) != len(canonical_items):
                errors.append("array items must be unique")
        item_schema = schema.get("items")
        if isinstance(item_schema, dict):
            for index, item in enumerate(instance):
                errors.extend(
                    f"[{index}]: {error}"
                    for error in schema_instance_errors(item_schema, item, schema_ids, root_schema)
                )

    if isinstance(instance, dict):
        for key in schema.get("required", []):
            if isinstance(key, str) and key not in instance:
                errors.append(f"{key!r} is required")
        properties = schema.get("properties", {})
        if not isinstance(properties, dict):
            properties = {}
        for key, value in instance.items():
            property_schema = properties.get(key)
            if isinstance(property_schema, dict):
                errors.extend(
                    f"{key}: {error}"
                    for error in schema_instance_errors(property_schema, value, schema_ids, root_schema)
                )
            elif schema.get("additionalProperties") is False:
                errors.append(f"additional property {key!r} is not allowed")

    for branch in schema.get("allOf", []):
        errors.extend(schema_instance_errors(branch, instance, schema_ids, root_schema))
    return errors


def semantic_errors(schema_id: str, instance: object) -> list[str]:
    if not isinstance(instance, dict):
        return []
    if schema_id == "apxm.executable-artifact.v1":
        return artifact_abstraction_errors(instance)
    if schema_id == "apxm.execution-commit.v1":
        return execution_commit_atomicity_errors(instance)
    if schema_id == "apxm.runtime-evidence.v1":
        return runtime_evidence_errors(instance)
    if schema_id == "apxm.committed-native-model-usage.v1":
        expected = file_digest(SCHEMAS_DIR / "apxm.committed-native-model-usage.v1.json")
        if instance.get("source_contract_digest") != expected:
            return ["source_contract_digest must match the exact owning schema bytes"]
        attempt = instance.get("attempt")
        if isinstance(attempt, dict):
            payload = (
                "apxm.committed-native-model-usage.v1\0"
                + str(instance.get("commit_id", ""))
                + "\0"
                + str(attempt.get("fact_id", ""))
            ).encode("utf-8")
            measurement_id = "sha256:" + hashlib.sha256(payload).hexdigest()
            if instance.get("usage_measurement_id") != measurement_id:
                return ["usage_measurement_id must match the exact committed attempt tuple"]
    if schema_id == "apxm.external-agent-evidence.v1":
        return external_agent_evidence_errors(instance)
    if schema_id == "apxm.capability-invocation.v1":
        return capability_invocation_errors(instance)
    if schema_id == "apxm.invocation-admission.v1":
        artifact_digest = instance.get("artifact_digest")
        release_digest = instance.get("release_digest")
        provenance_digest = instance.get("provenance_digest")
        if artifact_digest != release_digest:
            return ["artifact_digest must equal release_digest"]
        if artifact_digest != provenance_digest:
            return ["artifact_digest must equal provenance_digest"]
    return []


def capability_invocation_errors(instance: dict[str, Any]) -> list[str]:
    arguments = instance.get("arguments")
    if isinstance(arguments, dict):
        encoded = arguments.get("canonical_json")
        if isinstance(encoded, str):
            if len(encoded.encode("utf-8")) > 1_048_576:
                return ["arguments.canonical_json exceeds the canonical byte ceiling"]
            try:
                decoded = json.loads(encoded)
            except json.JSONDecodeError:
                return ["arguments.canonical_json must contain JSON"]
            expected = canonical(decoded)
            if encoded != expected:
                return ["arguments.canonical_json must use canonical JSON bytes"]
            digest = "sha256:" + hashlib.sha256(encoded.encode("utf-8")).hexdigest()
            if arguments.get("digest") != digest:
                return ["arguments.digest must cover the exact canonical JSON bytes"]

    correlation = instance.get("correlation")
    authority = instance.get("authority")
    effect = instance.get("effect")
    for container, field, expected_type in (
        (correlation, "program_invocation_ref", "ProgramInvocationRef"),
        (authority, "acting_principal_ref", "ActingPrincipalRef"),
        (authority, "agent_identity_ref", "AgentIdentityRef"),
        (authority, "capability_grant_ref", "CapabilityGrantRef"),
    ):
        if isinstance(container, dict):
            reference = container.get(field)
            if isinstance(reference, dict) and reference.get("ref_type") != expected_type:
                return [f"{field} must carry {expected_type}"]
    if isinstance(authority, dict):
        approval_refs = authority.get("approval_refs", [])
        for reference in approval_refs:
            if isinstance(reference, dict) and reference.get("ref_type") != "ApprovalRef":
                return ["approval_refs must carry ApprovalRef values"]
        approval_targets = [
            reference.get("ref") for reference in approval_refs if isinstance(reference, dict)
        ]
        if any(not isinstance(target, str) for target in approval_targets):
            return ["approval_refs must carry nonempty references"]
        if approval_targets != sorted(set(approval_targets)):
            return ["approval_refs must be lexically sorted and unique"]
    if isinstance(effect, dict):
        invocation = (
            correlation.get("program_invocation_ref")
            if isinstance(correlation, dict)
            else None
        )
        invocation_ref = invocation.get("ref") if isinstance(invocation, dict) else None
        node_execution_id = (
            correlation.get("node_execution_id") if isinstance(correlation, dict) else None
        )
        if isinstance(invocation_ref, str) and isinstance(node_execution_id, str):
            effect_preimage = (
                b"apxm.capability-effect.v1\0"
                + invocation_ref.encode("utf-8")
                + b"\0"
                + node_execution_id.encode("utf-8")
            )
            expected_effect_id = "capability-effect." + hashlib.sha256(effect_preimage).hexdigest()
            if effect.get("effect_id") != expected_effect_id:
                return ["effect_id must match the canonical invocation-coordinate preimage"]

        if all(isinstance(value, dict) for value in (arguments, correlation, authority)):
            request_identity = {
                "schema_version": "apxm.capability-request-identity.v1",
                "capability_ref": instance.get("capability_ref"),
                "arguments": arguments,
                "correlation": correlation,
                "authority": authority,
                "effect_id": effect.get("effect_id"),
            }
            expected_request_digest = content_digest(request_identity)
            if effect.get("request_digest") != expected_request_digest:
                return ["request_digest must cover the canonical request identity"]

        idempotency = effect.get("idempotency_key")
        if isinstance(idempotency, dict):
            if isinstance(invocation, dict) and idempotency.get("scope_ref") != invocation.get("ref"):
                return ["effect idempotency scope must equal the Program Invocation"]
            if idempotency.get("request_digest") != effect.get("request_digest"):
                return ["effect and idempotency request digests must match"]
            if idempotency.get("key_id") != effect.get("effect_id"):
                return ["effect id and idempotency key id must match"]
    return []


def external_agent_evidence_errors(instance: dict[str, Any]) -> list[str]:
    events = instance.get("attributed_events")
    if not isinstance(events, list):
        return []
    reverse_fields = ("reverse_operation", "reverse_target", "reverse_decision")
    last_sequence: int | None = None
    for index, event in enumerate(events):
        if not isinstance(event, dict):
            continue
        sequence = event.get("event_sequence")
        if isinstance(sequence, int):
            if last_sequence is not None and sequence <= last_sequence:
                return [f"attributed_events[{index}]: event_sequence must be strictly monotonic"]
            last_sequence = sequence
        kind = event.get("kind")
        present = [field for field in reverse_fields if field in event]
        if kind == "reverse_request":
            missing = [field for field in reverse_fields if field not in event]
            if missing:
                return [
                    f"attributed_events[{index}]: a reverse_request records {missing[0]}"
                ]
        elif present:
            return [
                f"attributed_events[{index}]: {present[0]} belongs only to a reverse_request"
            ]
    return []


def artifact_abstraction_errors(instance: dict[str, Any]) -> list[str]:
    requirements = instance.get("artifact_semantic_requirements")
    if not isinstance(requirements, list):
        return []
    for index, requirement in enumerate(requirements):
        if not isinstance(requirement, dict):
            continue
        scope = requirement.get("source_scope")
        if scope != "artifact_semantic":
            return [
                f"artifact_semantic_requirements[{index}]: an executable artifact "
                f"emits only artifact_semantic requirements, got {scope!r}"
            ]
    return []


def execution_commit_atomicity_errors(instance: dict[str, Any]) -> list[str]:
    forbidden = {
        "state_commit_ref",
        "checkpoint_commit_ref",
        "effect_commit_ref",
        "evidence_commit_ref",
        "usage_commit_ref",
        "output_commit_ref",
        "partial_commits",
        "split_commits",
    }
    present = sorted(forbidden.intersection(instance))
    if present:
        return [f"execution commit carries forbidden split/partial field {present[0]!r}"]
    expected = [
        "state_continuation",
        "checkpoint_effect_outcomes",
        "runtime_evidence",
        "usage_facts",
        "session_output_refs",
    ]
    if instance.get("atomic_write_set") != expected:
        return ["execution commit atomic_write_set must cover exactly the canonical atomic members"]
    for field, expected_type in (
        ("program_instance_ref", "ProgramInstanceRef"),
        ("invocation_ref", "ProgramInvocationRef"),
    ):
        reference = instance.get(field)
        if not isinstance(reference, dict) or reference.get("ref_type") != expected_type:
            return [f"execution commit {field} must carry {expected_type}"]
    idempotency_key = instance.get("idempotency_key")
    invocation_ref = instance.get("invocation_ref")
    if (
        isinstance(idempotency_key, dict)
        and isinstance(invocation_ref, dict)
        and idempotency_key.get("scope_ref") != invocation_ref.get("ref")
    ):
        return ["execution commit idempotency scope must equal invocation_ref.ref"]
    return []


def runtime_evidence_errors(instance: dict[str, Any]) -> list[str]:
    facts = instance.get("facts")
    if not isinstance(facts, list):
        return []
    last_sequence: int | None = None
    seen_fact_ids: set[str] = set()
    seen_node_executions: dict[str, tuple[str, str, set[tuple[str, str]]]] = {}
    seen_model_attempt_ids: set[tuple[str, str, str]] = set()
    seen_model_attempt_indexes: set[tuple[str, str, int]] = set()
    seen_iterations: set[tuple[str, str, int]] = set()
    next_iterations: dict[tuple[str, str, str], int] = {}
    for index, fact in enumerate(facts):
        if not isinstance(fact, dict):
            continue
        fact_id = fact.get("fact_id")
        if isinstance(fact_id, str):
            if fact_id in seen_fact_ids:
                return [f"facts[{index}]: fact_id must be replay-stable and unique"]
            seen_fact_ids.add(fact_id)
        sequence = fact.get("event_sequence")
        if isinstance(sequence, int):
            if last_sequence is not None and sequence <= last_sequence:
                return [f"facts[{index}]: event_sequence must be strictly monotonic"]
            last_sequence = sequence
        if fact.get("fact_kind") == "effect.outcome_unknown":
            if fact.get("model_outcome") == "committed_success" or fact.get("invocation_state") in {
                "committed_yield",
                "committed_return",
            }:
                return [
                    f"facts[{index}]: an uncertain effect fact cannot also record committed success"
                ]
        if fact.get("fact_kind") == "region.occurrence_started":
            if not fact.get("region_occurrence_id") or not fact.get("static_region_id"):
                return [
                    f"facts[{index}]: a region occurrence requires dynamic and static region ids"
                ]
        if fact.get("fact_kind") == "node_execution.recorded":
            required = (
                "node_execution_id",
                "air_node_id",
                "execution_scope",
            )
            if any(not fact.get(field) for field in required):
                return [
                    f"facts[{index}]: a NodeExecution requires exact AIR and region joins"
                ]
            scope = fact["execution_scope"]
            memberships = scope.get("loop_memberships", []) if isinstance(scope, dict) else []
            node_execution_id = fact["node_execution_id"]
            if node_execution_id in seen_node_executions:
                return [f"facts[{index}]: node_execution_id conflicts with replay"]
            seen_node_executions[node_execution_id] = (
                fact["program_invocation_id"],
                fact["air_node_id"],
                {
                    (membership["static_loop_id"], membership["loop_occurrence_id"])
                    for membership in memberships
                    if isinstance(membership, dict)
                    and "static_loop_id" in membership
                    and "loop_occurrence_id" in membership
                },
            )
        if fact.get("fact_kind") == "attempt.recorded":
            node = seen_node_executions.get(fact.get("node_execution_id"))
            if node is None:
                return [
                    f"facts[{index}]: a model attempt requires a preceding committed NodeExecution"
                ]
            if node[0] != fact.get("program_invocation_id"):
                return [
                    f"facts[{index}]: a model attempt must match its NodeExecution Program Invocation"
                ]
            if node[1] != fact.get("air_node_id"):
                return [
                    f"facts[{index}]: a model attempt must match its NodeExecution AIR node"
                ]
            attempt_id_identity = (
                fact.get("program_invocation_id"),
                fact.get("node_execution_id"),
                fact.get("attempt_id"),
            )
            if attempt_id_identity in seen_model_attempt_ids:
                return [f"facts[{index}]: model attempt identity conflicts with replay"]
            seen_model_attempt_ids.add(attempt_id_identity)
            attempt_index_identity = (
                fact.get("program_invocation_id"),
                fact.get("node_execution_id"),
                fact.get("attempt_index"),
            )
            if attempt_index_identity in seen_model_attempt_indexes:
                return [f"facts[{index}]: model attempt index conflicts with replay"]
            seen_model_attempt_indexes.add(attempt_index_identity)
        if fact.get("fact_kind") == "LoopIterationCompleted":
            required = (
                "static_loop_id",
                "loop_occurrence_id",
                "iteration_index",
                "program_invocation_id",
                "causal_node_execution_ids",
            )
            if any(field not in fact for field in required):
                return [
                    f"facts[{index}]: LoopIterationCompleted requires loop and causal identity"
                ]
            causal_ids = fact["causal_node_execution_ids"]
            if (
                not isinstance(causal_ids, list)
                or not causal_ids
                or len(set(causal_ids)) != len(causal_ids)
                or any(
                    (
                        fact["static_loop_id"],
                        fact["loop_occurrence_id"],
                    )
                    not in seen_node_executions.get(value, ("", "", set()))[2]
                    for value in causal_ids
                )
            ):
                return [
                    f"facts[{index}]: LoopIterationCompleted causality must be non-empty, unique, committed, and same-occurrence"
                ]
            identity = (
                fact["static_loop_id"],
                fact["loop_occurrence_id"],
                fact["iteration_index"],
            )
            if identity in seen_iterations:
                return [
                    f"facts[{index}]: loop iteration identity conflicts with replay"
                ]
            seen_iterations.add(identity)
            sequence_key = (
                fact["static_loop_id"],
                fact["loop_occurrence_id"],
                fact["program_invocation_id"],
            )
            expected_iteration = next_iterations.get(sequence_key, 0)
            if fact["iteration_index"] != expected_iteration:
                return [
                    f"facts[{index}]: iteration_index must be zero-based and contiguous"
                ]
            next_iterations[sequence_key] = expected_iteration + 1
        if fact.get("fact_kind") == "hook.executed":
            required = (
                "hook_execution_id",
                "hook_id",
                "context_before_ref",
                "context_after_ref",
            )
            if any(not fact.get(field) for field in required):
                return [
                    f"facts[{index}]: a Hook execution requires exact Hook and Context joins"
                ]
        if fact.get("fact_kind") == "context.transitioned":
            required = (
                "context_transition_id",
                "context_before_ref",
                "context_after_ref",
            )
            if any(not fact.get(field) for field in required):
                return [
                    f"facts[{index}]: a Context transition requires exact before/after joins"
                ]
    return []


def load_schema_ids() -> dict[str, dict[str, Any]]:
    schema_ids: dict[str, dict[str, Any]] = {}
    for schemas_dir in (SCHEMAS_DIR, CONSTITUTION_SCHEMAS_DIR):
        for path in sorted(schemas_dir.glob("*.json")):
            data = load_json(path)
            if isinstance(data, dict) and isinstance(data.get("$id"), str):
                schema_ids.setdefault(data["$id"], data)
    return schema_ids


def validate_vectors(schema_ids: dict[str, dict[str, Any]]) -> None:
    for vector_name, schema_id in sorted(VECTOR_SCHEMA.items()):
        path = VECTORS_DIR / vector_name
        cases = load_json(path)
        if not isinstance(cases, list) or not cases:
            raise ValidationError(f"vectors/{vector_name}: must be a non-empty array")
        schema = schema_ids.get(schema_id)
        if schema is None:
            raise ValidationError(f"vectors/{vector_name}: unknown schema {schema_id}")
        saw_valid = False
        saw_invalid = False
        for index, case in enumerate(cases):
            if not isinstance(case, dict) or not isinstance(case.get("name"), str):
                raise ValidationError(f"vectors/{vector_name}[{index}]: case needs a name")
            expected = case.get("expected_valid")
            if not isinstance(expected, bool):
                raise ValidationError(
                    f"vectors/{vector_name}[{index}]: expected_valid must be boolean"
                )
            instance = case.get("input")
            errors = schema_instance_errors(schema, instance, schema_ids, schema)
            errors.extend(semantic_errors(schema_id, instance))
            actual = not errors
            if actual != expected:
                raise ValidationError(
                    f"vectors/{vector_name}[{index}] ({case['name']}): "
                    f"expected_valid={expected} but validator returned {actual} (errors={errors})"
                )
            saw_valid |= expected
            saw_invalid |= not expected
        if not saw_valid or not saw_invalid:
            raise ValidationError(f"vectors/{vector_name}: must contain both valid and invalid cases")


def compute_port_contract(
    descriptor: dict[str, Any],
    instance: dict[str, Any],
    *,
    boundary_key: str,
    request_schema: Path,
    result_schema: Path,
    failure_schema: Path,
    vectors: tuple[Path, ...],
) -> dict[str, Any]:
    boundary = descriptor[boundary_key]
    updated = copy.deepcopy(instance)
    updated["request_schema"]["digest"] = file_digest(request_schema)
    updated["result_schema"]["digest"] = file_digest(result_schema)
    updated["failure_schema"]["digest"] = file_digest(failure_schema)
    updated["lifecycle_digest"] = content_digest(boundary["lifecycle"])
    updated["authority_data_classification_digest"] = content_digest(
        boundary["authority_data_classification"]
    )
    updated["feature_vocabulary_digest"] = content_digest(boundary["feature_vocabulary"])
    updated["configuration_contract_digest"] = content_digest(boundary["configuration_contract"])
    if len(vectors) == 1:
        updated["conformance_vector_digest"] = file_digest(vectors[0])
    else:
        updated["conformance_vector_digest"] = content_digest(
            {
                "members": [
                    {
                        "path": f"vectors/{path.name}",
                        "digest": file_digest(path),
                    }
                    for path in vectors
                ]
            }
        )
    without_self = copy.deepcopy(updated)
    without_self.pop("port_contract_digest", None)
    updated["port_contract_digest"] = content_digest(without_self)
    return updated


def compute_descriptor(descriptor: dict[str, Any]) -> dict[str, Any]:
    updated = copy.deepcopy(descriptor)
    updated["constitution"]["digest"] = file_digest(
        CONSTITUTION_SCHEMAS_DIR / "contract-constitution.v1.json"
    )
    for entry in updated["referenced_common_envelopes"]:
        name = entry["schema_id"].removeprefix("apxm.").rsplit(".v", 1)[0] + ".v1.json"
        entry["digest"] = file_digest(CONSTITUTION_SCHEMAS_DIR / name)
    for entry in updated["owned_schemas"]:
        entry["digest"] = file_digest(CONTRACTS_DIR / entry["path"])
    for entry in updated["owned_port_contracts"]:
        entry["digest"] = file_digest(CONTRACTS_DIR / entry["path"])
    for entry in updated["conformance_vectors"]:
        entry["digest"] = file_digest(CONTRACTS_DIR / entry["path"])
    content = copy.deepcopy(updated)
    content.pop("descriptor_digest", None)
    updated["descriptor_digest"] = content_digest(content)
    return updated


def write_digests() -> None:
    descriptor = load_json(DESCRIPTOR_PATH)
    port_contracts: list[tuple[Path, dict[str, Any]]] = []
    for path, boundary_key, request, result, failure, vectors in PORT_CONTRACT_SPECS:
        updated = compute_port_contract(
            descriptor,
            load_json(path),
            boundary_key=boundary_key,
            request_schema=request,
            result_schema=result,
            failure_schema=failure,
            vectors=vectors,
        )
        path.write_text(json.dumps(updated, indent=2) + "\n", encoding="utf-8")
        port_contracts.append((path, updated))
    new_descriptor = compute_descriptor(descriptor)
    DESCRIPTOR_PATH.write_text(json.dumps(new_descriptor, indent=2) + "\n", encoding="utf-8")
    DESCRIPTOR_SIDECAR_PATH.write_text(
        f"{file_digest(DESCRIPTOR_PATH)}  contracts/descriptors/{DESCRIPTOR_PATH.name}\n"
        "digest_scope: exact repository bytes\n",
        encoding="utf-8",
    )
    print(f"descriptor_digest: {new_descriptor['descriptor_digest']}")
    for path, contract in port_contracts:
        print(f"{path.stem}: {contract['port_contract_digest']}")


def check_digests() -> str:
    descriptor = load_json(DESCRIPTOR_PATH)
    for path, boundary_key, request, result, failure, vectors in PORT_CONTRACT_SPECS:
        instance = load_json(path)
        expected = compute_port_contract(
            descriptor,
            instance,
            boundary_key=boundary_key,
            request_schema=request,
            result_schema=result,
            failure_schema=failure,
            vectors=vectors,
        )
        if expected != instance:
            raise ValidationError(f"{path.name} digests are stale; run --write-digests")
    expected_descriptor = compute_descriptor(descriptor)
    if expected_descriptor != descriptor:
        raise ValidationError("owner descriptor digests are stale; run --write-digests")
    return descriptor["descriptor_digest"]


def check_port_contract_envelope(schema_ids: dict[str, dict[str, Any]]) -> None:
    schema = schema_ids["apxm.port-contract.v1"]
    for path, *_ in PORT_CONTRACT_SPECS:
        instance = load_json(path)
        errors = schema_instance_errors(schema, instance, schema_ids, schema)
        if errors:
            raise ValidationError(f"{path.name} is not a valid port contract envelope: {errors}")
        if instance.get("semantic_owner") != "agents":
            raise ValidationError(f"{path.name} must be owned by agents")


def check_descriptor_shape(schema_ids: dict[str, dict[str, Any]]) -> None:
    descriptor = load_json(DESCRIPTOR_PATH)
    if descriptor.get("schema_version") != "apxm.agents-owner-descriptor.v1":
        raise ValidationError("descriptor schema_version drifted")
    if descriptor.get("semantic_owner") != "agents":
        raise ValidationError("descriptor semantic_owner must be agents")
    if descriptor.get("source_scope_invariant") != "artifact_semantic":
        raise ValidationError("descriptor source_scope_invariant must be artifact_semantic")
    declared = {entry["schema_id"] for entry in descriptor["owned_schemas"]}
    present = {load_json(path)["$id"] for path in SCHEMAS_DIR.glob("*.json")}
    if declared != present:
        raise ValidationError(
            f"owned_schemas drifted from schemas/: declared={sorted(declared)} present={sorted(present)}"
        )
    # Every referenced envelope must exist in the published constitution layer.
    for entry in descriptor["referenced_common_envelopes"] + [descriptor["constitution"]]:
        if entry["schema_id"] not in schema_ids:
            raise ValidationError(f"referenced envelope {entry['schema_id']} is not published")
    if "signing" in descriptor:
        raise ValidationError(
            "owner-descriptor signatures are detached distribution metadata"
        )
    check_reference_host_boundary(descriptor)
    check_reference_host_release_evidence()


def check_reference_host_boundary(descriptor: dict[str, Any]) -> None:
    references = descriptor.get("referenced_owner_descriptors")
    if not isinstance(references, list):
        raise ValidationError("referenced_owner_descriptors must be an array")
    if len(references) != 1:
        raise ValidationError(
            "referenced_owner_descriptors must contain exactly the canonical Host SDK cohort"
        )
    reference = references[0]
    if not isinstance(reference, dict):
        raise ValidationError("referenced_owner_descriptors[0] must be an object")
    semantic_owner = reference.get("semantic_owner")
    if semantic_owner in RETIRED_REFERENCE_HOST_OWNERS:
        raise ValidationError(
            f"referenced_owner_descriptors[0].semantic_owner uses retired alias {semantic_owner!r}"
        )
    schema_version = reference.get("schema_version")
    if schema_version in RETIRED_REFERENCE_HOST_SCHEMAS:
        raise ValidationError(
            f"referenced_owner_descriptors[0].schema_version uses retired alias {schema_version!r}"
        )
    if reference != REFERENCE_HOST_DESCRIPTOR:
        drifted = sorted(
            key
            for key in set(reference).union(REFERENCE_HOST_DESCRIPTOR)
            if reference.get(key) != REFERENCE_HOST_DESCRIPTOR.get(key)
        )
        raise ValidationError(
            "referenced_owner_descriptors[0] drifted from the canonical Host SDK cohort: "
            + ", ".join(drifted)
        )

    manifest = load_toml(WORKSPACE_MANIFEST)
    workspace = manifest.get("workspace")
    if not isinstance(workspace, dict):
        raise ValidationError("Cargo.toml must define a workspace table")
    dependencies = workspace.get("dependencies")
    if not isinstance(dependencies, dict):
        raise ValidationError("Cargo.toml must define workspace.dependencies")
    host_dependency = dependencies.get(REFERENCE_HOST_DEPENDENCY_NAME)
    if not isinstance(host_dependency, dict):
        raise ValidationError(
            f"Cargo.toml must declare workspace dependency {REFERENCE_HOST_DEPENDENCY_NAME}"
        )
    if host_dependency.get("git") != REFERENCE_HOST_DEPENDENCY_GIT:
        raise ValidationError(
            f"{REFERENCE_HOST_DEPENDENCY_NAME} must pin git {REFERENCE_HOST_DEPENDENCY_GIT}"
        )
    if host_dependency.get("rev") != REFERENCE_HOST_DESCRIPTOR["source_revision"]:
        raise ValidationError(
            f"{REFERENCE_HOST_DEPENDENCY_NAME} rev must match referenced owner descriptor source_revision"
        )


def check_reference_host_release_evidence() -> None:
    release_manifest = load_json(REFERENCE_HOST_RELEASE_MANIFEST_PATH)
    if release_manifest.get("schema_version") != "apxm.reference-host-release-manifest.v1":
        raise ValidationError("reference-host release manifest schema_version drifted")
    if release_manifest.get("semantic_owner") != "agents":
        raise ValidationError("reference-host release manifest semantic_owner must be agents")
    if release_manifest.get("profile_cohort") != REFERENCE_HOST_PROFILE_COHORT:
        raise ValidationError("reference-host release manifest profile_cohort drifted")
    if RETIRED_REFERENCE_HOST_ADMISSION_ALIAS in canonical(release_manifest):
        raise ValidationError(
            "reference-host release manifest must not cite retired alias apxm.execution-admission.v1"
        )

    execution_manifest = load_json(REFERENCE_HOST_EXECUTION_MANIFEST_PATH)
    if execution_manifest.get("schema_version") != REFERENCE_HOST_EXECUTION_MANIFEST_SCHEMA_VERSION:
        raise ValidationError("reference-host execution manifest schema_version drifted")
    if execution_manifest.get("semantic_owner") != "agents":
        raise ValidationError("reference-host execution manifest semantic_owner must be agents")
    for key in ("owner_executable", "owner_executable_path", "transport_protocol"):
        if execution_manifest.get(key) != release_manifest.get(key):
            raise ValidationError(
                f"reference-host execution manifest {key} must match the release manifest"
            )
    expected_publication_cohort = reference_host_publication_cohort(execution_manifest)
    publication_cohort = release_manifest.get("publication_cohort")
    if publication_cohort != expected_publication_cohort:
        raise ValidationError(
            "reference-host release manifest publication_cohort drifted from the execution manifest"
        )

    invoke_vector = load_json(REFERENCE_HOST_INVOKE_PARITY_VECTOR_PATH)
    if invoke_vector.get("schema_version") != REFERENCE_HOST_INVOKE_VECTOR_ID:
        raise ValidationError("reference-host invoke parity vector schema_version drifted")
    if invoke_vector.get("profiles") != REFERENCE_HOST_PROFILE_COHORT:
        raise ValidationError("reference-host invoke parity vector profiles drifted")
    lifecycle_vector = load_json(REFERENCE_HOST_LIFECYCLE_PARITY_VECTOR_PATH)
    if lifecycle_vector.get("schema_version") != REFERENCE_HOST_LIFECYCLE_VECTOR_ID:
        raise ValidationError("reference-host lifecycle parity vector schema_version drifted")
    if lifecycle_vector.get("profiles") != REFERENCE_HOST_PROFILE_COHORT:
        raise ValidationError("reference-host lifecycle parity vector profiles drifted")
    expected_execution_manifest_ref = reference_host_execution_manifest_ref()
    expected_invoke_vector_ref = reference_host_invoke_vector_ref(invoke_vector)
    expected_lifecycle_vector_ref = reference_host_lifecycle_vector_ref(lifecycle_vector)
    golden_vectors = release_manifest.get("golden_vectors")
    if not isinstance(golden_vectors, list):
        raise ValidationError("reference-host release manifest golden_vectors must be an array")
    expected_golden_vectors = [expected_invoke_vector_ref, expected_lifecycle_vector_ref]
    if golden_vectors != expected_golden_vectors:
        raise ValidationError("reference-host release manifest golden_vectors drifted")

    expected_attestation = {
        "profile_cohort": REFERENCE_HOST_PROFILE_COHORT,
        "execution_manifest": expected_execution_manifest_ref,
        "lifecycle_vector": expected_lifecycle_vector_ref,
        "shared_contracts": lifecycle_vector["shared_contracts"],
        "required_cases": [case["name"] for case in lifecycle_vector["cases"]],
    }
    attestation = release_manifest.get("lifecycle_cohort_attestation")
    if attestation != expected_attestation:
        raise ValidationError(
            "reference-host release manifest lifecycle_cohort_attestation drifted"
        )


def scan_plan_references() -> None:
    hits: list[str] = []
    for path in sorted(CONTRACTS_DIR.rglob("*")):
        if not path.is_file() or path.suffix not in {".json", ".md", ".py"}:
            continue
        if path == SCRIPT_DIR / "validate_owner_descriptor.py":
            continue  # this gate defines the forbidden patterns themselves
        text = path.read_text(encoding="utf-8")
        for line_no, line in enumerate(text.splitlines(), start=1):
            for pattern in FORBIDDEN_TEXT:
                if re.search(pattern, line, re.IGNORECASE):
                    hits.append(f"{path.relative_to(CONTRACTS_DIR)}:{line_no}: {pattern}")
            if FORBIDDEN_LANE_IDS.search(line):
                hits.append(f"{path.relative_to(CONTRACTS_DIR)}:{line_no}: lane-id token")
    if hits:
        raise ValidationError("plan-reference scan found hits:\n  " + "\n  ".join(hits))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write-digests", action="store_true", help="recompute and write digests")
    args = parser.parse_args()

    if args.write_digests:
        write_digests()
        return 0

    try:
        schema_ids = load_schema_ids()
        validate_vectors(schema_ids)
        check_port_contract_envelope(schema_ids)
        check_descriptor_shape(schema_ids)
        digest = check_digests()
        scan_plan_references()
    except (ValidationError, KeyError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1

    print("PASS: agents owner descriptor")
    print(f"descriptor_digest: {digest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
