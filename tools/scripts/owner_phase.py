"""Run one APXM owner phase and emit its final neutral result."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
from typing import Any, Sequence


ROOT = Path(__file__).resolve().parents[2]
PHASES: dict[str, tuple[str, ...]] = {
    "owner-e2e": ("release-qualification", "test-compilation-protocol", "test-runtime-protocol"),
    "owner-compilation-runtime": ("release-qualification", "compile-service-canonical", "execute-canonical"),
    "owner-negative-recovery": ("release-qualification", "test-kernel", "test-runtime-protocol", "test-runtime-service"),
    "owner-integrated-execution": ("release-qualification", "test-compilation-protocol", "test-runtime-protocol", "compile-service-canonical", "execute-canonical"),
    "owner-protocol-clients": ("release-qualification", "test-compilation-service", "test-runtime-service", "test-event-http", "test-interaction-client"),
    "owner-restart-reopen": ("release-qualification", "test-runtime-protocol", "test-runtime-service", "test-kernel", "test-execution"),
}
SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
# The release manifest binds the Python frontend bridge alongside the two
# service executables, so the qualification result carries three artifact
# digests and only two manifest services.
SERVICE_NAMES = frozenset({"compilation-service", "runtime-service"})
FRONTEND_NATIVE = "python-frontend-native"
BOUND_ARTIFACTS = SERVICE_NAMES | {FRONTEND_NATIVE}
REVISION = re.compile(r"^[0-9a-f]{40}$")
QUALIFICATION_SCHEMA = "apxm.agents.release-qualification.v1"
COMMAND_EVIDENCE_SCHEMA = "apxm.agents.owner-command-evidence.v1"
COMMAND_ARTIFACT_SCHEMA = "apxm.agents.owner-command-execution.v1"
COMMAND_EVIDENCE_ROOT = Path(".apxm") / "owner-command-evidence"
OUTPUT_FRAME_MAGIC = b"apxm.owner-command-output.v1\0"
OUTPUT_FRAME_LENGTH_BYTES = 8


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
    if not isinstance(services, dict) or set(services) != BOUND_ARTIFACTS:
        raise ValueError(
            "owner release qualification did not bind exactly the two APXM services and the frontend bridge"
        )
    if any(not isinstance(digest, str) or not SHA256.fullmatch(digest) for digest in services.values()):
        raise ValueError("owner release qualification contains an invalid service digest")
    frontend_native = value.get("manifest_frontend_native")
    if (
        not isinstance(frontend_native, dict)
        or set(frontend_native) != {"name", "path", "digest"}
        or frontend_native["name"] != FRONTEND_NATIVE
        or frontend_native["digest"] != services[FRONTEND_NATIVE]
        or not isinstance(frontend_native["path"], str)
        or not frontend_native["path"]
        or Path(frontend_native["path"]).is_absolute()
        or ".." in Path(frontend_native["path"]).parts
    ):
        raise ValueError("owner release qualification frontend bridge binding does not match the result digests")
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
    if seen != SERVICE_NAMES:
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


def _canonical_json_bytes(value: object) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
        allow_nan=False,
    ).encode("utf-8")


def _frame_command_output(stdout: bytes, stderr: bytes) -> bytes:
    return (
        OUTPUT_FRAME_MAGIC
        + len(stdout).to_bytes(OUTPUT_FRAME_LENGTH_BYTES, "big")
        + stdout
        + len(stderr).to_bytes(OUTPUT_FRAME_LENGTH_BYTES, "big")
        + stderr
    )


def _print_command_stream(payload: bytes, *, stderr: bool) -> None:
    stream = sys.stderr if stderr else sys.stdout
    buffer = getattr(stream, "buffer", None)
    if buffer is not None:
        buffer.write(payload)
        buffer.flush()
    else:
        stream.write(payload.decode("utf-8", errors="replace"))
        stream.flush()


def _ensure_directory(path: Path, *, label: str) -> None:
    if path.is_symlink():
        raise ValueError(f"{label} is a symlink")
    if path.exists():
        if not path.is_dir():
            raise ValueError(f"{label} is not a directory")
        return
    path.mkdir()
    if path.is_symlink() or not path.is_dir():
        raise ValueError(f"{label} could not be created safely")


def _evidence_parent(root: Path, phase: str) -> None:
    owner_root = root.resolve(strict=True)
    if not owner_root.is_dir():
        raise ValueError("owner checkout root is not a directory")
    apxm_root = owner_root / ".apxm"
    _ensure_directory(apxm_root, label=".apxm root")
    evidence_root = apxm_root / COMMAND_EVIDENCE_ROOT.name
    _ensure_directory(evidence_root, label="command evidence root")
    phase_root = evidence_root / phase
    _ensure_directory(phase_root, label="command evidence phase root")


def _validate_parent_chain(root: Path, parent: Path) -> None:
    owner_root = root.resolve(strict=True)
    try:
        relative = parent.relative_to(owner_root)
    except ValueError as error:
        raise ValueError("command artifact parent escapes owner checkout") from error
    current = owner_root
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise ValueError("command artifact parent is a symlink")
        if not current.is_dir():
            raise ValueError("command artifact parent is not a directory")


def _atomic_write(root: Path, relative: Path, payload: bytes) -> None:
    owner_root = root.resolve(strict=True)
    target = owner_root / relative
    parent = target.parent
    _validate_parent_chain(owner_root, parent)
    if target.is_symlink() or (target.exists() and not target.is_file()):
        raise ValueError("command artifact target is not a regular file")
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{target.name}.", suffix=".tmp", dir=parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, target)
    finally:
        if temporary.exists() or temporary.is_symlink():
            temporary.unlink()


def _existing_artifact(root: Path, reference: str) -> Path:
    owner_root = root.resolve(strict=True)
    candidate = owner_root / reference
    _validate_parent_chain(owner_root, candidate.parent)
    if candidate.is_symlink() or not candidate.is_file():
        raise ValueError("command artifact is unavailable")
    resolved = candidate.resolve(strict=True)
    try:
        resolved.relative_to(owner_root)
    except ValueError as error:
        raise ValueError("command artifact escapes owner checkout") from error
    return resolved


def _command_evidence_entry(
    phase: str,
    command: str,
    output: bytes,
    ordinal: int,
    *,
    root: Path = ROOT,
) -> dict[str, Any]:
    _evidence_parent(root, phase)
    output_relative = COMMAND_EVIDENCE_ROOT / phase / f"{ordinal:03d}.output"
    _atomic_write(root, output_relative, output)
    output_digest = "sha256:" + hashlib.sha256(output).hexdigest()
    output_artifact = {
        "reference": output_relative.as_posix(),
        "digest": output_digest,
    }
    artifact_document = {
        "schema": COMMAND_ARTIFACT_SCHEMA,
        "owner": "agents",
        "command": command,
        "status": "passed",
        "output_digest": output_digest,
        "output_artifact": output_artifact,
    }
    artifact_bytes = _canonical_json_bytes(artifact_document)
    artifact_relative = COMMAND_EVIDENCE_ROOT / phase / f"{ordinal:03d}.json"
    _atomic_write(root, artifact_relative, artifact_bytes)
    return {
        "command": command,
        "status": "passed",
        "output_digest": output_digest,
        "output_artifact": output_artifact,
        "evidence_artifact": {
            "reference": artifact_relative.as_posix(),
            "digest": "sha256:" + hashlib.sha256(artifact_bytes).hexdigest(),
        },
    }


def _strict_command_evidence(
    value: object,
    *,
    phase: str,
    root: Path = ROOT,
) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {
        "schema", "owner", "phase", "status", "commands"
    }:
        raise ValueError("owner command evidence schema is invalid")
    if (
        value.get("schema") != COMMAND_EVIDENCE_SCHEMA
        or value.get("owner") != "agents"
        or value.get("phase") != phase
        or value.get("status") != "passed"
    ):
        raise ValueError("owner command evidence identity or status is invalid")
    commands = value.get("commands")
    if not isinstance(commands, list) or not commands:
        raise ValueError("owner command evidence has no successful commands")
    expected_commands = {
        f"dekk agents {command}" for command in PHASES[phase]
    }
    observed_commands = {
        entry.get("command")
        for entry in commands
        if isinstance(entry, dict) and isinstance(entry.get("command"), str)
    }
    if observed_commands != expected_commands or len(commands) != len(observed_commands):
        raise ValueError("owner command evidence does not cover the declared phase commands")
    for entry in commands:
        if not isinstance(entry, dict) or set(entry) != {
            "command", "status", "output_digest", "output_artifact", "evidence_artifact"
        }:
            raise ValueError("owner command evidence entry schema is invalid")
        command = entry.get("command")
        output_digest = entry.get("output_digest")
        output_artifact = entry.get("output_artifact")
        artifact = entry.get("evidence_artifact")
        if (
            not isinstance(command, str)
            or not command.startswith("dekk agents ")
            or entry.get("status") != "passed"
            or not isinstance(output_digest, str)
            or not SHA256.fullmatch(output_digest)
            or not isinstance(output_artifact, dict)
            or set(output_artifact) != {"reference", "digest"}
            or not isinstance(output_artifact.get("reference"), str)
            or not output_artifact["reference"].startswith(".apxm/owner-command-evidence/")
            or not isinstance(output_artifact.get("digest"), str)
            or not SHA256.fullmatch(output_artifact["digest"])
            or not isinstance(artifact, dict)
            or set(artifact) != {"reference", "digest"}
            or not isinstance(artifact.get("reference"), str)
            or not artifact["reference"].startswith(".apxm/owner-command-evidence/")
            or not isinstance(artifact.get("digest"), str)
            or not SHA256.fullmatch(artifact["digest"])
        ):
            raise ValueError("owner command evidence entry is invalid")
        output_reference = str(output_artifact["reference"])
        try:
            output_path = _existing_artifact(root, output_reference)
        except ValueError as error:
            raise ValueError(f"owner command output artifact is unavailable: {error}") from error
        try:
            output_bytes = output_path.read_bytes()
        except OSError as error:
            raise ValueError("owner command output artifact is unreadable") from error
        if (
            "sha256:" + hashlib.sha256(output_bytes).hexdigest() != output_digest
            or output_artifact["digest"] != output_digest
        ):
            raise ValueError("owner command output artifact is not bound")
        reference = str(artifact["reference"])
        try:
            artifact_path = _existing_artifact(root, reference)
        except ValueError as error:
            raise ValueError(f"owner command evidence artifact is unavailable: {error}") from error
        try:
            raw = artifact_path.read_bytes()
            document = json.loads(raw.decode("utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError("owner command evidence artifact is not canonical JSON") from error
        if (
            raw != _canonical_json_bytes(document)
            or "sha256:" + hashlib.sha256(raw).hexdigest() != artifact["digest"]
            or document != {
                "schema": COMMAND_ARTIFACT_SCHEMA,
                "owner": "agents",
                "command": command,
                "status": "passed",
                "output_digest": output_digest,
                "output_artifact": output_artifact,
            }
        ):
            raise ValueError("owner command evidence artifact is not bound")
    return value


def final_result(
    phase: str,
    *,
    qualification: dict[str, Any],
    command_evidence: dict[str, Any] | None = None,
    root: Path = ROOT,
) -> dict[str, Any]:
    result: dict[str, Any] = {
        "schema": "apxm.agents.owner-phase-result.v1",
        "owner": "agents",
        "phase": phase,
        "status": "passed",
        "qualification": qualification,
    }
    if command_evidence is not None:
        result["command_evidence"] = _strict_command_evidence(
            command_evidence, phase=phase, root=root
        )
    result["qualification_digest"] = "sha256:" + hashlib.sha256(
        _canonical_json_bytes(qualification)
    ).hexdigest()
    return result


def run_phase(phase: str, commands: Sequence[str] | None = None) -> int:
    if phase not in PHASES:
        raise ValueError(f"unknown APXM owner phase: {phase}")
    dekk = shutil.which("dekk")
    if not dekk:
        raise RuntimeError("dekk executable is unavailable")
    qualification: dict[str, Any] | None = None
    command_evidence: list[dict[str, Any]] = []
    for ordinal, command in enumerate(commands or PHASES[phase]):
        completed = subprocess.run(
            [dekk, "agents", command], cwd=ROOT, check=False, capture_output=True, text=False
        )
        stdout = completed.stdout or b""
        stderr = completed.stderr or b""
        if stdout:
            _print_command_stream(stdout, stderr=False)
        if stderr:
            _print_command_stream(stderr, stderr=True)
        if completed.returncode != 0:
            return completed.returncode
        if command == "release-qualification":
            try:
                qualification_output = stdout.decode("utf-8")
            except UnicodeDecodeError as error:
                raise ValueError("owner release qualification output is not UTF-8 JSON") from error
            qualification = _strict_qualification(_last_json_object(qualification_output))
        command_evidence.append(
            _command_evidence_entry(
                phase,
                f"dekk agents {command}",
                _frame_command_output(stdout, stderr),
                ordinal,
            )
        )
    if qualification is None:
        raise RuntimeError("owner phase completed without owner release qualification evidence")
    evidence = {
        "schema": COMMAND_EVIDENCE_SCHEMA,
        "owner": "agents",
        "phase": phase,
        "status": "passed",
        "commands": command_evidence,
    }
    print(
        json.dumps(
            final_result(
                phase,
                qualification=qualification,
                command_evidence=evidence,
            ),
            sort_keys=True,
            separators=(",", ":"),
        )
    )
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
