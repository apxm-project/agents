#!/usr/bin/env python3
"""Write preregistered, held-out prompt-evaluation evidence outside production."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import os
import re
import subprocess
import sys
from dataclasses import asdict, dataclass
from pathlib import Path
from pathlib import PurePosixPath
from typing import Any

from apxm_vllm.contract import build_layout


EVIDENCE_SCHEMA_VERSION = 4
OBSERVED_BACKEND_RECEIPT_SCHEMA_VERSION = 2
OBSERVED_BACKEND_EXECUTION_SCHEMA_VERSION = 1
SUPPORTED_METRICS = frozenset({"exact_match", "contains", "token_overlap"})
SAFE_IDENTIFIER_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
REVISION_RE = re.compile(r"^[0-9a-f]{7,64}$")
REPOSITORY_ID_RE = re.compile(
    r"^[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9][A-Za-z0-9._-]*$"
)
BACKEND_MEASUREMENT_SOURCES = {
    "deterministic-fixture": "fixture-values",
    "observed-backend-run": "backend-telemetry",
}
BASE_OUTPUT_FIELDS = frozenset({"id", "output", "arm_id", "token_count", "latency_ms"})
OBSERVED_OUTPUT_FIELDS = BASE_OUTPUT_FIELDS | {"evidence"}


@dataclass(frozen=True)
class CaseScore:
    case_id: str
    baseline: float
    candidate: float
    baseline_token_count: int
    candidate_token_count: int
    baseline_latency_ms: float
    candidate_latency_ms: float


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_json(value: Any) -> str:
    encoded = json.dumps(
        value,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return sha256_bytes(encoded)


def git_output(root: Path, *args: str) -> bytes:
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except FileNotFoundError as error:
        raise ValueError("git is required to record working-tree provenance") from error
    except subprocess.CalledProcessError as error:
        detail = error.stderr.decode(errors="replace").strip()
        suffix = f": {detail}" if detail else ""
        raise ValueError(f"unable to record working-tree provenance{suffix}") from error
    return result.stdout


def validate_repo_relative_path(value: str, name: str) -> str:
    path = PurePosixPath(value)
    if (
        "\\" in value
        or path.is_absolute()
        or not path.parts
        or value != path.as_posix()
        or any(part in {".", ".."} for part in path.parts)
    ):
        raise ValueError(f"{name} must be a repository-relative path")
    return value


def fingerprint_worktree_path(root: Path, relative_path: str) -> dict[str, str | None]:
    portable = validate_repo_relative_path(relative_path, "working-tree path")
    path = root / Path(portable)
    if path.is_symlink():
        return {
            "kind": "symlink",
            "sha256": sha256_bytes(os.fsencode(os.readlink(path))),
        }
    if path.is_file():
        return {"kind": "file", "sha256": sha256_file(path)}
    if not path.exists():
        return {"kind": "missing", "sha256": None}
    raise ValueError(f"working-tree path is not a file, symlink, or deletion: {portable}")


def parse_worktree_status(root: Path, value: bytes) -> list[dict[str, Any]]:
    records = value.split(b"\0")
    entries: list[dict[str, Any]] = []
    index = 0
    while index < len(records):
        record = records[index]
        index += 1
        if not record:
            continue

        record_type = record[:1]
        original_path: str | None = None
        if record_type == b"1":
            fields = record.split(b" ", 8)
            if len(fields) != 9:
                raise ValueError("git returned malformed ordinary working-tree status")
            status = fields[1].decode("ascii")
            relative_path = os.fsdecode(fields[8])
        elif record_type == b"2":
            fields = record.split(b" ", 9)
            if len(fields) != 10 or index >= len(records):
                raise ValueError("git returned malformed renamed working-tree status")
            status = fields[1].decode("ascii")
            relative_path = os.fsdecode(fields[9])
            original_path = os.fsdecode(records[index])
            index += 1
        elif record_type == b"u":
            fields = record.split(b" ", 10)
            if len(fields) != 11:
                raise ValueError("git returned malformed unmerged working-tree status")
            status = fields[1].decode("ascii")
            relative_path = os.fsdecode(fields[10])
        elif record_type == b"?":
            status = "??"
            relative_path = os.fsdecode(record[2:])
        else:
            raise ValueError("git returned an unsupported working-tree status record")

        portable_path_value = validate_repo_relative_path(relative_path, "working-tree path")
        entry: dict[str, Any] = {
            "path": portable_path_value,
            "status": status,
            **fingerprint_worktree_path(root, portable_path_value),
        }
        if original_path is not None:
            entry["original_path"] = validate_repo_relative_path(
                original_path, "working-tree original path"
            )
        entries.append(entry)

    return sorted(entries, key=lambda entry: (entry["path"], entry.get("original_path", "")))


def collect_working_tree_provenance(root: Path) -> dict[str, Any]:
    head_revision = require_revision(
        git_output(root, "rev-parse", "--verify", "HEAD").decode("ascii").strip(),
        "working-tree HEAD revision",
    )
    entries = parse_worktree_status(
        root,
        git_output(root, "status", "--porcelain=v2", "--untracked-files=all", "-z"),
    )
    canonical_entries = json.dumps(
        entries,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return {
        "head_revision": head_revision,
        "dirty": bool(entries),
        "dirty_entry_count": len(entries),
        "dirty_content_sha256": sha256_bytes(canonical_entries),
        "entries": entries,
    }


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected a JSON object")
    return value


def load_jsonl(path: Path, required_fields: set[str]) -> dict[str, dict[str, Any]]:
    rows: dict[str, dict[str, Any]] = {}
    with path.open(encoding="utf-8") as handle:
        for number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            value = json.loads(line)
            if not isinstance(value, dict):
                raise ValueError(f"{path}:{number}: expected an object")
            if not required_fields.issubset(value):
                raise ValueError(f"{path}:{number}: missing {sorted(required_fields - value.keys())}")
            case_id = value["id"]
            if not isinstance(case_id, str) or not case_id:
                raise ValueError(f"{path}:{number}: id must be a non-empty string")
            if case_id in rows:
                raise ValueError(f"{path}:{number}: duplicate id {case_id!r}")
            rows[case_id] = value
    if not rows:
        raise ValueError(f"{path}: expected at least one case")
    return rows


def require_mapping(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def require_non_empty_string(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{name} must be a non-empty string")
    return value


def require_sha256(value: Any, name: str) -> str:
    if not isinstance(value, str) or not SHA256_RE.fullmatch(value):
        raise ValueError(f"{name} must be a lowercase SHA-256 digest")
    return value


def require_number(value: Any, name: str, minimum: float, maximum: float) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    number = float(value)
    if not math.isfinite(number) or not minimum <= number <= maximum:
        raise ValueError(f"{name} must be between {minimum} and {maximum}")
    return number


def require_non_negative_int(value: Any, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"{name} must be a non-negative integer")
    return value


def require_bool(value: Any, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def require_identifier(value: Any, name: str) -> str:
    identifier = require_non_empty_string(value, name)
    if not SAFE_IDENTIFIER_RE.fullmatch(identifier):
        raise ValueError(f"{name} must be a safe non-empty identifier")
    return identifier


def require_exact_keys(value: dict[str, Any], name: str, expected: set[str]) -> None:
    missing = sorted(expected - value.keys())
    unexpected = sorted(value.keys() - expected)
    if missing:
        raise ValueError(f"{name} missing {missing}")
    if unexpected:
        raise ValueError(f"{name} has unexpected keys {unexpected}")


def require_repository_id(value: Any, name: str) -> str:
    repository_id = require_non_empty_string(value, name)
    if not REPOSITORY_ID_RE.fullmatch(repository_id):
        raise ValueError(f"{name} must be a portable owner/repository identifier")
    return repository_id


def require_revision(value: Any, name: str) -> str:
    revision = require_non_empty_string(value, name)
    if not REVISION_RE.fullmatch(revision):
        raise ValueError(f"{name} must be a lowercase revision digest")
    return revision


def require_bundle_relative_path(value: Any, name: str) -> str:
    raw_path = require_non_empty_string(value, name)
    path = PurePosixPath(raw_path)
    if (
        "\\" in raw_path
        or path.is_absolute()
        or raw_path != path.as_posix()
        or any(part in {".", ".."} for part in path.parts)
    ):
        raise ValueError(f"{name} must be a portable bundle-relative path")
    return raw_path


def validate_evidence_reference(value: Any, name: str) -> dict[str, str]:
    reference = require_mapping(value, name)
    require_exact_keys(reference, name, {"path", "sha256"})
    return {
        "path": require_bundle_relative_path(reference.get("path"), f"{name}.path"),
        "sha256": require_sha256(reference.get("sha256"), f"{name}.sha256"),
    }


def validate_backend_capabilities(value: Any) -> dict[str, Any]:
    capabilities = require_mapping(value, "preregistration backend_evidence.capabilities")
    require_exact_keys(
        capabilities,
        "preregistration backend_evidence.capabilities",
        {
            "protocol",
            "model",
            "context_window",
            "supports_functions",
            "supports_vision",
            "supports_thinking",
            "supports_structured_outputs",
            "source",
        },
    )
    source = require_identifier(
        capabilities.get("source"), "preregistration backend_evidence.capabilities.source"
    )
    if source != "registered-backend":
        raise ValueError(
            "preregistration backend_evidence.capabilities.source must be 'registered-backend'"
        )
    return {
        "protocol": require_identifier(
            capabilities.get("protocol"),
            "preregistration backend_evidence.capabilities.protocol",
        ),
        "model": require_identifier(
            capabilities.get("model"), "preregistration backend_evidence.capabilities.model"
        ),
        "context_window": require_non_negative_int(
            capabilities.get("context_window"),
            "preregistration backend_evidence.capabilities.context_window",
        ),
        "supports_functions": require_bool(
            capabilities.get("supports_functions"),
            "preregistration backend_evidence.capabilities.supports_functions",
        ),
        "supports_vision": require_bool(
            capabilities.get("supports_vision"),
            "preregistration backend_evidence.capabilities.supports_vision",
        ),
        "supports_thinking": require_bool(
            capabilities.get("supports_thinking"),
            "preregistration backend_evidence.capabilities.supports_thinking",
        ),
        "supports_structured_outputs": require_bool(
            capabilities.get("supports_structured_outputs"),
            "preregistration backend_evidence.capabilities.supports_structured_outputs",
        ),
        "source": source,
    }


def validate_backend_evidence(value: Any) -> dict[str, Any]:
    backend_evidence = require_mapping(value, "preregistration backend_evidence")
    kind = backend_evidence.get("kind")
    if kind not in BACKEND_MEASUREMENT_SOURCES:
        raise ValueError(
            "preregistration backend_evidence.kind must be one of "
            + ", ".join(sorted(BACKEND_MEASUREMENT_SOURCES))
        )
    expected_keys = {"kind", "backend", "measurement_source"}
    if kind == "observed-backend-run":
        expected_keys.add("capabilities")
    require_exact_keys(
        backend_evidence,
        "preregistration backend_evidence",
        expected_keys,
    )
    measurement_source = backend_evidence.get("measurement_source")
    expected_measurement_source = BACKEND_MEASUREMENT_SOURCES[kind]
    if measurement_source != expected_measurement_source:
        raise ValueError(
            "preregistration backend_evidence "
            f"kind {kind!r} must declare measurement_source {expected_measurement_source!r}"
        )
    backend = require_mapping(
        backend_evidence.get("backend"), "preregistration backend_evidence.backend"
    )
    require_exact_keys(
        backend,
        "preregistration backend_evidence.backend",
        {"id", "revision"},
    )
    validated = {
        "kind": kind,
        "backend": {
            "id": require_identifier(
                backend.get("id"), "preregistration backend_evidence.backend.id"
            ),
            "revision": require_identifier(
                backend.get("revision"), "preregistration backend_evidence.backend.revision"
            ),
        },
        "measurement_source": measurement_source,
    }
    if kind == "observed-backend-run":
        validated["capabilities"] = validate_backend_capabilities(
            backend_evidence.get("capabilities")
        )
    return validated


def validate_provenance(value: Any) -> dict[str, str]:
    provenance = require_mapping(value, "preregistration provenance")
    require_exact_keys(
        provenance,
        "preregistration provenance",
        {"repository", "revision", "bundle_id"},
    )
    return {
        "repository": require_repository_id(
            provenance.get("repository"), "preregistration provenance.repository"
        ),
        "revision": require_revision(
            provenance.get("revision"), "preregistration provenance.revision"
        ),
        "bundle_id": require_identifier(
            provenance.get("bundle_id"), "preregistration provenance.bundle_id"
        ),
    }


def validate_preregistration(preregistration: dict[str, Any]) -> dict[str, Any]:
    if preregistration.get("schema_version") != EVIDENCE_SCHEMA_VERSION:
        raise ValueError(f"preregistration schema_version must be {EVIDENCE_SCHEMA_VERSION}")

    scenario = require_identifier(preregistration.get("scenario"), "preregistration scenario")
    metric = preregistration.get("metric")
    if metric not in SUPPORTED_METRICS:
        raise ValueError(
            "preregistration metric must be one of " + ", ".join(sorted(SUPPORTED_METRICS))
        )

    dataset = require_mapping(preregistration.get("dataset"), "preregistration dataset")
    require_non_empty_string(dataset.get("id"), "preregistration dataset.id")
    require_non_empty_string(dataset.get("revision"), "preregistration dataset.revision")

    split_policy = require_mapping(
        preregistration.get("split_policy"), "preregistration split_policy"
    )
    if not split_policy:
        raise ValueError("preregistration split_policy must not be empty")

    splits = require_mapping(preregistration.get("splits"), "preregistration splits")
    validated_splits: dict[str, dict[str, Any]] = {}
    for split_name in ("optimization", "held_out"):
        split = require_mapping(splits.get(split_name), f"preregistration splits.{split_name}")
        validated_splits[split_name] = {
            "sha256": require_sha256(
                split.get("sha256"), f"preregistration splits.{split_name}.sha256"
            ),
            "case_count": require_non_negative_int(
                split.get("case_count"), f"preregistration splits.{split_name}.case_count"
            ),
        }
        if validated_splits[split_name]["case_count"] == 0:
            raise ValueError(f"preregistration splits.{split_name}.case_count must be positive")

    arms = require_mapping(preregistration.get("arms"), "preregistration arms")
    validated_arms: dict[str, dict[str, str]] = {}
    for arm_name in ("baseline", "candidate"):
        arm = require_mapping(arms.get(arm_name), f"preregistration arms.{arm_name}")
        validated_arms[arm_name] = {
            "id": require_non_empty_string(arm.get("id"), f"preregistration arms.{arm_name}.id")
        }
    if validated_arms["baseline"]["id"] == validated_arms["candidate"]["id"]:
        raise ValueError("preregistration baseline and candidate identities must differ")

    backend_evidence = validate_backend_evidence(preregistration.get("backend_evidence"))
    provenance = validate_provenance(preregistration.get("provenance"))

    decision_rule = require_mapping(
        preregistration.get("decision_rule"), "preregistration decision_rule"
    )
    validated_rule = {
        "minimum_candidate_mean": require_number(
            decision_rule.get("minimum_candidate_mean"),
            "preregistration decision_rule.minimum_candidate_mean",
            0.0,
            1.0,
        ),
        "minimum_delta": require_number(
            decision_rule.get("minimum_delta"),
            "preregistration decision_rule.minimum_delta",
            -1.0,
            1.0,
        ),
    }

    return {
        "scenario": scenario,
        "metric": metric,
        "dataset": dataset,
        "split_policy": split_policy,
        "splits": validated_splits,
        "arms": validated_arms,
        "backend_evidence": backend_evidence,
        "provenance": provenance,
        "decision_rule": validated_rule,
    }


def load_evidence_receipt(
    reference: dict[str, str], bundle_root: Path, name: str
) -> dict[str, Any]:
    path = bundle_root / reference["path"]
    portable_path(path, bundle_root, name)
    if not path.is_file():
        raise ValueError(f"{name} must reference an existing file")
    if sha256_file(path) != reference["sha256"]:
        raise ValueError(f"{name} digest does not match referenced file")
    return load_json(path)


def require_utc_timestamp(value: Any, name: str) -> str:
    timestamp = require_non_empty_string(value, name)
    if not timestamp.endswith("Z"):
        raise ValueError(f"{name} must be a UTC timestamp ending in Z")
    try:
        dt.datetime.fromisoformat(timestamp[:-1] + "+00:00")
    except ValueError as error:
        raise ValueError(f"{name} must be an ISO-8601 UTC timestamp") from error
    return timestamp


def validate_observed_backend_execution(
    value: Any,
    bundle_root: Path,
    backend_evidence: dict[str, Any],
    case_id: str,
    arm_id: str,
    receipt: dict[str, Any],
    label: str,
) -> dict[str, str]:
    reference = validate_evidence_reference(
        value, f"{label} execution evidence for {case_id!r}"
    )
    execution = load_evidence_receipt(
        reference,
        bundle_root,
        f"{label} execution evidence for {case_id!r}",
    )
    require_exact_keys(
        execution,
        f"{label} execution evidence for {case_id!r}",
        {
            "schema_version",
            "kind",
            "backend",
            "capabilities_sha256",
            "case_id",
            "arm_id",
            "request",
            "response",
            "provider_response_id",
            "provider_model",
            "output_sha256",
            "token_count",
            "latency_ms",
            "observed_at_utc",
        },
    )
    if execution.get("schema_version") != OBSERVED_BACKEND_EXECUTION_SCHEMA_VERSION:
        raise ValueError(
            f"{label} execution evidence for {case_id!r} schema_version must be "
            f"{OBSERVED_BACKEND_EXECUTION_SCHEMA_VERSION}"
        )
    if execution.get("kind") != "recorded-backend-execution":
        raise ValueError(
            f"{label} execution evidence for {case_id!r} kind must be recorded-backend-execution"
        )
    if execution.get("backend") != backend_evidence["backend"]:
        raise ValueError(
            f"{label} execution evidence for {case_id!r} does not match preregistered backend"
        )
    if require_sha256(
        execution.get("capabilities_sha256"),
        f"{label} execution evidence for {case_id!r}.capabilities_sha256",
    ) != sha256_json(backend_evidence["capabilities"]):
        raise ValueError(
            f"{label} execution evidence for {case_id!r} does not match backend capabilities"
        )
    if execution.get("case_id") != case_id or execution.get("arm_id") != arm_id:
        raise ValueError(
            f"{label} execution evidence for {case_id!r} does not match case or arm identity"
        )
    request_reference = validate_evidence_reference(
        execution.get("request"), f"{label} request evidence for {case_id!r}"
    )
    response_reference = validate_evidence_reference(
        execution.get("response"), f"{label} response evidence for {case_id!r}"
    )
    load_evidence_receipt(
        request_reference, bundle_root, f"{label} request evidence for {case_id!r}"
    )
    load_evidence_receipt(
        response_reference, bundle_root, f"{label} response evidence for {case_id!r}"
    )
    if request_reference["sha256"] != receipt["request_sha256"]:
        raise ValueError(
            f"{label} execution evidence for {case_id!r} does not match the receipt request"
        )
    for field in ("output_sha256", "token_count", "latency_ms"):
        if execution.get(field) != receipt[field]:
            raise ValueError(
                f"{label} execution evidence for {case_id!r} does not match receipt {field}"
            )
    require_identifier(
        execution.get("provider_response_id"),
        f"{label} execution evidence for {case_id!r}.provider_response_id",
    )
    require_identifier(
        execution.get("provider_model"),
        f"{label} execution evidence for {case_id!r}.provider_model",
    )
    require_utc_timestamp(
        execution.get("observed_at_utc"),
        f"{label} execution evidence for {case_id!r}.observed_at_utc",
    )
    return reference


def validate_observed_backend_receipt(
    value: Any,
    bundle_root: Path,
    backend_evidence: dict[str, Any],
    case_id: str,
    arm_id: str,
    row: dict[str, Any],
    label: str,
) -> dict[str, str]:
    reference = validate_evidence_reference(value, f"{label} evidence for {case_id!r}")
    receipt = load_evidence_receipt(reference, bundle_root, f"{label} evidence for {case_id!r}")
    require_exact_keys(
        receipt,
        f"{label} evidence receipt for {case_id!r}",
        {
            "schema_version",
            "kind",
            "backend",
            "case_id",
            "arm_id",
            "request_sha256",
            "output_sha256",
            "token_count",
            "latency_ms",
            "execution_evidence",
        },
    )
    if receipt.get("schema_version") != OBSERVED_BACKEND_RECEIPT_SCHEMA_VERSION:
        raise ValueError(
            f"{label} evidence receipt for {case_id!r} schema_version must be "
            f"{OBSERVED_BACKEND_RECEIPT_SCHEMA_VERSION}"
        )
    if receipt.get("kind") != "observed-backend-receipt":
        raise ValueError(f"{label} evidence receipt for {case_id!r} kind must be observed-backend-receipt")
    receipt_backend = require_mapping(
        receipt.get("backend"), f"{label} evidence receipt for {case_id!r}.backend"
    )
    require_exact_keys(
        receipt_backend,
        f"{label} evidence receipt for {case_id!r}.backend",
        {"id", "revision"},
    )
    if {
        "id": require_identifier(
            receipt_backend.get("id"), f"{label} evidence receipt for {case_id!r}.backend.id"
        ),
        "revision": require_identifier(
            receipt_backend.get("revision"),
            f"{label} evidence receipt for {case_id!r}.backend.revision",
        ),
    } != backend_evidence["backend"]:
        raise ValueError(f"{label} evidence receipt for {case_id!r} does not match preregistered backend")
    if receipt.get("case_id") != case_id:
        raise ValueError(f"{label} evidence receipt for {case_id!r} does not match case_id")
    if receipt.get("arm_id") != arm_id:
        raise ValueError(f"{label} evidence receipt for {case_id!r} does not match arm_id")
    require_sha256(
        receipt.get("request_sha256"),
        f"{label} evidence receipt for {case_id!r}.request_sha256",
    )
    if require_sha256(
        receipt.get("output_sha256"), f"{label} evidence receipt for {case_id!r}.output_sha256"
    ) != sha256_text(row["output"]):
        raise ValueError(f"{label} evidence receipt for {case_id!r} does not match output")
    if require_non_negative_int(
        receipt.get("token_count"), f"{label} evidence receipt for {case_id!r}.token_count"
    ) != row["token_count"]:
        raise ValueError(f"{label} evidence receipt for {case_id!r} does not match token_count")
    if require_number(
        receipt.get("latency_ms"),
        f"{label} evidence receipt for {case_id!r}.latency_ms",
        0.0,
        math.inf,
    ) != float(row["latency_ms"]):
        raise ValueError(f"{label} evidence receipt for {case_id!r} does not match latency_ms")
    validate_observed_backend_execution(
        receipt.get("execution_evidence"),
        bundle_root,
        backend_evidence,
        case_id,
        arm_id,
        receipt,
        label,
    )
    return reference


def validate_output_rows(
    rows: dict[str, dict[str, Any]],
    arm_id: str,
    label: str,
    backend_evidence: dict[str, Any],
    bundle_root: Path,
) -> dict[str, dict[str, str]]:
    expected_fields = (
        OBSERVED_OUTPUT_FIELDS
        if backend_evidence["kind"] == "observed-backend-run"
        else BASE_OUTPUT_FIELDS
    )
    evidence_references: dict[str, dict[str, str]] = {}
    for case_id, row in rows.items():
        require_exact_keys(row, f"{label} output for {case_id!r}", set(expected_fields))
        require_non_empty_string(row.get("output"), f"{label} output for {case_id!r}")
        if row.get("arm_id") != arm_id:
            raise ValueError(f"{label} output for {case_id!r} does not match preregistered arm_id")
        require_non_negative_int(row.get("token_count"), f"{label} token_count for {case_id!r}")
        require_number(row.get("latency_ms"), f"{label} latency_ms for {case_id!r}", 0.0, math.inf)
        if backend_evidence["kind"] == "observed-backend-run":
            evidence_references[case_id] = validate_observed_backend_receipt(
                row.get("evidence"),
                bundle_root,
                backend_evidence,
                case_id,
                arm_id,
                row,
                label,
            )
    return evidence_references


def score(metric: str, expected: str, output: str) -> float:
    if metric == "exact_match":
        return float(output == expected)
    if metric == "contains":
        return float(expected in output)
    expected_tokens = set(expected.split())
    output_tokens = set(output.split())
    if not expected_tokens and not output_tokens:
        return 1.0
    if not expected_tokens or not output_tokens:
        return 0.0
    return len(expected_tokens & output_tokens) / len(expected_tokens | output_tokens)


def utc_run_id() -> str:
    return dt.datetime.now(dt.UTC).strftime("%Y%m%dT%H%M%SZ")


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def portable_path(path: Path, root: Path, label: str) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError as error:
        raise ValueError(f"{label} must be inside bundle root {root}") from error


def file_reference(path: Path, bundle_root: Path, label: str) -> dict[str, str]:
    return {"path": portable_path(path, bundle_root, label), "sha256": sha256_file(path)}


def observation_summary(rows: dict[str, dict[str, Any]]) -> dict[str, float | int]:
    token_counts = [int(row["token_count"]) for row in rows.values()]
    latencies = [float(row["latency_ms"]) for row in rows.values()]
    return {
        "total_token_count": sum(token_counts),
        "mean_token_count": sum(token_counts) / len(token_counts),
        "total_latency_ms": sum(latencies),
        "mean_latency_ms": sum(latencies) / len(latencies),
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--bundle-root", type=Path)
    parser.add_argument("--optimization", type=Path, required=True)
    parser.add_argument("--held-out", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--run-id", help="Optional reproducible run identifier")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    bundle_root = (args.bundle_root or args.preregistration.parent).resolve()
    if not bundle_root.is_dir():
        raise ValueError(f"bundle root must be an existing directory: {bundle_root}")

    preregistration = validate_preregistration(load_json(args.preregistration))
    for path, label in (
        (args.preregistration, "preregistration"),
        (args.optimization, "optimization input"),
        (args.held_out, "held-out input"),
        (args.baseline, "baseline output"),
        (args.candidate, "candidate output"),
    ):
        portable_path(path, bundle_root, label)

    optimization_digest = sha256_file(args.optimization)
    held_out_digest = sha256_file(args.held_out)
    if optimization_digest != preregistration["splits"]["optimization"]["sha256"]:
        raise ValueError("optimization input digest does not match preregistration")
    if held_out_digest != preregistration["splits"]["held_out"]["sha256"]:
        raise ValueError("held-out input digest does not match preregistration")

    optimization = load_jsonl(args.optimization, {"id"})
    held_out = load_jsonl(args.held_out, {"id", "expected"})
    if len(optimization) != preregistration["splits"]["optimization"]["case_count"]:
        raise ValueError("optimization input case count does not match preregistration")
    if len(held_out) != preregistration["splits"]["held_out"]["case_count"]:
        raise ValueError("held-out input case count does not match preregistration")
    overlap = sorted(set(optimization) & set(held_out))
    if overlap:
        raise ValueError("optimization and held-out case ids must be disjoint")
    for case_id, row in held_out.items():
        require_non_empty_string(row.get("expected"), f"held-out expected value for {case_id!r}")

    baseline = load_jsonl(args.baseline, {"id"})
    candidate = load_jsonl(args.candidate, {"id"})
    expected_ids = set(held_out)
    if set(baseline) != expected_ids or set(candidate) != expected_ids:
        raise ValueError("baseline and candidate ids must exactly match the held-out set")
    baseline_receipts = validate_output_rows(
        baseline,
        preregistration["arms"]["baseline"]["id"],
        "baseline",
        preregistration["backend_evidence"],
        bundle_root,
    )
    candidate_receipts = validate_output_rows(
        candidate,
        preregistration["arms"]["candidate"]["id"],
        "candidate",
        preregistration["backend_evidence"],
        bundle_root,
    )

    scores = [
        CaseScore(
            case_id=case_id,
            baseline=score(
                preregistration["metric"],
                held_out[case_id]["expected"],
                baseline[case_id]["output"],
            ),
            candidate=score(
                preregistration["metric"],
                held_out[case_id]["expected"],
                candidate[case_id]["output"],
            ),
            baseline_token_count=baseline[case_id]["token_count"],
            candidate_token_count=candidate[case_id]["token_count"],
            baseline_latency_ms=float(baseline[case_id]["latency_ms"]),
            candidate_latency_ms=float(candidate[case_id]["latency_ms"]),
        )
        for case_id in sorted(expected_ids)
    ]
    baseline_mean = sum(item.baseline for item in scores) / len(scores)
    candidate_mean = sum(item.candidate for item in scores) / len(scores)
    delta = candidate_mean - baseline_mean
    decision_rule = preregistration["decision_rule"]
    accepted = (
        candidate_mean >= decision_rule["minimum_candidate_mean"]
        and delta >= decision_rule["minimum_delta"]
    )

    layout = build_layout(__file__)
    working_tree = collect_working_tree_provenance(layout.repo_root)
    if working_tree["head_revision"] != preregistration["provenance"]["revision"]:
        raise ValueError(
            "preregistration provenance.revision does not match the evaluated working-tree HEAD"
        )
    run_id = args.run_id or utc_run_id()
    require_identifier(run_id, "run id")
    run_dir = layout.evaluation_dir / preregistration["scenario"] / "runs" / run_id
    if run_dir.exists():
        raise ValueError(f"evaluation run directory already exists: {run_dir}")
    run_dir.mkdir(parents=True)
    manifest = {
        "schema_version": EVIDENCE_SCHEMA_VERSION,
        "scenario": preregistration["scenario"],
        "dataset": preregistration["dataset"],
        "split_policy": preregistration["split_policy"],
        "splits": preregistration["splits"],
        "arms": preregistration["arms"],
        "metric": preregistration["metric"],
        "decision_rule": decision_rule,
        "backend_evidence": preregistration["backend_evidence"],
        "provenance": {
            "declared": preregistration["provenance"],
            "preregistration": file_reference(args.preregistration, bundle_root, "preregistration"),
            "runner_sha256": sha256_file(Path(__file__)),
            "working_tree": working_tree,
        },
        "inputs": {
            "optimization": file_reference(args.optimization, bundle_root, "optimization input"),
            "held_out": file_reference(args.held_out, bundle_root, "held-out input"),
            "baseline": file_reference(args.baseline, bundle_root, "baseline output"),
            "candidate": file_reference(args.candidate, bundle_root, "candidate output"),
        },
        "case_count": len(scores),
    }
    if preregistration["backend_evidence"]["kind"] == "observed-backend-run":
        manifest["observed_backend_receipts"] = {
            "baseline": [
                {"case_id": case_id, "evidence": baseline_receipts[case_id]}
                for case_id in sorted(baseline_receipts)
            ],
            "candidate": [
                {"case_id": case_id, "evidence": candidate_receipts[case_id]}
                for case_id in sorted(candidate_receipts)
            ],
        }
    summary = {
        "baseline_mean": baseline_mean,
        "candidate_mean": candidate_mean,
        "delta": delta,
        "decision": {
            "accepted": accepted,
            "minimum_candidate_mean": decision_rule["minimum_candidate_mean"],
            "minimum_delta": decision_rule["minimum_delta"],
        },
        "measurements": {
            "source": preregistration["backend_evidence"]["measurement_source"],
            "is_backend_telemetry": (
                preregistration["backend_evidence"]["kind"] == "observed-backend-run"
            ),
            "baseline": observation_summary(baseline),
            "candidate": observation_summary(candidate),
        },
        "scores": [asdict(item) for item in scores],
    }
    write_json(run_dir / "manifest.json", manifest)
    write_json(run_dir / "summary.json", summary)
    print(
        json.dumps(
            {
                "run_dir": portable_path(run_dir, layout.repo_root, "evaluation run directory"),
                "baseline_mean": baseline_mean,
                "candidate_mean": candidate_mean,
                "accepted": accepted,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"offline prompt evaluation failed: {error}", file=sys.stderr)
        raise SystemExit(2)
