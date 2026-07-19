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
                   descriptor and the execution-commit port contract instance.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
CONTRACTS_DIR = SCRIPT_DIR.parent
AGENTS_ROOT = CONTRACTS_DIR.parent
WORKSPACE_DIR = AGENTS_ROOT.parent
CONSTITUTION_SCHEMAS_DIR = WORKSPACE_DIR / "contracts" / "schemas"

SCHEMAS_DIR = CONTRACTS_DIR / "schemas"
VECTORS_DIR = CONTRACTS_DIR / "vectors"
PORT_CONTRACTS_DIR = CONTRACTS_DIR / "port-contracts"
DESCRIPTOR_PATH = CONTRACTS_DIR / "descriptors" / "apxm.agents-owner-descriptor.v1.json"
PORT_CONTRACT_PATH = PORT_CONTRACTS_DIR / "apxm.execution-commit.port-contract.v1.json"

EXECUTION_COMMIT_ENVELOPE = CONSTITUTION_SCHEMAS_DIR / "execution-commit.v1.json"
EXECUTION_COMMIT_VECTORS = VECTORS_DIR / "apxm.execution-commit.v1.json"

# Vector file -> schema `$id` it is validated against. Constitution-owned
# envelopes are resolved from the published constitution layer.
VECTOR_SCHEMA = {
    "apxm.frontend-graph.v1.json": "apxm.frontend-graph.v1",
    "apxm.air.v1.json": "apxm.air.v1",
    "apxm.executable-artifact.v1.json": "apxm.executable-artifact.v1",
    "apxm.runtime-evidence.v1.json": "apxm.runtime-evidence.v1",
    "apxm.source-map.v1.json": "apxm.source-map.v1",
    "apxm.execution-commit.v1.json": "apxm.execution-commit.v1",
    "apxm.port-contract.v1.json": "apxm.port-contract.v1",
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


class ValidationError(Exception):
    pass


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def content_digest(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical(value).encode("utf-8")).hexdigest()


def file_digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


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
    return []


def runtime_evidence_errors(instance: dict[str, Any]) -> list[str]:
    facts = instance.get("facts")
    if not isinstance(facts, list):
        return []
    last_sequence: int | None = None
    for index, fact in enumerate(facts):
        if not isinstance(fact, dict):
            continue
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


def compute_port_contract(descriptor: dict[str, Any], instance: dict[str, Any]) -> dict[str, Any]:
    boundary = descriptor["port_boundary"]
    envelope_digest = file_digest(EXECUTION_COMMIT_ENVELOPE)
    updated = copy.deepcopy(instance)
    updated["request_schema"]["digest"] = envelope_digest
    updated["result_schema"]["digest"] = envelope_digest
    updated["failure_schema"]["digest"] = envelope_digest
    updated["lifecycle_digest"] = content_digest(boundary["lifecycle"])
    updated["authority_data_classification_digest"] = content_digest(
        boundary["authority_data_classification"]
    )
    updated["feature_vocabulary_digest"] = content_digest(boundary["feature_vocabulary"])
    updated["configuration_contract_digest"] = content_digest(boundary["configuration_contract"])
    updated["conformance_vector_digest"] = file_digest(EXECUTION_COMMIT_VECTORS)
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
    content.pop("signing", None)
    updated["descriptor_digest"] = content_digest(content)
    return updated


def write_digests() -> None:
    descriptor = load_json(DESCRIPTOR_PATH)
    instance = load_json(PORT_CONTRACT_PATH)
    new_instance = compute_port_contract(descriptor, instance)
    PORT_CONTRACT_PATH.write_text(json.dumps(new_instance, indent=2) + "\n", encoding="utf-8")
    new_descriptor = compute_descriptor(descriptor)
    DESCRIPTOR_PATH.write_text(json.dumps(new_descriptor, indent=2) + "\n", encoding="utf-8")
    print(f"descriptor_digest: {new_descriptor['descriptor_digest']}")
    print(f"port_contract_digest: {new_instance['port_contract_digest']}")


def check_digests() -> str:
    descriptor = load_json(DESCRIPTOR_PATH)
    instance = load_json(PORT_CONTRACT_PATH)
    expected_instance = compute_port_contract(descriptor, instance)
    if expected_instance != instance:
        raise ValidationError(
            "execution-commit port contract digests are stale; run --write-digests"
        )
    expected_descriptor = compute_descriptor(descriptor)
    if expected_descriptor != descriptor:
        raise ValidationError("owner descriptor digests are stale; run --write-digests")
    return descriptor["descriptor_digest"]


def check_port_contract_envelope(schema_ids: dict[str, dict[str, Any]]) -> None:
    instance = load_json(PORT_CONTRACT_PATH)
    schema = schema_ids["apxm.port-contract.v1"]
    errors = schema_instance_errors(schema, instance, schema_ids, schema)
    if errors:
        raise ValidationError(f"execution-commit port contract is not a valid envelope: {errors}")
    if instance.get("semantic_owner") != "agents":
        raise ValidationError("execution-commit port contract must be owned by agents")


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
    signing = descriptor.get("signing", {})
    if signing.get("status") == "signing_pending_no_key":
        if "signature" in signing:
            raise ValidationError("signing is pending but a signature is present")
    elif "signature" not in signing:
        raise ValidationError("descriptor claims signed status without a signature")


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
