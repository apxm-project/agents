"""The source/compiler acceptance gate stays exact, ordered, and fail-closed."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.scripts import check_source_compiler_acceptance


class SourceCompilerAcceptanceTests(unittest.TestCase):
    def test_acceptance_steps_are_unique_and_exact(self) -> None:
        steps = check_source_compiler_acceptance.acceptance_steps()
        self.assertEqual(
            [step.gate_id for step in steps],
            [
                "owner-descriptor",
                "check-contract-codegen",
                "check-frontend-surface",
                "test-source-port",
                "test-program",
                "test-compiler",
                "check-frontend-parity",
                "test-external-source-package",
                "compile-service-canonical",
                "execute-canonical",
            ],
        )
        self.assertEqual(
            len({step.gate_id for step in steps}),
            len(steps),
            "each acceptance gate id is unique",
        )
        self.assertTrue(
            all(step.command[:2] == ("dekk", "agents") for step in steps),
            "the acceptance bundle uses only the repository authority CLI",
        )

    def test_run_acceptance_reports_success_when_every_gate_passes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            logs_dir = Path(temporary) / "logs"

            def succeed(command, cwd, capture_output, text):  # noqa: ANN001
                self.assertEqual(cwd, check_source_compiler_acceptance.REPO_ROOT)
                self.assertTrue(capture_output)
                self.assertTrue(text)
                return subprocess.CompletedProcess(command, 0, stdout="ok\n", stderr="")

            report = check_source_compiler_acceptance.run_acceptance(
                logs_dir, runner=succeed
            )
            self.assertTrue(
                logs_dir.joinpath("owner-descriptor.log").is_file(),
                "the acceptance run writes per-step logs beside the report",
            )

        self.assertEqual(report["schema_version"], "apxm.source-compiler-acceptance.v1")
        self.assertEqual(report["overall_status"], "passed")
        self.assertEqual(len(report["steps"]), len(check_source_compiler_acceptance.acceptance_steps()))
        self.assertTrue(
            all(step["status"] == "passed" for step in report["steps"]),
            "every successful gate is recorded as passed",
        )
        self.assertTrue(
            all(step["log_path"].endswith(".log") for step in report["steps"]),
            "every step records a log path in the report",
        )

    def test_run_acceptance_records_failures_without_dropping_later_steps(self) -> None:
        failing_gate = "test-program"
        executed: list[str] = []

        with tempfile.TemporaryDirectory() as temporary:
            logs_dir = Path(temporary) / "logs"

            def mixed(command, cwd, capture_output, text):  # noqa: ANN001
                self.assertEqual(cwd, check_source_compiler_acceptance.REPO_ROOT)
                self.assertTrue(capture_output)
                self.assertTrue(text)
                gate_id = command[2]
                executed.append(gate_id)
                return subprocess.CompletedProcess(
                    command,
                    1 if gate_id == failing_gate else 0,
                    stdout=f"{gate_id}\n",
                    stderr="",
                )

            report = check_source_compiler_acceptance.run_acceptance(
                logs_dir, runner=mixed
            )

        self.assertEqual(report["overall_status"], "failed")
        self.assertEqual(
            executed,
            [step.gate_id for step in check_source_compiler_acceptance.acceptance_steps()],
            "the report includes every gate even when one fails",
        )
        recorded = {step["gate_id"]: step for step in report["steps"]}
        self.assertEqual(recorded[failing_gate]["status"], "failed")
        self.assertEqual(recorded[failing_gate]["returncode"], 1)
        self.assertTrue(
            recorded[failing_gate]["log_path"].endswith(f"{failing_gate}.log")
        )

    def test_write_report_creates_parent_directories(self) -> None:
        report = {
            "schema_version": "apxm.source-compiler-acceptance.v1",
            "overall_status": "passed",
            "steps": [],
        }
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "nested" / "report.json"
            check_source_compiler_acceptance.write_report(report, path)
            self.assertTrue(path.is_file())
            self.assertIn('"overall_status": "passed"', path.read_text(encoding="utf-8"))

    def test_main_fails_closed_when_any_gate_fails(self) -> None:
        report = {
            "schema_version": "apxm.source-compiler-acceptance.v1",
            "generated_at": "2026-08-04T00:00:00+00:00",
            "issue_scope": {"issue": 35, "plan_tags": ["P-002", "G1"]},
            "overall_status": "failed",
            "steps": [],
        }
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "report.json"
            with patch.object(
                check_source_compiler_acceptance,
                "parse_args",
                return_value=type("Args", (), {"report_json": destination})(),
            ), patch.object(
                check_source_compiler_acceptance,
                "run_acceptance",
                return_value=report,
            ):
                self.assertEqual(check_source_compiler_acceptance.main(), 1)
                self.assertTrue(destination.is_file())


if __name__ == "__main__":
    unittest.main()
