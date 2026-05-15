#!/usr/bin/env python3
"""Lint APXM claim reports against referenced evidence artifacts.

The linter accepts either a JSON evidence manifest or a Markdown claim report
containing a block of the form:

<!-- apxm-claim-evidence
{ ... JSON ... }
-->

Paths are resolved relative to the repository root unless absolute. Generated
CSV, service, log, and scheduler evidence should live under `.apxm`.
"""

from __future__ import annotations

import argparse
import csv
import glob
import json
import re
import sys
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[4]
EVIDENCE_BLOCK_RE = re.compile(
    r"<!--\s*apxm-claim-evidence\s*(\{.*?\})\s*-->",
    re.DOTALL,
)


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("claim", type=Path, help="Markdown claim report or JSON evidence manifest")
    parser.add_argument("--repo-root", type=Path, default=REPO_ROOT)
    return parser.parse_args()


def _load_manifest(path: Path) -> dict[str, Any]:
    text = path.read_text(encoding="utf-8")
    if path.suffix == ".json":
        data = json.loads(text)
    else:
        match = EVIDENCE_BLOCK_RE.search(text)
        if not match:
            raise ValueError(
                f"{path} has no apxm-claim-evidence JSON block"
            )
        data = json.loads(match.group(1))
    if not isinstance(data, dict):
        raise ValueError("claim evidence manifest must be a JSON object")
    return data


def _resolve(repo_root: Path, raw: str | Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else repo_root / path


def _paths(value: Any) -> list[str]:
    if value is None:
        return []
    if isinstance(value, str):
        return [value]
    if isinstance(value, list) and all(isinstance(item, str) for item in value):
        return list(value)
    raise ValueError(f"expected string or list of strings, got {value!r}")


def _require_existing_files(
    errors: list[str],
    repo_root: Path,
    label: str,
    values: Any,
) -> list[Path]:
    paths: list[Path] = []
    for raw in _paths(values):
        resolved = _resolve(repo_root, raw)
        if any(ch in raw for ch in "*?["):
            matches = [Path(p) for p in glob.glob(str(resolved))]
            if not matches:
                errors.append(f"{label}: no files matched {raw}")
            paths.extend(matches)
            continue
        if not resolved.is_file():
            errors.append(f"{label}: missing file {resolved}")
        else:
            paths.append(resolved)
    return paths


def _read_csv(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as fh:
        return list(csv.DictReader(fh))


def _lint_batch_csv(errors: list[str], path: Path) -> None:
    rows = _read_csv(path)
    if not rows:
        errors.append(f"csv: {path} has no rows")
        return
    for index, row in enumerate(rows, start=2):
        failed = int(row.get("failed_tenants", "0") or 0)
        if failed:
            errors.append(f"csv: {path}:{index} has failed_tenants={failed}")


def _lint_tenant_csv(errors: list[str], path: Path) -> None:
    rows = _read_csv(path)
    if not rows:
        errors.append(f"tenant_csv: {path} has no rows")
        return
    for index, row in enumerate(rows, start=2):
        returncode = int(row.get("returncode", "0") or 0)
        if returncode:
            errors.append(f"tenant_csv: {path}:{index} has returncode={returncode}")


def _load_json(errors: list[str], label: str, path: Path) -> dict[str, Any] | None:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        errors.append(f"{label}: cannot read JSON {path}: {exc}")
        return None
    if not isinstance(data, dict):
        errors.append(f"{label}: expected JSON object in {path}")
        return None
    return data


def _graph_capabilities(data: dict[str, Any]) -> dict[str, Any]:
    backends = data.get("backends")
    if isinstance(backends, dict):
        caps = backends.get("graph_capabilities")
        if isinstance(caps, dict):
            return caps
    caps = data.get("graph_capabilities")
    return caps if isinstance(caps, dict) else {}


def _lint_capabilities(
    errors: list[str],
    path: Path,
    required_scheduler_policy: str,
) -> None:
    data = _load_json(errors, "capability_json", path)
    if data is None:
        return
    caps = _graph_capabilities(data)
    if not caps:
        errors.append(f"capability_json: {path} has no graph_capabilities")
        return
    if required_scheduler_policy == "priority":
        if not any(
            isinstance(value, dict) and value.get("supports_priority") is True
            for value in caps.values()
        ):
            errors.append(
                f"capability_json: {path} has no backend with supports_priority=true"
            )


def _extract_scheduler_policy(data: dict[str, Any]) -> str | None:
    policy = data.get("policy")
    if isinstance(policy, str):
        return policy
    scheduler = data.get("scheduler")
    if isinstance(scheduler, dict) and isinstance(scheduler.get("policy"), str):
        return scheduler["policy"]
    return None


def _lint_scheduler(
    errors: list[str],
    path: Path,
    required_scheduler_policy: str,
) -> None:
    data = _load_json(errors, "scheduler_evidence", path)
    if data is None:
        return
    policy = _extract_scheduler_policy(data)
    if policy != required_scheduler_policy:
        errors.append(
            f"scheduler_evidence: {path} policy={policy!r}, expected {required_scheduler_policy!r}"
        )


def _lint_service_state(
    errors: list[str],
    path: Path,
    required_scheduler_policy: str,
) -> None:
    data = _load_json(errors, "service_state", path)
    if data is None:
        return
    policy = data.get("scheduling_policy")
    if policy != required_scheduler_policy:
        errors.append(
            f"service_state: {path} scheduling_policy={policy!r}, expected {required_scheduler_policy!r}"
        )
    for key in ("max_num_seqs", "model", "name"):
        if data.get(key) in (None, ""):
            errors.append(f"service_state: {path} missing {key}")


def lint_claim(path: Path, repo_root: Path) -> list[str]:
    manifest = _load_manifest(path)
    errors: list[str] = []
    required_policy = str(manifest.get("required_scheduler_policy", "priority"))

    csvs = _require_existing_files(errors, repo_root, "csv", manifest.get("csvs"))
    tenant_csvs = _require_existing_files(
        errors, repo_root, "tenant_csv", manifest.get("tenant_csvs")
    )
    capability_json = _require_existing_files(
        errors, repo_root, "capability_json", manifest.get("capability_json")
    )
    scheduler_evidence = _require_existing_files(
        errors, repo_root, "scheduler_evidence", manifest.get("scheduler_evidence")
    )
    service_state = _require_existing_files(
        errors, repo_root, "service_state", manifest.get("service_state")
    )
    logs = _require_existing_files(errors, repo_root, "logs", manifest.get("logs"))

    for path in csvs:
        _lint_batch_csv(errors, path)
    for path in tenant_csvs:
        _lint_tenant_csv(errors, path)
    for path in capability_json:
        _lint_capabilities(errors, path, required_policy)
    for path in scheduler_evidence:
        _lint_scheduler(errors, path, required_policy)
    for path in service_state:
        _lint_service_state(errors, path, required_policy)
    for path in logs:
        if path.stat().st_size == 0:
            errors.append(f"logs: {path} is empty")

    return errors


def main() -> int:
    args = _parse_args()
    try:
        errors = lint_claim(args.claim, args.repo_root.resolve())
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"claim lint failed: {exc}", file=sys.stderr)
        return 2
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"claim lint passed: {args.claim}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
