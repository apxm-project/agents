"""Run the product-neutral APXM owner qualification and handoff gates.

The owner handoff is deliberately local to Agents.  It carries only APXM
source, descriptor, service, and command evidence identities; downstream
products translate this neutral result at their own boundary.
"""

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
    "owner-qualification": (
        "release-qualification",
        "test-compilation-runtime-failure-restart",
    ),
    "owner-failure-restart": (
        "release-qualification",
        "test-compilation-runtime-failure-restart",
    ),
}
REVISION = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
QUALIFICATION_SCHEMA = "apxm.agents.release-qualification.v1"
RESULT_SCHEMA = "apxm.agents.owner-qualification.v1"
FAILURE_SCHEMA = "apxm.agents.owner-qualification-failure.v1"
HANDOFF_SCHEMA = "apxm.agents.owner-handoff.v1"
COMMAND_SCHEMA = "apxm.agents.owner-command-execution.v1"
EVIDENCE_ROOT = Path(".apxm") / "owner-qualification"


def _canonical_json(value: object) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        allow_nan=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def _digest(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def _last_json_object(output: bytes) -> dict[str, Any] | None:
    try:
        text = output.decode("utf-8")
    except UnicodeDecodeError:
        return None
    decoder = json.JSONDecoder()
    for offset, character in enumerate(text):
        if character != "{":
            continue
        try:
            value, end = decoder.raw_decode(text[offset:])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and not text[offset + end :].strip():
            return value
    return None


def _print_bytes(payload: bytes, *, stderr: bool) -> None:
    stream = sys.stderr if stderr else sys.stdout
    binary = getattr(stream, "buffer", None)
    if binary is not None:
        binary.write(payload)
        binary.flush()
    else:
        stream.write(payload.decode("utf-8", errors="replace"))
        stream.flush()


def _ensure_directory(path: Path, *, label: str) -> None:
    if path.is_symlink():
        raise ValueError(f"{label} is a symlink")
    if path.exists() and not path.is_dir():
        raise ValueError(f"{label} is not a directory")
    if not path.exists():
        path.mkdir()
    if path.is_symlink() or not path.is_dir():
        raise ValueError(f"{label} is not a directory")


def _safe_parent(root: Path, parent: Path) -> None:
    root = root.resolve(strict=True)
    try:
        relative = parent.relative_to(root)
    except ValueError as error:
        raise ValueError("owner evidence path escapes the checkout") from error
    current = root
    for part in relative.parts:
        current /= part
        if current.is_symlink() or not current.is_dir():
            raise ValueError("owner evidence path contains a symlink or non-directory")


def _write_immutable(root: Path, relative: Path, payload: bytes) -> tuple[Path, str]:
    root = root.resolve(strict=True)
    target = root / relative
    _safe_parent(root, target.parent)
    if target.is_symlink() or (target.exists() and not target.is_file()):
        raise ValueError(f"immutable owner evidence target is not a regular file: {relative}")
    if target.exists():
        existing = target.read_bytes()
        if existing != payload:
            raise ValueError(f"refusing to overwrite immutable owner evidence: {relative}")
        return target, _digest(existing)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{target.name}.", suffix=".tmp", dir=target.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        try:
            os.link(temporary, target)
        except FileExistsError:
            existing = target.read_bytes()
            if existing != payload:
                raise ValueError(f"refusing to overwrite immutable owner evidence: {relative}")
        return target, _digest(payload)
    finally:
        if temporary.exists() or temporary.is_symlink():
            temporary.unlink()


def _classify_failure(returncode: int, output: bytes) -> str:
    lowered = output.decode("utf-8", errors="replace").casefold()
    if returncode in {137, 143} or any(
        marker in lowered
        for marker in ("out of memory", "no space left on device", "sigkill", "sigbus")
    ):
        return "resource"
    if any(
        marker in lowered
        for marker in ("e0514", "metadata version", "undefined symbol", "wrong architecture")
    ):
        return "abi"
    return "code"


def _strict_qualification(value: object, *, root: Path) -> dict[str, Any]:
    if not isinstance(value, dict) or value.get("schema") != QUALIFICATION_SCHEMA:
        raise ValueError("owner release qualification did not emit its neutral result schema")
    if value.get("owner") != "agents" or value.get("qualified") is not True:
        raise ValueError("owner release qualification did not pass")
    revision = value.get("source_revision")
    if not isinstance(revision, str) or not REVISION.fullmatch(revision):
        raise ValueError("owner release qualification has no immutable source revision")
    services = value.get("service_digests")
    if (
        not isinstance(services, dict)
        or set(services) != {"compilation-service", "runtime-service"}
        or any(not isinstance(item, str) or not DIGEST.fullmatch(item) for item in services.values())
    ):
        raise ValueError("owner release qualification did not bind both service digests")
    for field in (
        "source_descriptor_digest",
        "owner_descriptor_digest",
        "release_manifest_digest",
    ):
        if not isinstance(value.get(field), str) or not DIGEST.fullmatch(value[field]):
            raise ValueError(f"owner release qualification has no immutable {field}")
    if value.get("evidence_root") != str(root.resolve()):
        raise ValueError("owner release qualification evidence root is not this owner checkout")
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
        item.get("command")
        for item in gates
        if isinstance(item, dict) and item.get("returncode") == 0
    } if isinstance(gates, list) else set()
    if observed != expected:
        raise ValueError("owner release qualification did not prove the exact owner gates")
    return value


def _command_evidence(
    root: Path,
    phase: str,
    ordinal: int,
    command: str,
    status: str,
    returncode: int,
    output: bytes,
) -> dict[str, Any]:
    _ensure_directory(root / ".apxm", label=".apxm root")
    _ensure_directory(root / EVIDENCE_ROOT, label="owner qualification evidence root")
    phase_root = root / EVIDENCE_ROOT / phase
    _ensure_directory(phase_root, label="owner qualification phase root")
    output_digest = _digest(output)
    stem = f"{ordinal:03d}-{output_digest.removeprefix('sha256:')[:16]}"
    output_relative = EVIDENCE_ROOT / phase / f"{stem}.output"
    output_path, _ = _write_immutable(root, output_relative, output)
    document = {
        "schema": COMMAND_SCHEMA,
        "owner": "agents",
        "phase": phase,
        "command": command,
        "status": status,
        "returncode": returncode,
        "output_digest": output_digest,
        "output_artifact": {
            "reference": output_relative.as_posix(),
            "digest": output_digest,
        },
    }
    document_bytes = _canonical_json(document)
    document_relative = EVIDENCE_ROOT / phase / f"{stem}.json"
    document_path, document_digest = _write_immutable(root, document_relative, document_bytes)
    entry: dict[str, Any] = {
        "command": command,
        "status": status,
        "returncode": returncode,
        "output_digest": output_digest,
        "output_artifact": {
            "reference": output_relative.as_posix(),
            "digest": output_digest,
        },
        "evidence_artifact": {
            "reference": document_relative.as_posix(),
            "digest": document_digest,
        },
    }
    if status != "passed":
        entry["failure_classification"] = _classify_failure(returncode, output)
    return entry


def _handoff(
    root: Path,
    phase: str,
    qualification: dict[str, Any],
    commands: list[dict[str, Any]],
) -> tuple[Path, str]:
    qualification_bytes = _canonical_json(qualification)
    handoff = {
        "schema": HANDOFF_SCHEMA,
        "owner": "agents",
        "phase": phase,
        "status": "passed",
        "source_revision": qualification["source_revision"],
        "source_descriptor_digest": qualification["source_descriptor_digest"],
        "owner_descriptor_digest": qualification["owner_descriptor_digest"],
        "release_manifest_digest": qualification["release_manifest_digest"],
        "service_digests": qualification["service_digests"],
        "qualification_digest": _digest(qualification_bytes),
        "commands": commands,
    }
    relative = EVIDENCE_ROOT / "handoff" / f"{phase}.json"
    _ensure_directory(root / EVIDENCE_ROOT / "handoff", label="owner qualification handoff root")
    return _write_immutable(root, relative, _canonical_json(handoff))


def _run_phase(phase: str, *, root: Path = ROOT) -> int:
    commands = PHASES[phase]
    dekk = os.environ.get("DEKK", "").strip() or shutil.which("dekk")
    if not dekk:
        raise RuntimeError("dekk executable is unavailable")
    qualification: dict[str, Any] | None = None
    evidence: list[dict[str, Any]] = []
    for ordinal, command in enumerate(commands):
        completed = subprocess.run(
            [dekk, "agents", command],
            cwd=root,
            check=False,
            capture_output=True,
        )
        stdout = completed.stdout or b""
        stderr = completed.stderr or b""
        if stdout:
            _print_bytes(stdout, stderr=False)
        if stderr:
            _print_bytes(stderr, stderr=True)
        combined = stdout + stderr
        status = "passed" if completed.returncode == 0 else "failed"
        entry = _command_evidence(
            root,
            phase,
            ordinal,
            f"dekk agents {command}",
            status,
            completed.returncode,
            combined,
        )
        evidence.append(entry)
        if command == "release-qualification" and completed.returncode == 0:
            qualification = _strict_qualification(_last_json_object(stdout), root=root)
        if completed.returncode != 0:
            failure = {
                "schema": FAILURE_SCHEMA,
                "owner": "agents",
                "phase": phase,
                "status": "failed",
                "failed_command": entry["command"],
                "failure_classification": entry["failure_classification"],
                "returncode": completed.returncode,
                "evidence_artifact": entry["evidence_artifact"],
            }
            print(json.dumps(failure, sort_keys=True, separators=(",", ":")))
            return completed.returncode
    if qualification is None:
        raise RuntimeError("owner phase completed without release qualification evidence")
    handoff_path, handoff_digest = _handoff(root, phase, qualification, evidence)
    result = {
        "schema": RESULT_SCHEMA,
        "owner": "agents",
        "phase": phase,
        "status": "passed",
        "qualification": qualification,
        "qualification_digest": _digest(_canonical_json(qualification)),
        "command_evidence": evidence,
        "handoff": {
            "reference": handoff_path.relative_to(root.resolve()).as_posix(),
            "digest": handoff_digest,
        },
    }
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=tuple(PHASES))
    args = parser.parse_args(argv)
    try:
        return _run_phase(args.phase)
    except (OSError, RuntimeError, ValueError) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
