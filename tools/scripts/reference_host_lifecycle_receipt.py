#!/usr/bin/env python3
"""Exercise the canonical reference-host transport and record lifecycle evidence."""

from __future__ import annotations

import argparse
import copy
import hashlib
import importlib.util
import json
import subprocess
import sys
from pathlib import Path
from typing import Any, Callable


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
REFERENCE_HOST_RECEIPT_SCRIPT = REPOSITORY_ROOT / "tools" / "scripts" / "reference_host_receipt.py"
OWNER_DESCRIPTOR_PATH = (
    REPOSITORY_ROOT / "contracts" / "descriptors" / "apxm.agents-owner-descriptor.v1.json"
)
EXECUTION_MANIFEST_PATH = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "manifests"
    / "apxm.reference-host-execution-manifest.v1.json"
)
INVOKE_VECTOR_PATH = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "vectors"
    / "apxm.reference-host.invoke-parity.v1.json"
)
LIFECYCLE_VECTOR_PATH = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "vectors"
    / "apxm.reference-host.lifecycle-parity.v1.json"
)
RELEASE_MANIFEST_PATH = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "manifests"
    / "apxm.reference-host-release-manifest.v1.json"
)
DEFAULT_STARTUP_INPUT_PATH = (
    REPOSITORY_ROOT
    / ".apxm"
    / "reference-host"
    / "startup-inputs"
    / "apxm.reference-host-startup-input.v1.json"
)
DEFAULT_BUILD_RECEIPT_PATH = (
    REPOSITORY_ROOT
    / ".apxm"
    / "reference-host"
    / "receipts"
    / "apxm.reference-host-build-receipt.v1.json"
)
DEFAULT_LIFECYCLE_RECEIPT_PATH = (
    REPOSITORY_ROOT
    / ".apxm"
    / "reference-host"
    / "receipts"
    / "apxm.reference-host-lifecycle-receipt.v1.json"
)
REQUEST_SCHEMA = "apxm.runtime.host-request.v1"
HOST_SCHEMA = "apxm.runtime.host-response.v1"
ADMISSION_SCHEMA = "apxm.invocation-admission.v1"
STARTUP_INPUT_SCHEMA = "apxm.reference-host-startup-input.v1"
TRANSPORT_PROTOCOL = "jsonl-stdin-stdout"
OWNER_EXECUTABLE = "apxm-reference-host"
OWNER_EXECUTABLE_PATH = "crates/tools/cli/src/bin/reference_host.rs"
RECEIPT_SCHEMA_VERSION = "apxm.reference-host-lifecycle-receipt.v1"
LIFECYCLE_PROBE_SCHEMA = "apxm.reference-host.lifecycle-probe.v1"
IN_FLIGHT_DRAIN_PROBE = "drain_shutdown_after_in_flight_completion"
FAIL_CLOSED_ON = ["missing", "placeholder", "dirty", "mismatched", "implicit-default"]
HEX40 = set("0123456789abcdef")


class LifecycleReceiptError(RuntimeError):
    """Structured failure for the operator-facing lifecycle receipt."""

    def __init__(self, code: str, message: str, *, details: dict[str, Any] | None = None) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.details = details or {}


class ReferenceHostTransport:
    """JSONL stdin/stdout transport for the canonical reference host."""

    def __init__(self, executable_path: Path, startup_input_path: Path) -> None:
        self.executable_path = executable_path
        self.startup_input_path = startup_input_path
        self.process = subprocess.Popen(
            [str(executable_path), "--startup-input", str(startup_input_path)],
            cwd=REPOSITORY_ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        if self.process.stdin is None or self.process.stdout is None:
            raise LifecycleReceiptError(
                "transport_spawn_failed",
                "reference-host transport did not expose stdin/stdout pipes",
            )

    def request(self, payload: dict[str, Any]) -> dict[str, Any]:
        line = json.dumps(payload, sort_keys=True)
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()
        response_line = self.process.stdout.readline()
        if not response_line:
            stderr = ""
            if self.process.stderr is not None:
                stderr = self.process.stderr.read().strip()
            raise LifecycleReceiptError(
                "transport_eof",
                "reference-host transport closed before producing a response line",
                details={"stderr": stderr, "request": payload},
            )
        try:
            return json.loads(response_line)
        except json.JSONDecodeError as error:
            raise LifecycleReceiptError(
                "invalid_transport_json",
                f"reference-host transport returned non-JSON output: {error}",
                details={"raw_line": response_line},
            ) from error

    def close(self) -> None:
        stderr = ""
        if self.process.stdin is not None:
            self.process.stdin.close()
        if self.process.stderr is not None:
            stderr = self.process.stderr.read().strip()
        exit_code = self.process.wait()
        if exit_code != 0:
            raise LifecycleReceiptError(
                "transport_exit_nonzero",
                f"reference-host transport exited with status {exit_code}",
                details={"stderr": stderr},
            )


def load_module(path: Path, name: str) -> Any:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def file_digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def serialized_air_digest(air: dict[str, Any]) -> str:
    """Match the owner runtime's AirModule serde digest exactly."""
    encoded = json.dumps(air, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def is_exact_digest(value: str) -> bool:
    if not value.startswith("sha256:") or len(value) != 71:
        return False
    return all(char in "0123456789abcdef" for char in value.removeprefix("sha256:"))


def is_placeholder_digest(value: str) -> bool:
    return is_exact_digest(value) and len(set(value.removeprefix("sha256:"))) == 1


def expect(condition: bool, code: str, message: str, **details: Any) -> None:
    if not condition:
        raise LifecycleReceiptError(code, message, details=details or None)


def response_error_code(response: dict[str, Any]) -> str | None:
    error = response.get("error")
    if isinstance(error, dict):
        code = error.get("code")
        if isinstance(code, str):
            return code
    return None


def request_payload(
    operation: str,
    *,
    admission: dict[str, Any] | None = None,
    air: dict[str, Any] | None = None,
    schema_version: str = REQUEST_SCHEMA,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "schema_version": schema_version,
        "operation": operation,
    }
    if admission is not None:
        payload["admission"] = admission
    if air is not None:
        payload["air"] = air
    return payload


def valid_admission(
    startup_input: dict[str, Any],
    *,
    invocation_id: str = "invocation.1",
    artifact_digest: str | None = None,
) -> dict[str, Any]:
    return {
        "schema_version": ADMISSION_SCHEMA,
        "invocation_id": invocation_id,
        "artifact_digest": artifact_digest or startup_input["release_digest"],
        "release_digest": startup_input["release_digest"],
        "port_bindings_digest": startup_input["port_bindings_digest"],
        "resource_ceiling_digest": startup_input["resource_ceiling_digest"],
        "provenance_digest": startup_input["release_digest"],
    }


def invalid_provenance_admission(
    startup_input: dict[str, Any], *, artifact_digest: str | None = None
) -> dict[str, Any]:
    admission = valid_admission(startup_input, artifact_digest=artifact_digest)
    wrong_digest = "sha256:" + ("d" * 64)
    if wrong_digest == startup_input["release_digest"]:
        wrong_digest = "sha256:" + ("e" * 64)
    admission["provenance_digest"] = wrong_digest
    return admission


def host_readiness(
    startup_input: dict[str, Any], state: str, in_flight: int = 0
) -> dict[str, Any]:
    return {
        "schema_version": "apxm.runtime.host.v1",
        "contract_id": "apxm.runtime.host.v1",
        "state": state,
        "release_digest": startup_input["release_digest"],
        "port_bindings_digest": startup_input["port_bindings_digest"],
        "resource_ceiling_digest": startup_input["resource_ceiling_digest"],
        "in_flight": in_flight,
    }


def host_readiness_response(
    startup_input: dict[str, Any], state: str, in_flight: int = 0
) -> dict[str, Any]:
    return {
        "schema_version": HOST_SCHEMA,
        "status": "readiness",
        "readiness": host_readiness(startup_input, state, in_flight),
    }


def load_profile_and_vectors() -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    descriptor = load_json(OWNER_DESCRIPTOR_PATH)
    profiles = descriptor.get("published_host_lifecycle_profiles")
    expect(
        isinstance(profiles, list) and len(profiles) == 1,
        "invalid_owner_descriptor",
        "owner descriptor must publish exactly one reference-host lifecycle profile",
    )
    profile = profiles[0]
    expect(
        isinstance(profile, dict),
        "invalid_owner_descriptor",
        "published_host_lifecycle_profiles[0] must be an object",
    )
    return profile, load_json(INVOKE_VECTOR_PATH), load_json(LIFECYCLE_VECTOR_PATH)


def load_runtime_startup_input(startup_input_path: Path) -> dict[str, Any]:
    expect(
        startup_input_path.is_file(),
        "missing_startup_input",
        "reference-host startup-input artifact is missing",
        startup_input_path=str(startup_input_path),
    )
    startup_input = load_json(startup_input_path)
    expect(
        startup_input.get("schema_version") == STARTUP_INPUT_SCHEMA,
        "invalid_startup_input",
        "startup input schema_version must remain apxm.reference-host-startup-input.v1",
    )
    expect(
        startup_input.get("semantic_owner") == "agents",
        "invalid_startup_input",
        "startup input semantic_owner must be agents",
    )
    expect(
        startup_input.get("owner_executable") == OWNER_EXECUTABLE,
        "invalid_startup_input",
        "startup input owner_executable must remain the canonical reference host",
    )
    expect(
        startup_input.get("owner_executable_path") == OWNER_EXECUTABLE_PATH,
        "invalid_startup_input",
        "startup input owner_executable_path must remain the canonical source path",
    )
    expect(
        startup_input.get("transport_protocol") == TRANSPORT_PROTOCOL,
        "invalid_startup_input",
        "startup input transport_protocol must remain jsonl-stdin-stdout",
    )
    expect(
        startup_input.get("fail_closed_on") == FAIL_CLOSED_ON,
        "invalid_startup_input",
        "startup input fail_closed_on must publish the exact fail-closed reasons",
    )
    for field in ("release_digest", "port_bindings_digest", "resource_ceiling_digest"):
        value = startup_input.get(field)
        expect(
            isinstance(value, str) and is_exact_digest(value),
            "invalid_startup_input",
            f"startup input {field} must be an exact sha256:<64 hex> digest",
            field=field,
        )
        expect(
            not is_placeholder_digest(value),
            "invalid_startup_input",
            f"startup input {field} must not be a placeholder digest",
            field=field,
        )

    provenance = startup_input.get("provenance")
    expect(
        isinstance(provenance, dict),
        "invalid_startup_input",
        "startup input provenance must be an object",
    )
    owner_revision = provenance.get("owner_revision")
    expect(
        isinstance(owner_revision, str)
        and len(owner_revision) == 40
        and all(char in HEX40 for char in owner_revision),
        "invalid_startup_input",
        "startup input provenance.owner_revision must be a lowercase 40-hex revision",
    )
    for field in ("descriptor_semantic_digest", "descriptor_exact_checksum"):
        value = provenance.get(field)
        expect(
            isinstance(value, str) and is_exact_digest(value) and not is_placeholder_digest(value),
            "invalid_startup_input",
            f"startup input provenance.{field} must be an exact non-placeholder digest",
            field=field,
        )
    expect(
        provenance.get("dirty") is False,
        "invalid_startup_input",
        "startup input provenance must prove a clean owner checkout",
    )

    manifest_ref = startup_input.get("reference_host_release_manifest")
    expect(
        isinstance(manifest_ref, dict),
        "invalid_startup_input",
        "startup input reference_host_release_manifest must be an object",
    )
    expected_manifest_path = str(RELEASE_MANIFEST_PATH.resolve())
    expect(
        manifest_ref.get("path") == expected_manifest_path,
        "invalid_startup_input",
        "startup input release manifest path must remain the canonical exact release manifest",
        expected_path=expected_manifest_path,
        observed_path=manifest_ref.get("path"),
    )
    expected_manifest_digest = file_digest(RELEASE_MANIFEST_PATH)
    expect(
        manifest_ref.get("digest") == expected_manifest_digest,
        "invalid_startup_input",
        "startup input release manifest digest mismatched the committed release manifest",
        expected_digest=expected_manifest_digest,
        observed_digest=manifest_ref.get("digest"),
    )
    return startup_input


def run_build_receipt(build_receipt_path: Path) -> tuple[int, dict[str, Any]]:
    module = load_module(REFERENCE_HOST_RECEIPT_SCRIPT, "reference_host_receipt")
    return module.write_reference_host_receipt(receipt_path=build_receipt_path)


def build_receipt_evidence(build_receipt_path: Path, build_receipt: dict[str, Any]) -> dict[str, Any]:
    evidence = {
        "path": str(build_receipt_path),
        "exists": build_receipt_path.is_file(),
        "status": build_receipt.get("status"),
    }
    if build_receipt_path.is_file():
        evidence["digest"] = file_digest(build_receipt_path)
    return evidence


def make_transport_factory(
    factory: Callable[[Path, Path], ReferenceHostTransport] | None,
) -> Callable[[Path, Path], ReferenceHostTransport]:
    return factory or (lambda executable_path, startup_input_path: ReferenceHostTransport(executable_path, startup_input_path))


def close_transport(transport: ReferenceHostTransport) -> None:
    transport.close()


def validate_build_identity(
    build_receipt: dict[str, Any], startup_input: dict[str, Any]
) -> dict[str, Any]:
    identity = build_receipt.get("build_identity")
    expect(
        isinstance(identity, dict),
        "invalid_build_receipt",
        "build receipt must publish build_identity",
    )
    expected_owner_revision = startup_input["provenance"]["owner_revision"]
    expect(
        identity.get("owner_revision") == expected_owner_revision,
        "invalid_build_receipt",
        "build receipt owner revision must match startup input provenance",
        expected_owner_revision=expected_owner_revision,
        observed_owner_revision=identity.get("owner_revision"),
    )
    expected_release_manifest_path = str(RELEASE_MANIFEST_PATH.relative_to(REPOSITORY_ROOT))
    expect(
        identity.get("release_manifest_path") == expected_release_manifest_path,
        "invalid_build_receipt",
        "build receipt release manifest path drifted from the canonical cohort path",
        expected_release_manifest_path=expected_release_manifest_path,
        observed_release_manifest_path=identity.get("release_manifest_path"),
    )
    expected_execution_manifest_path = str(EXECUTION_MANIFEST_PATH.relative_to(REPOSITORY_ROOT))
    expect(
        identity.get("execution_manifest_path") == expected_execution_manifest_path,
        "invalid_build_receipt",
        "build receipt execution manifest path drifted from the canonical cohort path",
        expected_execution_manifest_path=expected_execution_manifest_path,
        observed_execution_manifest_path=identity.get("execution_manifest_path"),
    )

    binary_platform = identity.get("binary_platform")
    expect(
        isinstance(binary_platform, dict),
        "invalid_build_receipt",
        "build receipt binary_platform must be an object",
    )
    expect(
        binary_platform.get("selection") in {"explicit-target", "native-host-default"},
        "invalid_build_receipt",
        "build receipt binary_platform.selection must remain exact",
        observed_selection=binary_platform.get("selection"),
    )
    for field in ("system", "machine", "platform_tag"):
        value = binary_platform.get(field)
        expect(
            isinstance(value, str) and value,
            "invalid_build_receipt",
            f"build receipt binary_platform.{field} must be explicit",
            field=field,
        )
    if binary_platform.get("selection") == "explicit-target":
        expect(
            isinstance(binary_platform.get("target"), str) and binary_platform["target"],
            "invalid_build_receipt",
            "build receipt binary_platform.target must be explicit for explicit-target builds",
        )

    release_attestation = build_receipt.get("reference_host_release")
    cohort = release_attestation.get("cohort") if isinstance(release_attestation, dict) else None
    expect(
        isinstance(cohort, dict) and cohort.get("revision") == expected_owner_revision,
        "invalid_build_receipt",
        "build receipt release attestation must bind the same owner revision",
        observed_release_attestation=release_attestation,
    )
    return identity


def assert_runtime_evidence(response: dict[str, Any], terminal_kind: str) -> dict[str, Any]:
    runtime_evidence = response.get("runtime_evidence")
    expect(
        isinstance(runtime_evidence, dict)
        and runtime_evidence.get("schema_version") == "apxm.runtime-evidence.v1",
        "case_failed",
        "runtime evidence must remain apxm.runtime-evidence.v1",
    )
    facts = runtime_evidence.get("facts")
    expect(isinstance(facts, list) and facts, "case_failed", "runtime evidence facts must be present")
    terminal_facts = [
        fact
        for fact in facts
        if isinstance(fact, dict) and fact.get("fact_kind") == terminal_kind
    ]
    expect(
        len(terminal_facts) == 1,
        "case_failed",
        f"runtime evidence must record exactly one {terminal_kind} fact",
    )
    return runtime_evidence


def run_case_readiness(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        request = request_payload("readiness")
        response = transport.request(request)
        expect(response == host_readiness_response(startup_input, "ready"), "case_failed", "readiness response drifted")
        return {
            "name": "readiness",
            "status": "passed",
            "requests": [request],
            "responses": [response],
        }
    finally:
        close_transport(transport)


def run_case_positive_commit(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    minimal_air: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        request = request_payload(
            "invoke",
            admission=valid_admission(
                startup_input, artifact_digest=serialized_air_digest(minimal_air)
            ),
            air=minimal_air,
        )
        response = transport.request(request)
        expect(response.get("status") == "committed", "case_failed", "positive admission must commit")
        runtime_evidence = assert_runtime_evidence(response, "invocation.committed")
        return {
            "name": "positive_commit_minimal_air",
            "status": "passed",
            "requests": [request],
            "responses": [response],
            "runtime_evidence_terminal_kind": "invocation.committed",
            "runtime_evidence": runtime_evidence,
        }
    finally:
        close_transport(transport)


def run_case_negative_admission(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    minimal_air: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        request = request_payload(
            "invoke",
            admission=invalid_provenance_admission(
                startup_input, artifact_digest=serialized_air_digest(minimal_air)
            ),
            air=minimal_air,
        )
        response = transport.request(request)
        expect(response.get("status") == "rejected", "case_failed", "invalid admission must be rejected")
        expect(
            response_error_code(response) == "provenance_mismatch",
            "case_failed",
            "invalid admission must fail closed with provenance_mismatch",
        )
        expect(
            response.get("readiness") == host_readiness(startup_input, "ready"),
            "case_failed",
            "invalid admission must leave the host ready",
        )
        return {
            "name": "negative_admission_provenance_rejection",
            "status": "passed",
            "requests": [request],
            "responses": [response],
        }
    finally:
        close_transport(transport)


def run_case_invalid_air(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    invalid_air: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        request = request_payload(
            "invoke",
            admission=valid_admission(
                startup_input, artifact_digest=serialized_air_digest(invalid_air)
            ),
            air=invalid_air,
        )
        response = transport.request(request)
        expect(response.get("status") == "rejected", "case_failed", "invalid AIR must be rejected")
        expect(
            response_error_code(response) == "invalid_air",
            "case_failed",
            "invalid AIR must fail closed with invalid_air",
        )
        expect(
            response.get("readiness") == host_readiness(startup_input, "ready"),
            "case_failed",
            "invalid AIR must leave the host ready",
        )
        return {
            "name": "invalid_air_failure",
            "status": "passed",
            "requests": [request],
            "responses": [response],
        }
    finally:
        close_transport(transport)


def run_case_drain_rejection(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    minimal_air: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        drain_request = request_payload("drain")
        drain_response = transport.request(drain_request)
        expected_draining = host_readiness(startup_input, "draining")
        expect(drain_response == expected_draining, "case_failed", "drain response drifted")
        rejected_request = request_payload(
            "invoke",
            admission=valid_admission(
                startup_input, artifact_digest=serialized_air_digest(minimal_air)
            ),
            air=minimal_air,
        )
        rejected_response = transport.request(rejected_request)
        expect(
            rejected_response.get("status") == "rejected",
            "case_failed",
            "draining host must reject new admission",
        )
        expect(
            response_error_code(rejected_response) == "host_not_accepting",
            "case_failed",
            "draining host must reject with host_not_accepting",
        )
        expect(
            rejected_response.get("readiness") == expected_draining,
            "case_failed",
            "draining rejection must retain draining readiness",
        )
        return {
            "name": "drain_rejection_before_dispatch",
            "status": "passed",
            "requests": [drain_request, rejected_request],
            "responses": [drain_response, rejected_response],
        }
    finally:
        close_transport(transport)


def run_in_flight_drain_probe(
    executable_path: Path, startup_input_path: Path
) -> dict[str, Any]:
    try:
        result = subprocess.run(
            [
                str(executable_path),
                "--startup-input",
                str(startup_input_path),
                "--lifecycle-probe",
                IN_FLIGHT_DRAIN_PROBE,
            ],
            cwd=REPOSITORY_ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise LifecycleReceiptError(
            "in_flight_drain_probe_failed",
            "reference-host in-flight drain probe could not start",
            details={"error": str(error), "executable_path": str(executable_path)},
        ) from error
    expect(
        result.returncode == 0,
        "in_flight_drain_probe_failed",
        "reference-host in-flight drain probe exited nonzero",
        returncode=result.returncode,
        stderr=result.stderr.strip(),
    )
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    expect(
        len(lines) == 1,
        "in_flight_drain_probe_failed",
        "reference-host in-flight drain probe must emit exactly one JSON object",
        stdout=result.stdout,
    )
    try:
        probe = json.loads(lines[0])
    except json.JSONDecodeError as error:
        raise LifecycleReceiptError(
            "in_flight_drain_probe_failed",
            f"reference-host in-flight drain probe returned invalid JSON: {error}",
            details={"stdout": result.stdout},
        ) from error
    expect(
        isinstance(probe, dict),
        "in_flight_drain_probe_failed",
        "reference-host in-flight drain probe must return an object",
    )
    return probe


def run_case_in_flight_drain(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    lifecycle_probe_runner: Callable[[Path, Path], dict[str, Any]],
    **_: Any,
) -> dict[str, Any]:
    probe = lifecycle_probe_runner(executable_path, startup_input_path)
    expect(
        probe.get("schema_version") == LIFECYCLE_PROBE_SCHEMA,
        "case_failed",
        "in-flight drain probe schema drifted",
    )
    expect(
        probe.get("semantic_owner") == "agents",
        "case_failed",
        "in-flight drain probe semantic owner drifted",
    )
    expect(
        probe.get("case") == IN_FLIGHT_DRAIN_PROBE,
        "case_failed",
        "in-flight drain probe case drifted",
    )
    expect(
        probe.get("transition") == "stop_admission_then_finish_in_flight",
        "case_failed",
        "in-flight drain probe transition drifted",
    )
    expect(
        probe.get("in_flight_before_drain") == host_readiness(startup_input, "ready", 1),
        "case_failed",
        "in-flight drain probe must observe one admitted invocation before drain",
    )
    expect(
        probe.get("drain_response") == host_readiness(startup_input, "draining", 1),
        "case_failed",
        "in-flight drain probe must stop admission while work remains in flight",
    )
    completion = probe.get("completion_response")
    expect(
        isinstance(completion, dict) and completion.get("status") == "committed",
        "case_failed",
        "in-flight drain probe must finish the admitted invocation",
    )
    runtime_evidence = assert_runtime_evidence(completion, "invocation.committed")
    terminal_readiness = probe.get("terminal_readiness")
    expect(
        terminal_readiness == host_readiness(startup_input, "stopped"),
        "case_failed",
        "in-flight drain probe must reach terminal quiescence",
    )
    return {
        "name": IN_FLIGHT_DRAIN_PROBE,
        "status": "passed",
        "probe": probe,
        "runtime_evidence_terminal_kind": "invocation.committed",
        "runtime_evidence": runtime_evidence,
        "in_flight_before_drain": 1,
        "terminal_state_after_in_flight": "stopped",
        "covered_release_cases": [IN_FLIGHT_DRAIN_PROBE],
    }


def run_case_cancellation(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        request = request_payload("cancel")
        response = transport.request(request)
        expect(response.get("status") == "cancelled", "case_failed", "cancel must terminate before admission")
        expect(
            response.get("reason") == "cancelled_before_admission",
            "case_failed",
            "cancel must use cancelled_before_admission",
        )
        expect(
            response.get("readiness") == host_readiness(startup_input, "stopped"),
            "case_failed",
            "cancel must stop the host",
        )
        return {
            "name": "cancellation_before_admission",
            "status": "passed",
            "requests": [request],
            "responses": [response],
            "covered_release_cases": ["cancellation_before_admission"],
        }
    finally:
        close_transport(transport)


def run_case_shutdown(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        request = request_payload("shutdown")
        response = transport.request(request)
        expect(response.get("status") == "shutdown", "case_failed", "shutdown must reach a terminal state")
        expect(
            response.get("reason") == "shutdown_terminal",
            "case_failed",
            "shutdown must record shutdown_terminal",
        )
        expect(
            response.get("readiness") == host_readiness(startup_input, "stopped"),
            "case_failed",
            "shutdown must stop the host",
        )
        return {
            "name": "explicit_shutdown_terminal_state",
            "status": "passed",
            "requests": [request],
            "responses": [response],
            "covered_release_cases": ["explicit_shutdown_terminal_state"],
        }
    finally:
        close_transport(transport)


def run_case_restart_recovery(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    minimal_air: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        invoke_request = request_payload(
            "invoke",
            admission=valid_admission(
                startup_input, artifact_digest=serialized_air_digest(minimal_air)
            ),
            air=minimal_air,
        )
        committed = transport.request(invoke_request)
        runtime_evidence = assert_runtime_evidence(committed, "invocation.committed")
        shutdown_request = request_payload("shutdown")
        shutdown = transport.request(shutdown_request)
        expect(shutdown.get("status") == "shutdown", "case_failed", "restart recovery setup must shut down cleanly")
        restart_request = request_payload("restart")
        restarted = transport.request(restart_request)
        expect(restarted.get("status") == "restarted", "case_failed", "restart must succeed from a stopped state")
        recovery = restarted.get("recovery")
        expect(
            isinstance(recovery, dict)
            and recovery.get("contract") == "reconcile_from_runtime_evidence"
            and recovery.get("status") == "reconciled",
            "case_failed",
            "restart must reconcile from runtime evidence",
        )
        expect(
            recovery.get("runtime_evidence") == runtime_evidence,
            "case_failed",
            "restart recovery must publish the canonical runtime evidence",
        )
        expect(
            restarted.get("readiness") == host_readiness(startup_input, "ready"),
            "case_failed",
            "restart must restore ready readiness",
        )
        return {
            "name": "restart_recovery_from_runtime_evidence",
            "status": "passed",
            "requests": [invoke_request, shutdown_request, restart_request],
            "responses": [committed, shutdown, restarted],
            "runtime_evidence_terminal_kind": "invocation.committed",
            "runtime_evidence": runtime_evidence,
            "covered_release_cases": [
                "explicit_shutdown_terminal_state",
                "restart_recovery_from_runtime_evidence",
            ],
        }
    finally:
        close_transport(transport)


def run_case_revocation(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    minimal_air: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        revoke_request = request_payload("revoke")
        revoked = transport.request(revoke_request)
        expect(revoked.get("status") == "revoked", "case_failed", "revoke must close admission before dispatch")
        expect(
            revoked.get("reason") == "admission_revoked",
            "case_failed",
            "revoke must record admission_revoked",
        )
        expect(
            revoked.get("readiness") == host_readiness(startup_input, "stopped"),
            "case_failed",
            "revoke must stop the host",
        )
        rejected_request = request_payload(
            "invoke",
            admission=valid_admission(
                startup_input, artifact_digest=serialized_air_digest(minimal_air)
            ),
            air=minimal_air,
        )
        rejected = transport.request(rejected_request)
        expect(
            rejected.get("status") == "rejected"
            and response_error_code(rejected) == "admission_revoked",
            "case_failed",
            "revoked host must fail closed on the next activation",
        )
        expect(
            rejected.get("readiness") == host_readiness(startup_input, "stopped"),
            "case_failed",
            "revoked host must remain stopped",
        )
        return {
            "name": "revocation_before_dispatch",
            "status": "passed",
            "requests": [revoke_request, rejected_request],
            "responses": [revoked, rejected],
            "covered_release_cases": ["revocation_before_dispatch"],
        }
    finally:
        close_transport(transport)


def run_case_boundary_fail_closed(
    executable_path: Path,
    startup_input_path: Path,
    startup_input: dict[str, Any],
    transport_factory: Callable[[Path, Path], ReferenceHostTransport],
    **_: Any,
) -> dict[str, Any]:
    transport = transport_factory(executable_path, startup_input_path)
    try:
        requests = [
            request_payload("readiness", schema_version="apxm.runtime.host-request.v0"),
            request_payload("not-a-real-operation"),
            request_payload("invoke"),
        ]
        responses = [transport.request(request) for request in requests]
        observed_codes = [response_error_code(response) for response in responses]
        expect(
            observed_codes == ["invalid_request_schema", "unknown_operation", "missing_admission"],
            "case_failed",
            "boundary rejections drifted",
            observed_codes=observed_codes,
        )
        expect(
            responses[-1].get("readiness") == host_readiness(startup_input, "ready"),
            "case_failed",
            "boundary rejections must keep the host ready",
        )
        return {
            "name": "boundary_fail_closed",
            "status": "passed",
            "requests": requests,
            "responses": responses,
            "covered_release_cases": ["boundary_fail_closed"],
        }
    finally:
        close_transport(transport)


def executed_case_runners() -> list[tuple[str, Callable[..., dict[str, Any]]]]:
    return [
        ("readiness", run_case_readiness),
        ("positive_commit_minimal_air", run_case_positive_commit),
        ("negative_admission_provenance_rejection", run_case_negative_admission),
        ("invalid_air_failure", run_case_invalid_air),
        ("drain_rejection_before_dispatch", run_case_drain_rejection),
        (IN_FLIGHT_DRAIN_PROBE, run_case_in_flight_drain),
        ("cancellation_before_admission", run_case_cancellation),
        ("explicit_shutdown_terminal_state", run_case_shutdown),
        ("restart_recovery_from_runtime_evidence", run_case_restart_recovery),
        ("revocation_before_dispatch", run_case_revocation),
        ("boundary_fail_closed", run_case_boundary_fail_closed),
    ]


def lifecycle_outcomes_summary(cases: list[dict[str, Any]]) -> dict[str, Any]:
    cases_by_name = {case["name"]: case for case in cases if case.get("status") == "passed"}

    def case_named(name: str) -> dict[str, Any]:
        case = cases_by_name.get(name)
        expect(
            isinstance(case, dict),
            "case_failed",
            f"lifecycle summary requires the passed case {name}",
        )
        return case

    def response(name: str, index: int = 0) -> dict[str, Any]:
        case = case_named(name)
        responses = case.get("responses")
        expect(
            isinstance(responses, list)
            and len(responses) > index
            and isinstance(responses[index], dict),
            "case_failed",
            f"lifecycle summary requires response[{index}] for case {name}",
        )
        return responses[index]

    def readiness(name: str, index: int = 0) -> dict[str, Any]:
        snapshot = response(name, index).get("readiness")
        expect(
            isinstance(snapshot, dict),
            "case_failed",
            f"lifecycle summary requires readiness for case {name}",
        )
        return snapshot

    boundary_case = case_named("boundary_fail_closed")
    boundary_responses = boundary_case.get("responses")
    expect(
        isinstance(boundary_responses, list) and all(isinstance(entry, dict) for entry in boundary_responses),
        "case_failed",
        "lifecycle summary requires boundary_fail_closed responses",
    )
    drain_probe = case_named(IN_FLIGHT_DRAIN_PROBE).get("probe")
    expect(
        isinstance(drain_probe, dict),
        "case_failed",
        "lifecycle summary requires the in-flight drain probe payload",
    )
    restart_response = response("restart_recovery_from_runtime_evidence", 2)
    restart_recovery = restart_response.get("recovery")
    expect(
        isinstance(restart_recovery, dict),
        "case_failed",
        "lifecycle summary requires restart recovery details",
    )

    return {
        "readiness": {
            "initial_ready": response("readiness"),
            "draining_before_dispatch": response("drain_rejection_before_dispatch"),
            "draining_with_in_flight": drain_probe["drain_response"],
            "terminal_after_in_flight_drain": drain_probe["terminal_readiness"],
            "terminal_after_cancellation_before_admission": readiness(
                "cancellation_before_admission"
            ),
            "terminal_after_shutdown": readiness("explicit_shutdown_terminal_state"),
            "ready_after_restart_recovery": readiness(
                "restart_recovery_from_runtime_evidence", 2
            ),
            "terminal_after_revocation": readiness("revocation_before_dispatch", 1),
        },
        "admission": {
            "positive_commit": {
                "status": response("positive_commit_minimal_air").get("status"),
                "runtime_evidence_terminal_kind": case_named("positive_commit_minimal_air").get(
                    "runtime_evidence_terminal_kind"
                ),
            },
            "negative_provenance_rejection": {
                "status": response("negative_admission_provenance_rejection").get("status"),
                "error_code": response_error_code(
                    response("negative_admission_provenance_rejection")
                ),
            },
            "invalid_air_rejection": {
                "status": response("invalid_air_failure").get("status"),
                "error_code": response_error_code(response("invalid_air_failure")),
            },
            "rejection_while_draining": {
                "status": response("drain_rejection_before_dispatch", 1).get("status"),
                "error_code": response_error_code(response("drain_rejection_before_dispatch", 1)),
            },
            "revocation_before_dispatch": {
                "status": response("revocation_before_dispatch").get("status"),
                "reason": response("revocation_before_dispatch").get("reason"),
            },
            "rejection_after_revocation": {
                "status": response("revocation_before_dispatch", 1).get("status"),
                "error_code": response_error_code(response("revocation_before_dispatch", 1)),
            },
            "boundary_fail_closed": {
                "error_codes": [
                    response_error_code(entry) for entry in boundary_responses  # type: ignore[arg-type]
                ],
            },
        },
        "terminal": {
            "cancellation_before_admission": {
                "status": response("cancellation_before_admission").get("status"),
                "reason": response("cancellation_before_admission").get("reason"),
            },
            "explicit_shutdown": {
                "status": response("explicit_shutdown_terminal_state").get("status"),
                "reason": response("explicit_shutdown_terminal_state").get("reason"),
            },
            "restart_recovery": {
                "status": restart_response.get("status"),
                "contract": restart_recovery.get("contract"),
                "status_detail": restart_recovery.get("status"),
            },
        },
    }

def write_reference_host_lifecycle_receipt(
    *,
    receipt_path: Path = DEFAULT_LIFECYCLE_RECEIPT_PATH,
    build_receipt_path: Path = DEFAULT_BUILD_RECEIPT_PATH,
    startup_input_path: Path = DEFAULT_STARTUP_INPUT_PATH,
    build_receipt_runner: Callable[[Path], tuple[int, dict[str, Any]]] = run_build_receipt,
    transport_factory: Callable[[Path, Path], ReferenceHostTransport] | None = None,
    lifecycle_probe_runner: Callable[[Path, Path], dict[str, Any]] = run_in_flight_drain_probe,
) -> tuple[int, dict[str, Any]]:
    receipt: dict[str, Any] = {
        "schema_version": RECEIPT_SCHEMA_VERSION,
        "semantic_owner": "agents",
        "status": "failed",
    }

    try:
        startup_input = load_runtime_startup_input(startup_input_path)
        profile, invoke_vector, lifecycle_vector = load_profile_and_vectors()
        build_exit_code, build_receipt = build_receipt_runner(build_receipt_path)
        receipt["build_receipt"] = build_receipt_evidence(build_receipt_path, build_receipt)
        receipt["reference_host_release"] = build_receipt.get("reference_host_release")
        receipt["startup_input"] = {
            "path": str(startup_input_path),
            "digest": file_digest(startup_input_path),
            "release_digest": startup_input["release_digest"],
            "port_bindings_digest": startup_input["port_bindings_digest"],
            "resource_ceiling_digest": startup_input["resource_ceiling_digest"],
            "reference_host_release_manifest": startup_input["reference_host_release_manifest"],
            "provenance": startup_input["provenance"],
            "fail_closed_on": startup_input["fail_closed_on"],
        }
        receipt["release_manifest"] = {
            "path": str(RELEASE_MANIFEST_PATH),
            "digest": file_digest(RELEASE_MANIFEST_PATH),
        }
        receipt["lifecycle_profile"] = profile
        receipt["vectors"] = {
            "invoke_parity": {
                "path": str(INVOKE_VECTOR_PATH),
                "digest": file_digest(INVOKE_VECTOR_PATH),
            },
            "lifecycle_parity": {
                "path": str(LIFECYCLE_VECTOR_PATH),
                "digest": file_digest(LIFECYCLE_VECTOR_PATH),
            },
        }

        if build_exit_code != 0 or build_receipt.get("status") != "built":
            receipt["status"] = "build_receipt_failed"
            receipt["error"] = {
                "code": "build_receipt_failed",
                "message": "reference-host lifecycle receipt requires a successful canonical build receipt",
            }
            write_json(receipt_path, receipt)
            return 1, receipt

        build_identity = validate_build_identity(build_receipt, startup_input)
        receipt["build_identity"] = build_identity
        output = build_receipt.get("output")
        expect(
            isinstance(output, dict) and isinstance(output.get("path"), str) and output["path"],
            "build_receipt_failed",
            "build receipt did not publish the canonical executable path",
        )
        executable_path = Path(output["path"])
        expect(
            executable_path.is_file(),
            "missing_executable",
            "canonical reference-host executable is missing after a built receipt",
            executable_path=str(executable_path),
        )

        minimal_air = copy.deepcopy(invoke_vector["fixtures"]["minimal_valid_air"])
        invalid_air = copy.deepcopy(invoke_vector["fixtures"]["invalid_air_missing_source_map"])
        transport = make_transport_factory(transport_factory)

        cases: list[dict[str, Any]] = []
        covered_release_cases: set[str] = set()
        case_failures: list[dict[str, Any]] = []
        for _, runner in executed_case_runners():
            try:
                case = runner(
                    executable_path=executable_path,
                    startup_input_path=startup_input_path,
                    startup_input=startup_input,
                    minimal_air=minimal_air,
                    invalid_air=invalid_air,
                    transport_factory=transport,
                    lifecycle_probe_runner=lifecycle_probe_runner,
                )
                cases.append(case)
                covered_release_cases.update(case.get("covered_release_cases", []))
            except LifecycleReceiptError as error:
                failed_case = {
                    "name": runner.__name__.removeprefix("run_case_"),
                    "status": "failed",
                    "error": {
                        "code": error.code,
                        "message": error.message,
                    },
                }
                if error.details:
                    failed_case["error"]["details"] = error.details
                cases.append(failed_case)
                case_failures.append(failed_case)
                break

        required_cases = [case["name"] for case in lifecycle_vector["cases"]]
        covered_release_case_list = [case for case in required_cases if case in covered_release_cases]
        remaining_release_cases = [case for case in required_cases if case not in covered_release_cases]
        blockers: list[dict[str, str]] = []

        receipt["cases"] = cases
        receipt["coverage"] = {
            "required_release_cases": required_cases,
            "covered_release_cases": covered_release_case_list,
            "remaining_release_cases": remaining_release_cases,
            "remaining_release_case_blockers": blockers,
        }

        if case_failures:
            receipt["status"] = "case_failed"
            receipt["error"] = {
                "code": "case_failed",
                "message": "one or more lifecycle evidence cases failed closed",
                "failed_cases": [failure["name"] for failure in case_failures],
            }
            write_json(receipt_path, receipt)
            return 1, receipt

        receipt["lifecycle_outcomes"] = lifecycle_outcomes_summary(cases)
        receipt["status"] = "complete" if not remaining_release_cases else "partial"
        write_json(receipt_path, receipt)
        return 0, receipt
    except LifecycleReceiptError as error:
        receipt["status"] = error.code
        receipt["error"] = {
            "code": error.code,
            "message": error.message,
        }
        if error.details:
            receipt["error"]["details"] = error.details
        write_json(receipt_path, receipt)
        return 1, receipt


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt-path", type=Path, default=DEFAULT_LIFECYCLE_RECEIPT_PATH)
    parser.add_argument("--build-receipt-path", type=Path, default=DEFAULT_BUILD_RECEIPT_PATH)
    parser.add_argument("--startup-input-path", type=Path, default=DEFAULT_STARTUP_INPUT_PATH)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    exit_code, receipt = write_reference_host_lifecycle_receipt(
        receipt_path=args.receipt_path,
        build_receipt_path=args.build_receipt_path,
        startup_input_path=args.startup_input_path,
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
