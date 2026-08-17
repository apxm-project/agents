"""Run one APXM owner P80 phase and emit its final neutral result."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
from typing import Any, Sequence


ROOT = Path(__file__).resolve().parents[2]
PHASES: dict[str, tuple[str, ...]] = {
    "p80-e2e": ("release-qualification", "test-compilation-protocol", "test-runtime-protocol"),
    "p80-journey-c": ("release-qualification", "compile-service-canonical", "execute-canonical"),
    "p80-negative-recovery": ("release-qualification", "test-kernel", "test-runtime-protocol", "test-runtime-service"),
    "p80-journey-g": ("release-qualification", "test-compilation-protocol", "test-runtime-protocol", "compile-service-canonical", "execute-canonical"),
    "p80-journey-h": ("release-qualification", "test-compilation-service", "test-runtime-service", "test-event-http", "test-interaction-client"),
    "p80-restart-reopen": ("release-qualification", "test-runtime-protocol", "test-runtime-service", "test-kernel", "test-execution"),
}
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
REVISION = re.compile(r"^[0-9a-f]{40}$")
QUALIFICATION_SCHEMA = "apxm.agents.release-qualification.v1"


def _last_json_object(output: str) -> dict[str, Any] | None:
    decoder = json.JSONDecoder()
    for offset, character in enumerate(output):
        if character != "{":
            continue
        try:
            value, end = decoder.raw_decode(output[offset:])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and not output[offset + end :].strip():
            return value
    return None


def _strict_qualification(value: object) -> dict[str, Any]:
    if not isinstance(value, dict) or value.get("schema") != QUALIFICATION_SCHEMA or value.get("owner") != "agents":
        raise ValueError("owner release qualification did not emit the APXM result schema")
    if value.get("qualified") is not True:
        raise ValueError("owner release qualification was not successful")
    if not REVISION.fullmatch(str(value.get("source_revision", ""))):
        raise ValueError("owner release qualification has no immutable source revision")
    if value.get("evidence_root") != str(ROOT.resolve()):
        raise ValueError("owner release qualification evidence root is not this owner checkout")
    services = value.get("service_digests")
    if not isinstance(services, dict) or set(services) != {"compilation-service", "runtime-service"}:
        raise ValueError("owner release qualification did not bind exactly the two APXM services")
    if any(not isinstance(digest, str) or not SHA256.fullmatch(digest) for digest in services.values()):
        raise ValueError("owner release qualification contains an invalid service digest")
    for field in ("source_descriptor_digest", "owner_descriptor_digest", "release_manifest_digest"):
        if not isinstance(value.get(field), str) or not SHA256.fullmatch(value[field]):
            raise ValueError(f"owner release qualification has no immutable {field}")
    manifest_services = value.get("manifest_services")
    if not isinstance(manifest_services, list) or len(manifest_services) != 2:
        raise ValueError("owner release qualification did not bind exactly two manifest services")
    seen: set[str] = set()
    for service in manifest_services:
        if not isinstance(service, dict) or set(service) != {"name", "path", "digest"}:
            raise ValueError("owner release qualification contains an invalid service manifest entry")
        name = service["name"]
        if name not in services or name in seen:
            raise ValueError("owner release qualification contains an unknown or duplicate service")
        seen.add(name)
        path = service["path"]
        if (
            service["digest"] != services[name]
            or not isinstance(path, str)
            or not path
            or Path(path).is_absolute()
            or ".." in Path(path).parts
        ):
            raise ValueError("owner release qualification service binding does not match the result digests")
    if seen != set(services):
        raise ValueError("owner release qualification is missing a service binding")
    expected_files = {
        "source_descriptor_digest": ROOT / "deploy/services/source-revision.v1.json",
        "owner_descriptor_digest": ROOT / "contracts/descriptors/apxm.agents-owner-descriptor.v1.json",
        "release_manifest_digest": ROOT / "contracts/services/manifests/apxm.agents-service-release-manifest.v1.json",
    }
    for field, path in expected_files.items():
        if not path.is_file() or value[field] != "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest():
            raise ValueError(f"owner release qualification {field} is not bound to owner bytes")
    gates = value.get("gates")
    expected = {
        "dekk agents check",
        "dekk agents test",
        "dekk agents test-compilation-protocol",
        "dekk agents test-compilation-service",
        "dekk agents test-runtime-protocol",
        "dekk agents test-runtime-service",
    }
    observed = {
        gate.get("command")
        for gate in gates
        if isinstance(gate, dict) and gate.get("returncode") == 0
    } if isinstance(gates, list) else set()
    if observed != expected:
        raise ValueError("owner release qualification did not prove the exact required gates")
    return value


def final_result(phase: str, *, qualification: dict[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = {
        "schema": "apxm.agents.owner-phase-result.v1",
        "owner": "agents",
        "phase": phase,
        "status": "passed",
        "qualification": qualification,
    }
    result["qualification_digest"] = "sha256:" + hashlib.sha256(
        json.dumps(qualification, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    return result


def run_phase(phase: str, commands: Sequence[str] | None = None) -> int:
    if phase not in PHASES:
        raise ValueError(f"unknown APXM P80 phase: {phase}")
    dekk = shutil.which("dekk")
    if not dekk:
        raise RuntimeError("dekk executable is unavailable")
    qualification: dict[str, Any] | None = None
    for command in commands or PHASES[phase]:
        completed = subprocess.run(
            [dekk, "agents", command], cwd=ROOT, check=False, capture_output=True, text=True
        )
        if completed.stdout:
            print(completed.stdout, end="")
        if completed.stderr:
            print(completed.stderr, end="", file=sys.stderr)
        if completed.returncode != 0:
            return completed.returncode
        if command == "release-qualification":
            qualification = _strict_qualification(_last_json_object(completed.stdout))
    if qualification is None:
        raise RuntimeError("P80 phase completed without owner release qualification evidence")
    print(json.dumps(final_result(phase, qualification=qualification), sort_keys=True, separators=(",", ":")))
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=tuple(PHASES))
    args = parser.parse_args(argv)
    try:
        return run_phase(args.phase)
    except (RuntimeError, ValueError) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
