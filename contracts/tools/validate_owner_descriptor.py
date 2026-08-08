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
WORKSPACE_MANIFEST = AGENTS_ROOT / "Cargo.toml"
REFERENCE_HOST_RELEASE_MANIFEST_PATH = (
    CONTRACTS_DIR / "reference-host" / "manifests" / "apxm.reference-host-release-manifest.v1.json"
)
REFERENCE_HOST_EXECUTION_MANIFEST_PATH = (
    CONTRACTS_DIR / "reference-host" / "manifests" / "apxm.reference-host-execution-manifest.v1.json"
)
REFERENCE_HOST_STARTUP_INPUT_SCHEMA_PATH = (
    CONTRACTS_DIR / "schemas" / "apxm.reference-host-startup-input.v1.json"
)
REFERENCE_HOST_STARTUP_INPUT_VECTOR_PATH = (
    CONTRACTS_DIR / "vectors" / "apxm.reference-host-startup-input.v1.json"
)
REFERENCE_HOST_PUBLISHED_STARTUP_INPUT_SCHEMA_PATH = (
    CONTRACTS_DIR / "reference-host" / "schemas" / "apxm.reference-host-startup-input.v1.json"
)
REFERENCE_HOST_PUBLISHED_STARTUP_INPUT_VECTOR_PATH = (
    CONTRACTS_DIR / "reference-host" / "vectors" / "apxm.reference-host-startup-input.v1.json"
)
REFERENCE_HOST_INVOKE_PARITY_VECTOR_PATH = (
    CONTRACTS_DIR / "reference-host" / "vectors" / "apxm.reference-host.invoke-parity.v1.json"
)
REFERENCE_HOST_LIFECYCLE_PARITY_VECTOR_PATH = (
    CONTRACTS_DIR / "reference-host" / "vectors" / "apxm.reference-host.lifecycle-parity.v1.json"
)
REFERENCE_HOST_EXECUTABLE_HARNESS_PATH = (
    CHECKOUT_ROOT / "crates" / "tools" / "cli" / "tests" / "reference_host_jsonl.rs"
)
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
    "apxm.inference-driver-binding.v1.json": "apxm.inference-driver-binding.v1",
    "apxm.inference-credential-lease.v1.json": "apxm.inference-credential-lease.v1",
    "apxm.inference-usage-lineage.v1.json": "apxm.inference-usage-lineage.v1",
    "apxm.diagnostic-correlation.v1.json": "apxm.diagnostic-correlation.v1",
    "apxm.vllm-conformance-join.v1.json": "apxm.vllm-conformance-join.v1",
    "apxm.runtime-readiness.v1.json": "apxm.runtime-readiness.v1",
    "apxm.runtime-drain-quiescence.v1.json": "apxm.runtime-drain-quiescence.v1",
    "apxm.invocation-admission.v1.json": "apxm.invocation-admission.v1",
    "apxm.runtime.host-request.v1.json": "apxm.runtime.host-request.v1",
    "apxm.reference-host-startup-input.v1.json": "apxm.reference-host-startup-input.v1",
    "apxm.execution-admission.v1.json": "apxm.execution-admission.v1",
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
REFERENCE_HOST_STARTUP_INPUT_FIXTURE_SCHEMA_VERSION = (
    "apxm.reference-host.startup-input.test.v1"
)
REFERENCE_HOST_STARTUP_INPUT_FIXTURE_RELATIVE_PATH = (
    "reference-host/fixtures/apxm.reference-host.startup-input.test.json"
)
REFERENCE_HOST_STARTUP_INPUT_FAIL_CLOSED_REASONS = [
    "missing",
    "stale",
    "dirty",
    "mismatched",
]
REFERENCE_HOST_TRANSPORT_PROTOCOL = "jsonl-stdin-stdout"
REFERENCE_HOST_RELEASE_CONSTRAINTS = {
    "http_surface": "absent",
    "apxm_server_dependency": "absent",
    "source_checkout_fallback": "absent",
    "schema_aliases": "absent",
    "mixed_generation": "absent",
    "clic_dependency": "absent",
}
REFERENCE_HOST_INVOKE_VECTOR_ID = "apxm.reference-host.invoke-parity.v1"
REFERENCE_HOST_LIFECYCLE_VECTOR_ID = "apxm.reference-host.lifecycle-parity.v1"
RETIRED_REFERENCE_HOST_ADMISSION_ALIAS = "apxm.execution-admission.v1"
REFERENCE_HOST_EXECUTION_SCHEMA_KEYS = (
    "startup_input_schema",
    "host_execution_manifest_schema",
    "runtime_readiness_schema",
    "runtime_drain_quiescence_schema",
    "invocation_admission_schema",
    "host_request_schema",
    "host_response_schema",
    "host_transport_schema",
)
REFERENCE_HOST_EXECUTION_VECTOR_KEYS = (
    "startup_input_vector",
    "runtime_readiness_vector",
    "runtime_drain_quiescence_vector",
    "invocation_admission_vector",
    "host_request_vector",
    "host_response_vector",
    "host_transport_vector",
)
REFERENCE_HOST_EXECUTION_SCHEMA_SPECS = (
    (
        "startup_input_schema",
        "apxm.reference-host-startup-input.v1",
        "reference-host/schemas/apxm.reference-host-startup-input.v1.json",
        "schemas/apxm.reference-host-startup-input.v1.json",
    ),
    (
        "host_execution_manifest_schema",
        "apxm.host-execution-manifest.v1",
        "reference-host/schemas/apxm.host-execution-manifest.v1.json",
        "schemas/apxm.host-execution-manifest.v1.json",
    ),
    (
        "runtime_readiness_schema",
        "apxm.runtime-readiness.v1",
        "reference-host/schemas/apxm.runtime-readiness.v1.json",
        "schemas/apxm.runtime-readiness.v1.json",
    ),
    (
        "runtime_drain_quiescence_schema",
        "apxm.runtime-drain-quiescence.v1",
        "reference-host/schemas/apxm.runtime-drain-quiescence.v1.json",
        "schemas/apxm.runtime-drain-quiescence.v1.json",
    ),
    (
        "invocation_admission_schema",
        "apxm.invocation-admission.v1",
        "reference-host/schemas/apxm.invocation-admission.v1.json",
        "schemas/apxm.invocation-admission.v1.json",
    ),
    (
        "host_request_schema",
        "apxm.runtime.host-request.v1",
        "reference-host/schemas/apxm.runtime.host-request.v1.json",
        "schemas/apxm.runtime.host-request.v1.json",
    ),
    (
        "host_response_schema",
        "apxm.runtime.host-response.v1",
        "reference-host/schemas/apxm.runtime.host-response.v1.json",
        "schemas/apxm.runtime.host-response.v1.json",
    ),
    (
        "host_transport_schema",
        "apxm.runtime.host-transport.v1",
        "reference-host/schemas/apxm.runtime.host-transport.v1.json",
        "schemas/apxm.runtime.host-transport.v1.json",
    ),
)
REFERENCE_HOST_EXECUTION_VECTOR_SPECS = (
    (
        "startup_input_vector",
        "apxm.reference-host-startup-input.v1",
        "reference-host/vectors/apxm.reference-host-startup-input.v1.json",
        "vectors/apxm.reference-host-startup-input.v1.json",
    ),
    (
        "runtime_readiness_vector",
        "apxm.runtime-readiness.v1",
        "reference-host/vectors/apxm.runtime-readiness.v1.json",
        "vectors/apxm.runtime-readiness.v1.json",
    ),
    (
        "runtime_drain_quiescence_vector",
        "apxm.runtime-drain-quiescence.v1",
        "reference-host/vectors/apxm.runtime-drain-quiescence.v1.json",
        "vectors/apxm.runtime-drain-quiescence.v1.json",
    ),
    (
        "invocation_admission_vector",
        "apxm.invocation-admission.v1",
        "reference-host/vectors/apxm.invocation-admission.v1.json",
        "vectors/apxm.invocation-admission.v1.json",
    ),
    (
        "host_request_vector",
        "apxm.runtime.host-request.v1",
        "reference-host/vectors/apxm.runtime.host-request.v1.json",
        "vectors/apxm.runtime.host-request.v1.json",
    ),
    (
        "host_response_vector",
        "apxm.runtime.host-response.v1",
        "reference-host/vectors/apxm.runtime.host-response.v1.json",
        "vectors/apxm.runtime.host-response.v1.json",
    ),
    (
        "host_transport_vector",
        "apxm.runtime.host-transport.v1",
        "reference-host/vectors/apxm.runtime.host-transport.v1.json",
        "vectors/apxm.runtime.host-transport.v1.json",
    ),
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


def _git_common_dir() -> Path | None:
    try:
        common_dir = subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            check=True,
            capture_output=True,
            cwd=AGENTS_ROOT,
            text=True,
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None
    return Path(common_dir) if common_dir else None


def _constitution_schema_dirs() -> tuple[Path, ...]:
    candidates = [CONSTITUTION_SCHEMAS_DIR]
    common_dir = _git_common_dir()
    if common_dir is not None and common_dir.name == ".git":
        candidates.append(common_dir.parent.parent / "contracts" / "schemas")

    ordered: list[Path] = []
    seen: set[str] = set()
    for candidate in candidates:
        key = str(candidate)
        if key in seen or not candidate.is_dir():
            continue
        seen.add(key)
        ordered.append(candidate)
    return tuple(ordered)


def _published_schema_path(path: Path) -> Path:
    if path.is_file():
        return path
    if path.parent == CONSTITUTION_SCHEMAS_DIR:
        for schemas_dir in _constitution_schema_dirs():
            candidate = schemas_dir / path.name
            if candidate.is_file():
                return candidate
    return path


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


def is_placeholder_digest(digest: str) -> bool:
    if not digest.startswith("sha256:") or len(digest) != 71:
        return False
    hex_value = digest.removeprefix("sha256:")
    return len(set(hex_value)) == 1


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


def reference_host_release_manifest_ref() -> dict[str, str]:
    return {
        "schema_version": "apxm.reference-host-release-manifest.v1",
        "path": "reference-host/manifests/apxm.reference-host-release-manifest.v1.json",
        "digest": file_digest(REFERENCE_HOST_RELEASE_MANIFEST_PATH),
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
            for field, *_ in REFERENCE_HOST_EXECUTION_SCHEMA_SPECS
        ],
        "vectors": [
            publication_entry(field, key="schema_id")
            for field, *_ in REFERENCE_HOST_EXECUTION_VECTOR_SPECS
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


def reference_host_executable_harness_ref() -> dict[str, str]:
    return {
        "kind": "rust-integration-test",
        "path": "crates/tools/cli/tests/reference_host_jsonl.rs",
        "digest": file_digest(REFERENCE_HOST_EXECUTABLE_HARNESS_PATH),
    }


def reference_host_executable_parity_evidence(
    invoke_vector: dict[str, Any], lifecycle_vector: dict[str, Any]
) -> dict[str, Any]:
    case_names = [
        *(case["name"] for case in invoke_vector["cases"]),
        *(case["name"] for case in lifecycle_vector["cases"]),
    ]
    return {
        "owner_executable": {
            "name": "apxm-reference-host",
            "path": "crates/tools/cli/src/bin/reference_host.rs",
            "transport_protocol": "jsonl-stdin-stdout",
        },
        "harness": reference_host_executable_harness_ref(),
        "vectors": [
            reference_host_invoke_vector_ref(invoke_vector),
            reference_host_lifecycle_vector_ref(lifecycle_vector),
        ],
        "live_required_cases": case_names,
        "unavailable_required_cases": [],
    }


def reference_host_published_lifecycle_profile(
    release_manifest: dict[str, Any],
    execution_manifest: dict[str, Any],
    lifecycle_vector: dict[str, Any],
) -> dict[str, Any]:
    return {
        "profile_id": "reference-host",
        "owner_executable": release_manifest["owner_executable"],
        "transport_protocol": release_manifest["transport_protocol"],
        "profile_cohort": list(REFERENCE_HOST_PROFILE_COHORT),
        "startup_input_schema": "apxm.reference-host-startup-input.v1",
        "release_manifest": reference_host_release_manifest_ref(),
        "execution_manifest": reference_host_execution_manifest_ref(),
        "lifecycle_vector": reference_host_lifecycle_vector_ref(lifecycle_vector),
        "shared_contracts": lifecycle_vector["shared_contracts"],
        "required_cases": [case["name"] for case in lifecycle_vector["cases"]],
    }


def reference_host_startup_input_attestation(
    execution_manifest: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Attest the exact committed startup-input preflight bytes."""
    manifest = (
        load_json(REFERENCE_HOST_EXECUTION_MANIFEST_PATH)
        if execution_manifest is None
        else execution_manifest
    )
    check_reference_host_startup_input_preflight(manifest)
    fixture = manifest["startup_input_preflight_test_fixture"]
    artifact_file = resolve_contract_relative_path(
        fixture["artifact_path"], label="reference-host startup-input preflight fixture"
    )
    startup_input = load_json(artifact_file)
    return {
        "scope": fixture["scope"],
        "path": repo_relative_path(artifact_file),
        "schema_version": startup_input["schema_version"],
        "artifact_digest": fixture["artifact_digest"],
        "exact_bytes_digest": file_digest(artifact_file),
        "owner_executable": startup_input["owner_executable"],
        "owner_executable_path": startup_input["owner_executable_path"],
        "transport_protocol": startup_input["transport_protocol"],
        "provenance": startup_input["provenance"],
        "fail_closed_on": startup_input["fail_closed_on"],
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
        path = resolve_contract_relative_path(
            raw_path, label=f"owned port contract {contract_id}"
        )
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
    check_reference_host_release_evidence(descriptor)
    release_manifest = load_json(REFERENCE_HOST_RELEASE_MANIFEST_PATH)
    execution_manifest = load_json(REFERENCE_HOST_EXECUTION_MANIFEST_PATH)
    invoke_vector = load_json(REFERENCE_HOST_INVOKE_PARITY_VECTOR_PATH)
    lifecycle_vector = load_json(REFERENCE_HOST_LIFECYCLE_PARITY_VECTOR_PATH)
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
        "startup_input_preflight": reference_host_startup_input_attestation(
            execution_manifest
        ),
        "golden_vectors": [],
        "publication_cohort": {
            "manifests": [],
            "schemas": [],
            "vectors": [],
        },
        "owner_source_contracts": owner_source_contract_attestations(descriptor),
        "executable_parity_evidence": reference_host_executable_parity_evidence(
            invoke_vector, lifecycle_vector
        ),
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
            raise ValidationError(
                f"reference-host release manifest publication_cohort {group} must be an array"
            )
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
        max_items = schema.get("maxItems")
        if isinstance(max_items, int) and len(instance) > max_items:
            errors.append("array exceeds maxItems")
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


def _inference_digest(schema_id: str, *parts: object) -> str:
    hasher = hashlib.sha256()
    hasher.update(schema_id.encode("utf-8"))
    for part in parts:
        hasher.update(b"\0")
        hasher.update(str(part).encode("utf-8"))
    return "sha256:" + hasher.hexdigest()


def inference_credential_lease_errors(instance: dict[str, Any]) -> list[str]:
    expected = _inference_digest(
        "apxm.inference-credential-lease.v1",
        instance.get("lease_id", ""),
        "model_inference",
        instance.get("model_target_ref", ""),
        instance.get("exact_port_binding_digest", ""),
        instance.get("expires_at_unix_ms", ""),
    )
    if instance.get("lease_digest") != expected:
        return ["lease_digest must cover the exact target- and purpose-bound lease identity"]
    return []


def target_commitment_errors(instance: dict[str, Any]) -> list[str]:
    fields = (
        "target_ref",
        "target_digest",
        "model_deployment_ref",
        "exact_port_binding_digest",
        "port_contract_digest",
        "composition_digest",
        "target_generation",
        "deployment_generation",
        "binding_generation",
        "composition_generation",
        "generation_cohort_digest",
        "commit_digest",
        "state",
    )
    if any(field not in instance for field in fields):
        return ["target commitment must carry its complete closed identity"]
    if len(
        {
            instance["target_generation"],
            instance["deployment_generation"],
            instance["binding_generation"],
            instance["composition_generation"],
        }
    ) != 1:
        return ["target commitment generations must form one cohort"]
    if instance["state"] != "committed":
        return ["target commitment state must be committed"]
    generation = instance["target_generation"]
    cohort = _inference_digest(
        "apxm.inference-target-commitment.v1",
        "cohort",
        instance["target_ref"],
        instance["target_digest"],
        instance["model_deployment_ref"],
        instance["exact_port_binding_digest"],
        instance["port_contract_digest"],
        instance["composition_digest"],
        generation,
        instance["deployment_generation"],
        instance["binding_generation"],
        instance["composition_generation"],
    )
    if instance["generation_cohort_digest"] != cohort:
        return ["target commitment generation cohort digest is not canonical"]
    commit = _inference_digest(
        "apxm.inference-target-commitment.v1",
        "commit",
        cohort,
        instance["state"],
    )
    if instance["commit_digest"] != commit:
        return ["target commitment digest is not canonical"]
    return []


def inference_driver_binding_errors(instance: dict[str, Any]) -> list[str]:
    commitment = instance.get("target_commitment")
    if not isinstance(commitment, dict):
        return ["driver binding must carry a target commitment"]
    commitment_errors = target_commitment_errors(commitment)
    if commitment_errors:
        return commitment_errors
    for binding_field, commitment_field in (
        ("model_target_ref", "target_ref"),
        ("model_target_digest", "target_digest"),
        ("model_deployment_ref", "model_deployment_ref"),
        ("exact_port_binding_digest", "exact_port_binding_digest"),
        ("port_contract_digest", "port_contract_digest"),
        ("composition_digest", "composition_digest"),
    ):
        if instance.get(binding_field) != commitment.get(commitment_field):
            return [f"{binding_field} must match target_commitment.{commitment_field}"]
    return []


def model_inference_request_errors(instance: dict[str, Any]) -> list[str]:
    binding = instance.get("resolved_binding")
    if not isinstance(binding, dict):
        return []
    commitment = binding.get("target_commitment")
    if not isinstance(commitment, dict):
        return ["resolved binding must carry a target commitment"]
    commitment_errors = target_commitment_errors(commitment)
    if commitment_errors:
        return commitment_errors
    duplicated = (
        ("model_target.reference", binding.get("model_target", {}).get("reference"), commitment.get("target_ref")),
        ("model_target.target_digest", binding.get("model_target", {}).get("target_digest"), commitment.get("target_digest")),
        ("model_deployment_ref", binding.get("model_deployment_ref"), commitment.get("model_deployment_ref")),
        ("exact_port_binding.binding_digest", binding.get("exact_port_binding", {}).get("binding_digest"), commitment.get("exact_port_binding_digest")),
        ("exact_port_binding.port_contract_digest", binding.get("exact_port_binding", {}).get("port_contract_digest"), commitment.get("port_contract_digest")),
        ("composition_digest", binding.get("composition_digest"), commitment.get("composition_digest")),
    )
    for field, binding_value, commitment_value in duplicated:
        if binding_value != commitment_value:
            return [f"{field} must match target_commitment"]
    return []


def inference_usage_lineage_errors(instance: dict[str, Any]) -> list[str]:
    if instance.get("sealed") is not True:
        return ["usage lineage must be sealed before publication"]

    evidence_fact_id = instance.get("evidence_fact_id")
    commit_id = instance.get("commit_id")
    if (evidence_fact_id is None) != (commit_id is None):
        return ["evidence_fact_id and commit_id must be bound together"]

    commitment_fields = (
        "target_commitment_digest",
        "generation_cohort_digest",
        "target_generation",
        "target_port_contract_digest",
        "target_composition_digest",
    )
    commitment_values = [instance.get(field) for field in commitment_fields]
    if any(value is None for value in commitment_values) and any(
        value is not None for value in commitment_values
    ):
        return ["target commitment evidence fields must be bound together"]

    typed_error = instance.get("typed_error")
    parts: list[object] = [
        instance.get("effect_id", ""),
        instance.get("attempt_index", ""),
        instance.get("request_digest", ""),
        instance.get("model_target_ref", ""),
        instance.get("model_target_digest", ""),
        instance.get("model_deployment_ref", ""),
        instance.get("exact_port_binding_digest", ""),
    ]
    if all(value is not None for value in commitment_values):
        target_generation = instance["target_generation"]
        commitment = {
            "target_ref": instance.get("model_target_ref", ""),
            "target_digest": instance.get("model_target_digest", ""),
            "model_deployment_ref": instance.get("model_deployment_ref", ""),
            "exact_port_binding_digest": instance.get("exact_port_binding_digest", ""),
            "port_contract_digest": instance["target_port_contract_digest"],
            "composition_digest": instance["target_composition_digest"],
            "target_generation": target_generation,
            "deployment_generation": target_generation,
            "binding_generation": target_generation,
            "composition_generation": target_generation,
            "generation_cohort_digest": instance["generation_cohort_digest"],
            "commit_digest": instance["target_commitment_digest"],
            "state": "committed",
        }
        commitment_errors = target_commitment_errors(commitment)
        if commitment_errors:
            return commitment_errors
        parts.extend(commitment_values)
    if evidence_fact_id is not None:
        parts.extend((evidence_fact_id, commit_id))
    parts.extend(
        (
            instance.get("native_input_tokens", ""),
            instance.get("native_output_tokens", ""),
            instance.get("duration_ms", ""),
        )
    )
    if isinstance(typed_error, dict):
        parts.extend(
            (
                typed_error.get("category", ""),
                typed_error.get("code", ""),
                typed_error.get("message", ""),
            )
        )
    expected = _inference_digest("apxm.inference-usage-lineage.v1", *parts)
    if instance.get("lineage_id") != expected:
        return ["lineage_id must cover the immutable owner usage facts"]
    return []


def inference_diagnostic_correlation_errors(instance: dict[str, Any]) -> list[str]:
    for field in ("correlation_id", "commit_id", "model_target_ref", "model_deployment_ref"):
        value = instance.get(field)
        if (
            not isinstance(value, str)
            or not value
            or len(value) > 128
            or re.fullmatch(r"[A-Za-z0-9._:/-]+", value) is None
        ):
            return [f"{field} contains an unbounded or invalid diagnostic identifier"]
    for field in ("evidence_fact_ids", "log_refs", "metric_refs", "trace_refs"):
        references = instance.get(field, [])
        if not isinstance(references, list):
            continue
        if len(references) > 64:
            return [f"{field} exceeds the bounded diagnostic reference cardinality"]
        if any(
            not isinstance(reference, str)
            or not reference
            or len(reference) > 128
            or re.fullmatch(r"[A-Za-z0-9._:/-]+", reference) is None
            for reference in references
        ):
            return [f"{field} contains an unbounded or invalid diagnostic reference"]
    if (
        instance.get("agreement") == "disagrees_evidence_authoritative"
        and not all(
            isinstance(instance.get(field), int)
            and not isinstance(instance.get(field), bool)
            for field in ("claimed_input_tokens", "claimed_output_tokens")
        )
        ):
        return ["a disagreeing diagnostic must preserve both claimed usage values"]
    commitment_fields = (
        "target_commitment_digest",
        "generation_cohort_digest",
        "target_generation",
        "target_port_contract_digest",
        "target_composition_digest",
    )
    commitment_values = [instance.get(field) for field in commitment_fields]
    if any(value is None for value in commitment_values) and any(
        value is not None for value in commitment_values
    ):
        return ["diagnostic target commitment fields must be bound together"]
    if all(value is not None for value in commitment_values):
        generation = instance["target_generation"]
        commitment = {
            "target_ref": instance.get("model_target_ref", ""),
            "target_digest": instance.get("model_target_digest", ""),
            "model_deployment_ref": instance.get("model_deployment_ref", ""),
            "exact_port_binding_digest": instance.get("exact_port_binding_digest", ""),
            "port_contract_digest": instance["target_port_contract_digest"],
            "composition_digest": instance["target_composition_digest"],
            "target_generation": generation,
            "deployment_generation": generation,
            "binding_generation": generation,
            "composition_generation": generation,
            "generation_cohort_digest": instance["generation_cohort_digest"],
            "commit_digest": instance["target_commitment_digest"],
            "state": "committed",
        }
        commitment_errors = target_commitment_errors(commitment)
        if commitment_errors:
            return commitment_errors
    expected = _diagnostic_correlation_digest(instance)
    if instance.get("correlation_digest") != expected:
        return ["correlation_digest must cover the complete diagnostic envelope"]
    return []


def _diagnostic_correlation_digest(instance: dict[str, Any]) -> str:
    parts: list[object] = [
        instance.get("correlation_id", ""),
        instance.get("commit_id", ""),
        instance.get("request_digest", ""),
        instance.get("model_target_ref", ""),
        instance.get("model_target_digest", ""),
        instance.get("model_deployment_ref", ""),
        instance.get("exact_port_binding_digest", ""),
        instance.get("target_commitment_digest", "") or "",
        instance.get("generation_cohort_digest", "") or "",
        instance.get("target_generation", "") if instance.get("target_generation") is not None else "",
        instance.get("target_port_contract_digest", "") or "",
        instance.get("target_composition_digest", "") or "",
        instance.get("authority", ""),
        instance.get("agreement", ""),
        instance.get("claimed_input_tokens", "")
        if instance.get("claimed_input_tokens") is not None
        else "",
        instance.get("claimed_output_tokens", "")
        if instance.get("claimed_output_tokens") is not None
        else "",
    ]
    for field in ("evidence_fact_ids", "log_refs", "metric_refs", "trace_refs"):
        references = instance.get(field, [])
        parts.append(len(references) if isinstance(references, list) else "")
        if isinstance(references, list):
            parts.extend(references)
    return _inference_digest("apxm.diagnostic-correlation.v1", *parts)


def vllm_conformance_join_errors(instance: dict[str, Any]) -> list[str]:
    # Mirror of crates/runtime/inference/src/backend_join.rs.
    if instance.get("vllm_port_contract_digest") != (
        "sha256:361aaf5fd82ae1dd8279726769088c2711a55d964a8376faf4646a149fee9f3c"
    ):
        return ["vllm_port_contract_digest must match the pinned backend contract"]
    pinned_vectors = [
        "sha256:7add76f8f7df341785ef45ff51b38e979a309299509b32e76b311268cc93ea32",
        "sha256:a60b2364bbbe1304e96defcf55a8600d501933e0fbae97c140f2bc4377d5e822",
        "sha256:96914ff1d56cc2d1e74fda3a063287615392cb0197bcc1c853cb1a18a5e7e8c3",
        "sha256:5e7abcccf5c7f7276b9398ccd2c255671f23a6914eb23287b44518b30b8c7723",
        "sha256:1e9a389b24f08411738594ca3646bbcbeb18a77c398c6b07933d04903bd9be39",
    ]
    vectors = instance.get("joined_vector_digests", [])
    if not isinstance(vectors, list):
        return []
    if any(vector not in set(pinned_vectors) for vector in vectors):
        return ["joined_vector_digests contains an unpinned backend evidence digest"]
    if len(vectors) != len(set(vectors)):
        return ["joined_vector_digests must be unique"]
    if instance.get("join_status") == "joined":
        attestation = instance.get("release_attestation")
        if not isinstance(attestation, dict):
            return ["join_status requires exact external vLLM release evidence"]
        if attestation.get("release_id") != "apxm-vllm-5d825f6c1896":
            return ["release_attestation.release_id must match the pinned release"]
        if attestation.get("owner_revision") != "5d825f6c18961c2b38edb15834acbd794fc549eb":
            return ["release_attestation.owner_revision must match the pinned release"]
        if attestation.get("manifest_digest") != "sha256:b489043b6f723b3e7bf9b4a054f55383dde90f8cf25290275a813c0970bcaeb4":
            return ["release_attestation.manifest_digest must match the pinned release"]
        if attestation.get("port_contract_digest") != instance.get("vllm_port_contract_digest"):
            return ["release_attestation.port_contract_digest must match the joined port contract"]
        if attestation.get("vector_digests") != pinned_vectors:
            return ["release_attestation.vector_digests must match the pinned vector membership"]
        if vectors != attestation["vector_digests"]:
            return ["joined_vector_digests must match release_attestation.vector_digests"]
    elif instance.get("release_attestation") is not None:
        return ["candidate join must not carry release attestation"]
    return []


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
    if schema_id == "apxm.inference-credential-lease.v1":
        return inference_credential_lease_errors(instance)
    if schema_id == "apxm.model-inference-request.v1":
        return model_inference_request_errors(instance)
    if schema_id == "apxm.inference-driver-binding.v1":
        return inference_driver_binding_errors(instance)
    if schema_id == "apxm.inference-usage-lineage.v1":
        return inference_usage_lineage_errors(instance)
    if schema_id == "apxm.diagnostic-correlation.v1":
        return inference_diagnostic_correlation_errors(instance)
    if schema_id == "apxm.vllm-conformance-join.v1":
        return vllm_conformance_join_errors(instance)
    if schema_id == "apxm.external-agent-evidence.v1":
        return external_agent_evidence_errors(instance)
    if schema_id == "apxm.capability-invocation.v1":
        return capability_invocation_errors(instance)
    if schema_id == "apxm.invocation-admission.v1":
        # The transport authority carries independent digests. Runtime
        # admission verifies each digest against its own exact bytes; the
        # artifact, release, and provenance bytes are intentionally distinct.
        pass
    if schema_id == "apxm.reference-host-startup-input.v1":
        return reference_host_startup_input_errors(instance)
    return []


def reference_host_startup_input_errors(instance: dict[str, Any]) -> list[str]:
    if instance.get("fail_closed_on") != [
        "missing",
        "placeholder",
        "dirty",
        "mismatched",
        "implicit-default",
    ]:
        return ["fail_closed_on must publish the exact fail-closed reasons"]

    for field in ("release_digest", "port_bindings_digest", "resource_ceiling_digest"):
        digest = instance.get(field)
        if isinstance(digest, str) and is_placeholder_digest(digest):
            return [f"{field} must not be a placeholder digest"]

    manifest_ref = instance.get("reference_host_release_manifest")
    if isinstance(manifest_ref, dict):
        digest = manifest_ref.get("digest")
        if isinstance(digest, str) and is_placeholder_digest(digest):
            return ["reference_host_release_manifest.digest must not be a placeholder digest"]

    provenance = instance.get("provenance")
    if isinstance(provenance, dict):
        for field in ("descriptor_semantic_digest", "descriptor_exact_checksum"):
            digest = provenance.get(field)
            if isinstance(digest, str) and is_placeholder_digest(digest):
                return [f"provenance.{field} must not be a placeholder digest"]
        if provenance.get("dirty") is not False:
            return ["provenance.dirty must remain false"]

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
            commitment_errors = target_commitment_errors(
                {
                    "target_ref": fact.get("model_target_ref", ""),
                    "target_digest": fact.get("model_target_digest", ""),
                    "model_deployment_ref": fact.get("model_deployment_ref", ""),
                    "exact_port_binding_digest": fact.get(
                        "exact_port_binding_digest", ""
                    ),
                    "port_contract_digest": fact.get("target_port_contract_digest", ""),
                    "composition_digest": fact.get("target_composition_digest", ""),
                    "target_generation": fact.get("target_generation"),
                    "deployment_generation": fact.get("target_generation"),
                    "binding_generation": fact.get("target_generation"),
                    "composition_generation": fact.get("target_generation"),
                    "generation_cohort_digest": fact.get("generation_cohort_digest", ""),
                    "commit_digest": fact.get("target_commitment_digest", ""),
                    "state": "committed",
                }
            )
            if commitment_errors:
                return [f"facts[{index}]: {commitment_errors[0]}"]
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
    for schemas_dir in (SCHEMAS_DIR, *_constitution_schema_dirs()):
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
    updated["request_schema"]["digest"] = file_digest(_published_schema_path(request_schema))
    updated["result_schema"]["digest"] = file_digest(_published_schema_path(result_schema))
    updated["failure_schema"]["digest"] = file_digest(_published_schema_path(failure_schema))
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
    if "published_host_lifecycle_profiles" in updated:
        updated["published_host_lifecycle_profiles"] = [
            reference_host_published_lifecycle_profile(
                load_json(REFERENCE_HOST_RELEASE_MANIFEST_PATH),
                load_json(REFERENCE_HOST_EXECUTION_MANIFEST_PATH),
                load_json(REFERENCE_HOST_LIFECYCLE_PARITY_VECTOR_PATH),
            )
        ]
    updated["constitution"]["digest"] = file_digest(
        _published_schema_path(CONSTITUTION_SCHEMAS_DIR / "contract-constitution.v1.json")
    )
    for entry in updated["referenced_common_envelopes"]:
        name = entry["schema_id"].removeprefix("apxm.").rsplit(".v", 1)[0] + ".v1.json"
        entry["digest"] = file_digest(_published_schema_path(CONSTITUTION_SCHEMAS_DIR / name))
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
    check_reference_host_release_evidence(descriptor)


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


def check_reference_host_release_evidence(descriptor: dict[str, Any]) -> None:
    release_manifest = load_json(REFERENCE_HOST_RELEASE_MANIFEST_PATH)
    if release_manifest.get("schema_version") != "apxm.reference-host-release-manifest.v1":
        raise ValidationError("reference-host release manifest schema_version drifted")
    if release_manifest.get("semantic_owner") != "agents":
        raise ValidationError("reference-host release manifest semantic_owner must be agents")
    check_reference_host_release_manifest_contract(release_manifest)
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
    check_reference_host_startup_input_preflight(execution_manifest)
    check_reference_host_execution_publications(execution_manifest)
    expected_publication_cohort = reference_host_publication_cohort(execution_manifest)
    publication_cohort = release_manifest.get("publication_cohort")
    if publication_cohort != expected_publication_cohort:
        raise ValidationError(
            "reference-host release manifest publication_cohort drifted from the execution manifest"
        )

    lifecycle_vector = load_json(REFERENCE_HOST_LIFECYCLE_PARITY_VECTOR_PATH)
    if lifecycle_vector.get("schema_version") != REFERENCE_HOST_LIFECYCLE_VECTOR_ID:
        raise ValidationError("reference-host lifecycle parity vector schema_version drifted")
    if lifecycle_vector.get("profiles") != REFERENCE_HOST_PROFILE_COHORT:
        raise ValidationError("reference-host lifecycle parity vector profiles drifted")
    invoke_vector = load_json(REFERENCE_HOST_INVOKE_PARITY_VECTOR_PATH)
    if invoke_vector.get("schema_version") != REFERENCE_HOST_INVOKE_VECTOR_ID:
        raise ValidationError("reference-host invoke parity vector schema_version drifted")
    if invoke_vector.get("profiles") != REFERENCE_HOST_PROFILE_COHORT:
        raise ValidationError("reference-host invoke parity vector profiles drifted")
    expected_execution_manifest_ref = reference_host_execution_manifest_ref()
    expected_invoke_vector_ref = reference_host_invoke_vector_ref(invoke_vector)
    expected_lifecycle_vector_ref = reference_host_lifecycle_vector_ref(lifecycle_vector)
    golden_vectors = release_manifest.get("golden_vectors")
    if not isinstance(golden_vectors, list):
        raise ValidationError("reference-host release manifest golden_vectors must be an array")
    invoke_entries = [
        entry
        for entry in golden_vectors
        if isinstance(entry, dict) and entry.get("vector_id") == REFERENCE_HOST_INVOKE_VECTOR_ID
    ]
    lifecycle_entries = [
        entry
        for entry in golden_vectors
        if isinstance(entry, dict) and entry.get("vector_id") == REFERENCE_HOST_LIFECYCLE_VECTOR_ID
    ]
    if invoke_entries != [expected_invoke_vector_ref]:
        raise ValidationError("reference-host release manifest invoke parity golden vector drifted")
    if lifecycle_entries != [expected_lifecycle_vector_ref]:
        raise ValidationError(
            "reference-host release manifest lifecycle parity golden vector drifted"
        )

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

    expected_executable_parity_evidence = reference_host_executable_parity_evidence(
        invoke_vector, lifecycle_vector
    )
    if release_manifest.get("executable_parity_evidence") != expected_executable_parity_evidence:
        raise ValidationError(
            "reference-host release manifest executable_parity_evidence drifted"
        )

    expected_profile = reference_host_published_lifecycle_profile(
        release_manifest,
        execution_manifest,
        lifecycle_vector,
    )
    published_profiles = descriptor.get("published_host_lifecycle_profiles")
    if published_profiles != [expected_profile]:
        raise ValidationError(
            "owner descriptor published_host_lifecycle_profiles drifted from the exact reference-host publication"
        )


def check_reference_host_release_manifest_contract(
    release_manifest: dict[str, Any],
) -> None:
    if release_manifest.get("owner_executable") != "apxm-reference-host":
        raise ValidationError(
            "reference-host release manifest owner_executable must stay apxm-reference-host"
        )
    if release_manifest.get("transport_protocol") != REFERENCE_HOST_TRANSPORT_PROTOCOL:
        raise ValidationError(
            "reference-host release manifest transport_protocol must stay product-neutral jsonl-stdin-stdout"
        )
    if release_manifest.get("canonical_driver") != (
        "apxm_execution::RuntimeProfile::from_fully_admitted"
    ):
        raise ValidationError(
            "reference-host release manifest canonical_driver must use the shared fully admitted RuntimeProfile"
        )
    if release_manifest.get("constraints") != REFERENCE_HOST_RELEASE_CONSTRAINTS:
        raise ValidationError(
            "reference-host release manifest fail-closed constraints drifted"
        )


def check_reference_host_startup_input_preflight(
    execution_manifest: dict[str, Any],
) -> None:
    fixture = execution_manifest.get("startup_input_preflight_test_fixture")
    if not isinstance(fixture, dict):
        raise ValidationError(
            "reference-host execution manifest must declare startup_input_preflight_test_fixture"
        )
    if fixture.get("scope") != "test-only":
        raise ValidationError(
            "reference-host startup-input preflight fixture must be scoped to test-only"
        )
    artifact_path = fixture.get("artifact_path")
    if not isinstance(artifact_path, str) or not artifact_path:
        raise ValidationError(
            "reference-host startup-input preflight fixture path drifted"
        )
    artifact_file = CONTRACTS_DIR / artifact_path
    if not artifact_file.is_file():
        raise ValidationError("reference-host startup-input preflight fixture is missing")
    if artifact_path != REFERENCE_HOST_STARTUP_INPUT_FIXTURE_RELATIVE_PATH:
        raise ValidationError(
            "reference-host startup-input preflight fixture path drifted"
        )
    if fixture.get("owner_revision") != REFERENCE_HOST_DESCRIPTOR["source_revision"]:
        raise ValidationError(
            "reference-host startup-input preflight fixture owner revision is stale"
        )
    if fixture.get("fail_closed_on") != REFERENCE_HOST_STARTUP_INPUT_FAIL_CLOSED_REASONS:
        raise ValidationError(
            "reference-host startup-input preflight fail-closed reasons drifted"
        )
    if fixture.get("artifact_digest") != file_digest(artifact_file):
        raise ValidationError("reference-host startup-input preflight fixture digest mismatched")

    startup_input = load_json(artifact_file)
    if startup_input.get("schema_version") != REFERENCE_HOST_STARTUP_INPUT_FIXTURE_SCHEMA_VERSION:
        raise ValidationError("reference-host startup-input fixture schema_version drifted")
    if startup_input.get("semantic_owner") != "agents":
        raise ValidationError("reference-host startup-input fixture semantic_owner must be agents")
    if startup_input.get("scope") != "test-only":
        raise ValidationError("reference-host startup-input fixture must remain test-only")
    if startup_input.get("owner_executable") != execution_manifest.get("owner_executable"):
        raise ValidationError(
            "reference-host startup-input fixture owner executable mismatched"
        )
    if startup_input.get("owner_executable_path") != execution_manifest.get("owner_executable_path"):
        raise ValidationError(
            "reference-host startup-input fixture owner executable path mismatched"
        )
    if startup_input.get("transport_protocol") != execution_manifest.get("transport_protocol"):
        raise ValidationError(
            "reference-host startup-input fixture transport protocol mismatched"
        )
    if startup_input.get("fail_closed_on") != REFERENCE_HOST_STARTUP_INPUT_FAIL_CLOSED_REASONS:
        raise ValidationError(
            "reference-host startup-input fixture fail-closed reasons drifted"
        )

    provenance = startup_input.get("provenance")
    if not isinstance(provenance, dict):
        raise ValidationError("reference-host startup-input fixture provenance must be an object")
    if provenance.get("owner_revision") != REFERENCE_HOST_DESCRIPTOR["source_revision"]:
        raise ValidationError("reference-host startup-input fixture owner revision is stale")
    if provenance.get("descriptor_semantic_digest") != REFERENCE_HOST_DESCRIPTOR[
        "descriptor_semantic_digest"
    ]:
        raise ValidationError(
            "reference-host startup-input fixture descriptor semantic digest mismatched"
        )
    if provenance.get("descriptor_exact_checksum") != REFERENCE_HOST_DESCRIPTOR[
        "descriptor_exact_checksum"
    ]:
        raise ValidationError(
            "reference-host startup-input fixture descriptor exact checksum mismatched"
        )
    if provenance.get("dirty") is not False:
        raise ValidationError("reference-host startup-input fixture must be clean")


def check_reference_host_execution_publications(
    execution_manifest: dict[str, Any],
) -> None:
    for field, schema_id, published_path, canonical_source_path in (
        *REFERENCE_HOST_EXECUTION_SCHEMA_SPECS,
        *REFERENCE_HOST_EXECUTION_VECTOR_SPECS,
    ):
        entry = execution_manifest.get(field)
        if not isinstance(entry, dict):
            raise ValidationError(
                f"reference-host execution manifest field {field} must be an object"
            )
        if entry.get("schema_id") != schema_id:
            raise ValidationError(
                f"reference-host execution manifest field {field} schema_id drifted"
            )
        if entry.get("path") != published_path:
            raise ValidationError(
                f"reference-host execution manifest field {field} path drifted"
            )
        if entry.get("canonical_source_path") != canonical_source_path:
            raise ValidationError(
                f"reference-host execution manifest field {field} canonical_source_path drifted"
            )
        if field in REFERENCE_HOST_EXECUTION_SCHEMA_KEYS and entry.get("semantic_owner") != "agents":
            raise ValidationError(
                f"reference-host execution manifest field {field} semantic_owner must be agents"
            )
        published_file = resolve_contract_relative_path(published_path, label=field)
        source_file = resolve_contract_relative_path(canonical_source_path, label=field)
        if entry.get("digest") != file_digest(published_file):
            raise ValidationError(
                f"reference-host execution manifest field {field} digest is stale"
            )
        if published_file.read_bytes() != source_file.read_bytes():
            raise ValidationError(
                f"reference-host execution manifest field {field} must copy the canonical source bytes"
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
