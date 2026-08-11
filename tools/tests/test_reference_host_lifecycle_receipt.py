"""Reference-host lifecycle receipt stays exact, typed, and fail-closed."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPOSITORY_ROOT / "tools" / "scripts" / "reference_host_lifecycle_receipt.py"


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeTransport:
    def __init__(self, responses: list[dict[str, object]]) -> None:
        self.responses = list(responses)
        self.requests: list[dict[str, object]] = []
        self.closed = False

    def request(self, payload: dict[str, object]) -> dict[str, object]:
        self.requests.append(payload)
        if not self.responses:
            raise AssertionError("no scripted response left for fake transport")
        return self.responses.pop(0)

    def close(self) -> None:
        self.closed = True
        if self.responses:
            raise AssertionError("fake transport closed with unconsumed scripted responses")


class ScriptedTransportFactory:
    def __init__(self, responses_by_spawn: list[list[dict[str, object]]]) -> None:
        self._responses_by_spawn = [FakeTransport(responses) for responses in responses_by_spawn]
        self.call_count = 0

    def __call__(self, _executable_path: Path, _startup_input_path: Path) -> FakeTransport:
        if self.call_count >= len(self._responses_by_spawn):
            raise AssertionError("unexpected transport spawn")
        transport = self._responses_by_spawn[self.call_count]
        self.call_count += 1
        return transport


class ReferenceHostLifecycleReceiptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = load_module(SCRIPT_PATH, "reference_host_lifecycle_receipt")

    def test_serialized_air_digest_matches_owner_field_order(self) -> None:
        air = {
            "schema_version": "apxm.air.v2",
            "semantic_operations": [],
            "structural_ir": [],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": [],
            },
        }
        self.assertEqual(
            self.module.serialized_air_digest(air),
            "sha256:f3755ae0ba0b4e73d5d058291ba830e4668ab6b83b48d7413688c58befe57383",
        )

    def make_startup_input(self, root: Path) -> tuple[Path, dict[str, object]]:
        release_digest = "sha256:" + ("0123456789abcdef" * 4)
        port_bindings_digest = "sha256:" + ("abcdef0123456789" * 4)
        resource_ceiling_digest = "sha256:" + ("89abcdef01234567" * 4)
        descriptor_semantic_digest = "sha256:" + ("0011223344556677" * 4)
        descriptor_exact_checksum = "sha256:" + ("8899aabbccddeeff" * 4)
        startup_input = {
            "schema_version": self.module.STARTUP_INPUT_SCHEMA,
            "semantic_owner": "agents",
            "owner_executable": self.module.OWNER_EXECUTABLE,
            "owner_executable_path": self.module.OWNER_EXECUTABLE_PATH,
            "transport_protocol": self.module.TRANSPORT_PROTOCOL,
            "reference_host_release_manifest": {
                "path": str(self.module.RELEASE_MANIFEST_PATH.resolve()),
                "digest": self.module.file_digest(self.module.RELEASE_MANIFEST_PATH),
            },
            "release_digest": release_digest,
            "port_bindings_digest": port_bindings_digest,
            "resource_ceiling_digest": resource_ceiling_digest,
            "provenance": {
                "owner_revision": "a" * 40,
                "descriptor_semantic_digest": descriptor_semantic_digest,
                "descriptor_exact_checksum": descriptor_exact_checksum,
                "dirty": False,
            },
            "fail_closed_on": list(self.module.FAIL_CLOSED_ON),
        }
        path = root / "startup-input.json"
        path.write_text(json.dumps(startup_input, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        return path, startup_input

    def make_runtime_evidence(self, invocation_id: str) -> dict[str, object]:
        return {
            "schema_version": "apxm.runtime-evidence.v1",
            "facts": [
                {"fact_id": f"{invocation_id}.created", "event_sequence": 0, "fact_kind": "instance.created"},
                {"fact_id": f"{invocation_id}.admitted", "event_sequence": 1, "fact_kind": "invocation.admitted"},
                {
                    "fact_id": f"{invocation_id}.terminal",
                    "event_sequence": 2,
                    "fact_kind": "invocation.committed",
                    "commit_sequence": 1,
                },
            ],
        }

    def make_build_receipt_runner(
        self,
        executable_path: Path,
        *,
        status: str = "built",
    ):
        executable_path.parent.mkdir(parents=True, exist_ok=True)
        executable_path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")

        def runner(receipt_path: Path) -> tuple[int, dict[str, object]]:
            receipt = {
                "schema_version": "apxm.reference-host-build-receipt.v1",
                "semantic_owner": "agents",
                "status": status,
                "build_identity": {
                    "owner_revision": "a" * 40,
                    "release_manifest_path": str(
                        self.module.RELEASE_MANIFEST_PATH.relative_to(self.module.REPOSITORY_ROOT)
                    ),
                    "execution_manifest_path": str(
                        self.module.EXECUTION_MANIFEST_PATH.relative_to(
                            self.module.REPOSITORY_ROOT
                        )
                    ),
                    "binary_platform": {
                        "selection": "native-host-default",
                        "target": None,
                        "system": "darwin",
                        "machine": "arm64",
                        "platform_tag": "macosx-15.0-arm64",
                    },
                },
                "output": {"path": str(executable_path), "digest": self.module.file_digest(executable_path)},
                "reference_host_release": {"cohort": {"revision": "a" * 40}},
            }
            receipt_path.parent.mkdir(parents=True, exist_ok=True)
            receipt_path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            return (0 if status == "built" else 1), receipt

        return runner

    def make_in_flight_drain_probe(
        self, startup_input: dict[str, object], runtime_evidence: dict[str, object]
    ) -> dict[str, object]:
        return {
            "schema_version": self.module.LIFECYCLE_PROBE_SCHEMA,
            "semantic_owner": "agents",
            "case": self.module.IN_FLIGHT_DRAIN_PROBE,
            "transition": "stop_admission_then_finish_in_flight",
            "in_flight_before_drain": self.module.host_readiness(startup_input, "ready", 1),
            "drain_response": self.module.host_readiness(startup_input, "draining", 1),
            "completion_response": {
                "schema_version": self.module.HOST_SCHEMA,
                "status": "committed",
                "invocation_id": "probe.invocation.1",
                "runtime_evidence": runtime_evidence,
            },
            "terminal_readiness": self.module.host_readiness(startup_input, "stopped"),
        }

    def test_complete_receipt_records_in_flight_drain_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            temp_dir = Path(temp_dir_name)
            startup_input_path, startup_input = self.make_startup_input(temp_dir)
            ready = self.module.host_readiness(startup_input, "ready")
            stopped = self.module.host_readiness(startup_input, "stopped")
            draining = self.module.host_readiness(startup_input, "draining")
            runtime_evidence = self.make_runtime_evidence("invocation.1")
            committed = {
                "schema_version": self.module.HOST_SCHEMA,
                "status": "committed",
                "invocation_id": "invocation.1",
                "result": {"schema_version": "apxm.session-output.v1", "root": None},
                "runtime_evidence": runtime_evidence,
            }
            shutdown = {
                "schema_version": self.module.HOST_SCHEMA,
                "status": "shutdown",
                "reason": "shutdown_terminal",
                "readiness": stopped,
            }
            restarted = {
                "schema_version": self.module.HOST_SCHEMA,
                "status": "restarted",
                "recovery": {
                    "contract": "reconcile_from_runtime_evidence",
                    "status": "reconciled",
                    "runtime_evidence": runtime_evidence,
                },
                "readiness": ready,
            }
            factory = ScriptedTransportFactory(
                [
                    [self.module.host_readiness_response(startup_input, "ready")],
                    [committed],
                    [
                        {
                            "schema_version": self.module.HOST_SCHEMA,
                            "status": "rejected",
                            "error": {"code": "provenance_mismatch"},
                            "readiness": ready,
                        }
                    ],
                    [
                        {
                            "schema_version": self.module.HOST_SCHEMA,
                            "status": "rejected",
                            "error": {"code": "invalid_air"},
                            "readiness": ready,
                        }
                    ],
                    [
                        self.module.host_readiness_response(startup_input, "draining"),
                        {
                            "schema_version": self.module.HOST_SCHEMA,
                            "status": "rejected",
                            "error": {"code": "host_not_accepting"},
                            "readiness": draining,
                        },
                    ],
                    [
                        {
                            "schema_version": self.module.HOST_SCHEMA,
                            "status": "cancelled",
                            "reason": "cancelled_before_admission",
                            "readiness": stopped,
                        }
                    ],
                    [shutdown],
                    [committed, shutdown, restarted],
                    [
                        {
                            "schema_version": self.module.HOST_SCHEMA,
                            "status": "revoked",
                            "reason": "admission_revoked",
                            "readiness": stopped,
                        },
                        {
                            "schema_version": self.module.HOST_SCHEMA,
                            "status": "rejected",
                            "error": {"code": "admission_revoked"},
                            "readiness": stopped,
                        },
                    ],
                    [
                        {
                            "schema_version": self.module.HOST_SCHEMA,
                            "status": "rejected",
                            "error": {"code": "invalid_request_schema"},
                            "readiness": ready,
                        },
                        {
                            "schema_version": self.module.HOST_SCHEMA,
                            "status": "rejected",
                            "error": {"code": "unknown_operation"},
                            "readiness": ready,
                        },
                        {
                            "schema_version": self.module.HOST_SCHEMA,
                            "status": "rejected",
                            "error": {"code": "missing_admission"},
                            "readiness": ready,
                        },
                    ],
                ]
            )
            probe = self.make_in_flight_drain_probe(startup_input, runtime_evidence)
            build_receipt_path = temp_dir / "build-receipt.json"
            receipt_path = temp_dir / "lifecycle-receipt.json"
            executable_path = temp_dir / "target" / "release" / "apxm-reference-host"

            exit_code, receipt = self.module.write_reference_host_lifecycle_receipt(
                receipt_path=receipt_path,
                build_receipt_path=build_receipt_path,
                startup_input_path=startup_input_path,
                build_receipt_runner=self.make_build_receipt_runner(executable_path),
                transport_factory=factory,
                lifecycle_probe_runner=lambda _executable, _startup: probe,
            )
            persisted = json.loads(receipt_path.read_text(encoding="utf-8"))

        self.assertEqual(exit_code, 0)
        self.assertEqual(receipt["status"], "complete")
        self.assertEqual(receipt, persisted)
        self.assertEqual(factory.call_count, 10)
        self.assertEqual(receipt["build_identity"]["owner_revision"], "a" * 40)
        self.assertEqual(receipt["coverage"]["remaining_release_cases"], [])
        self.assertEqual(receipt["coverage"]["remaining_release_case_blockers"], [])
        self.assertEqual(
            receipt["lifecycle_outcomes"]["readiness"]["ready_after_restart_recovery"]["state"],
            "ready",
        )
        self.assertEqual(
            receipt["lifecycle_outcomes"]["admission"]["negative_provenance_rejection"][
                "error_code"
            ],
            "provenance_mismatch",
        )
        self.assertEqual(
            receipt["lifecycle_outcomes"]["admission"]["boundary_fail_closed"]["error_codes"],
            ["invalid_request_schema", "unknown_operation", "missing_admission"],
        )
        self.assertEqual(
            receipt["coverage"]["covered_release_cases"],
            [
                "cancellation_before_admission",
                "drain_shutdown_after_in_flight_completion",
                "explicit_shutdown_terminal_state",
                "restart_recovery_from_runtime_evidence",
                "revocation_before_dispatch",
                "boundary_fail_closed",
            ],
        )
        self.assertEqual(
            [case["name"] for case in receipt["cases"]],
            [
                "readiness",
                "positive_commit_minimal_air",
                "negative_admission_provenance_rejection",
                "invalid_air_failure",
                "drain_rejection_before_dispatch",
                "drain_shutdown_after_in_flight_completion",
                "cancellation_before_admission",
                "explicit_shutdown_terminal_state",
                "restart_recovery_from_runtime_evidence",
                "revocation_before_dispatch",
                "boundary_fail_closed",
            ],
        )
        drain_case = next(
            case for case in receipt["cases"] if case["name"] == "drain_shutdown_after_in_flight_completion"
        )
        self.assertEqual(drain_case["in_flight_before_drain"], 1)
        self.assertEqual(drain_case["terminal_state_after_in_flight"], "stopped")
        self.assertEqual(drain_case["runtime_evidence_terminal_kind"], "invocation.committed")

    def test_in_flight_drain_probe_fails_closed_on_terminal_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            temp_dir = Path(temp_dir_name)
            startup_input_path, startup_input = self.make_startup_input(temp_dir)
            bad_probe = {
                "schema_version": self.module.LIFECYCLE_PROBE_SCHEMA,
                "semantic_owner": "agents",
                "case": self.module.IN_FLIGHT_DRAIN_PROBE,
                "transition": "stop_admission_then_finish_in_flight",
                "in_flight_before_drain": self.module.host_readiness(startup_input, "ready", 1),
                "drain_response": self.module.host_readiness(startup_input, "draining", 1),
                "completion_response": {
                    "schema_version": self.module.HOST_SCHEMA,
                    "status": "committed",
                    "runtime_evidence": self.make_runtime_evidence("probe.invocation.1"),
                },
                "terminal_readiness": self.module.host_readiness(startup_input, "draining", 0),
            }

            with self.assertRaises(self.module.LifecycleReceiptError) as raised:
                self.module.run_case_in_flight_drain(
                    executable_path=temp_dir / "apxm-reference-host",
                    startup_input_path=startup_input_path,
                    startup_input=startup_input,
                    lifecycle_probe_runner=lambda _executable, _startup: bad_probe,
                )

        self.assertEqual(raised.exception.code, "case_failed")

    def test_missing_startup_input_fails_closed_before_build(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            temp_dir = Path(temp_dir_name)
            receipt_path = temp_dir / "lifecycle-receipt.json"
            build_called = False

            def unexpected_build(_receipt_path: Path):
                nonlocal build_called
                build_called = True
                return 0, {}

            exit_code, receipt = self.module.write_reference_host_lifecycle_receipt(
                receipt_path=receipt_path,
                build_receipt_path=temp_dir / "build-receipt.json",
                startup_input_path=temp_dir / "missing-startup-input.json",
                build_receipt_runner=unexpected_build,
            )

        self.assertEqual(exit_code, 1)
        self.assertFalse(build_called)
        self.assertEqual(receipt["status"], "missing_startup_input")
        self.assertEqual(receipt["error"]["code"], "missing_startup_input")

    def test_case_failure_is_reported_with_the_failed_case_name(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            temp_dir = Path(temp_dir_name)
            startup_input_path, startup_input = self.make_startup_input(temp_dir)
            wrong_ready = self.module.host_readiness(startup_input, "starting")
            factory = ScriptedTransportFactory(
                [[self.module.host_readiness_response(startup_input, "starting")]]
            )
            receipt_path = temp_dir / "lifecycle-receipt.json"
            executable_path = temp_dir / "target" / "release" / "apxm-reference-host"

            exit_code, receipt = self.module.write_reference_host_lifecycle_receipt(
                receipt_path=receipt_path,
                build_receipt_path=temp_dir / "build-receipt.json",
                startup_input_path=startup_input_path,
                build_receipt_runner=self.make_build_receipt_runner(executable_path),
                transport_factory=factory,
            )

        self.assertEqual(exit_code, 1)
        self.assertEqual(receipt["status"], "case_failed")
        self.assertEqual(receipt["error"]["code"], "case_failed")
        self.assertEqual(receipt["error"]["failed_cases"], ["readiness"])

    def test_missing_build_identity_fails_closed_before_transport(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            temp_dir = Path(temp_dir_name)
            startup_input_path, _startup_input = self.make_startup_input(temp_dir)
            receipt_path = temp_dir / "lifecycle-receipt.json"
            executable_path = temp_dir / "target" / "release" / "apxm-reference-host"
            executable_path.parent.mkdir(parents=True, exist_ok=True)
            executable_path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")

            def build_runner(receipt_file: Path):
                receipt = {
                    "schema_version": "apxm.reference-host-build-receipt.v1",
                    "semantic_owner": "agents",
                    "status": "built",
                    "output": {
                        "path": str(executable_path),
                        "digest": self.module.file_digest(executable_path),
                    },
                    "reference_host_release": {"cohort": {"revision": "a" * 40}},
                }
                receipt_file.parent.mkdir(parents=True, exist_ok=True)
                receipt_file.write_text(
                    json.dumps(receipt, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8",
                )
                return 0, receipt

            exit_code, receipt = self.module.write_reference_host_lifecycle_receipt(
                receipt_path=receipt_path,
                build_receipt_path=temp_dir / "build-receipt.json",
                startup_input_path=startup_input_path,
                build_receipt_runner=build_runner,
                transport_factory=ScriptedTransportFactory([]),
            )

        self.assertEqual(exit_code, 1)
        self.assertEqual(receipt["status"], "invalid_build_receipt")
        self.assertEqual(receipt["error"]["code"], "invalid_build_receipt")


if __name__ == "__main__":
    unittest.main()
