"""The source/compiler acceptance gate stays exact, ordered, and fail-closed."""

from __future__ import annotations

import signal
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.scripts import check_source_compiler_acceptance


class SourceCompilerAcceptanceTests(unittest.TestCase):
    def test_cli_codegen_uses_the_checked_in_common_schema_snapshot(self) -> None:
        build_script = (
            check_source_compiler_acceptance.REPO_ROOT
            / "crates"
            / "tools"
            / "cli"
            / "build.rs"
        ).read_text(encoding="utf-8")
        self.assertIn(
            "../../machine/program/tests/fixtures/contracts/"
            "apxm.contract-common.v1.json",
            build_script,
        )
        self.assertNotIn("workspace_root", build_script)
        self.assertNotIn('.join("contracts")', build_script)

    def test_acceptance_steps_are_unique_and_exact(self) -> None:
        steps = check_source_compiler_acceptance.acceptance_steps()
        self.assertEqual(
            [step.gate_id for step in steps],
            [
                "check-frontend-codegen",
                "check-frontend-surface",
                "test-source-port",
                "test-program-source",
                "test-compiler",
                "check-frontend-parity",
                "test-typescript-frontend",
                "check-source-compiler-boundary",
                "compile-service-canonical",
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
                logs_dir.joinpath("check-frontend-codegen.log").is_file(),
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
        self.assertTrue(
            all(step["log_digest"].startswith("sha256:") for step in report["steps"]),
            "every step records a content digest for its log",
        )

    def test_run_acceptance_records_failures_without_dropping_later_steps(self) -> None:
        failing_gate = "test-program-source"
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

    def test_run_acceptance_reports_toolchain_block_without_calling_it_a_failure(self) -> None:
        blocked_gate = "test-program-source"

        with tempfile.TemporaryDirectory() as temporary:
            logs_dir = Path(temporary) / "logs"

            def blocked(command, cwd, capture_output, text):  # noqa: ANN001
                self.assertEqual(cwd, check_source_compiler_acceptance.REPO_ROOT)
                if command[2] == blocked_gate:
                    return subprocess.CompletedProcess(
                        command,
                        2,
                        stdout="",
                        stderr="error: APXM native toolchain readiness failed.\n",
                    )
                return subprocess.CompletedProcess(command, 0, stdout="ok\n", stderr="")

            report = check_source_compiler_acceptance.run_acceptance(
                logs_dir, runner=blocked
            )

        self.assertEqual(report["overall_status"], "blocked")
        recorded = {step["gate_id"]: step for step in report["steps"]}
        self.assertEqual(recorded[blocked_gate]["status"], "blocked")
        self.assertEqual(recorded[blocked_gate]["block_reason"], "toolchain_unavailable")
        self.assertNotEqual(report["overall_status"], "passed")

    def test_unrelated_command_error_does_not_reclassify_an_ordinary_failure(self) -> None:
        result = check_source_compiler_acceptance.classify_step(
            subprocess.CompletedProcess(
                ("dekk", "agents", "test-program"),
                1,
                stdout="",
                stderr="source assertion failed: command not found in fixture",
            )
        )

        self.assertEqual(result, ("failed", None))

    def test_toolchain_detail_without_readiness_header_stays_a_failure(self) -> None:
        result = check_source_compiler_acceptance.classify_step(
            subprocess.CompletedProcess(
                ("dekk", "agents", "test-program"),
                1,
                stdout="",
                stderr="source assertion mentions matching Linux arm64 compiler/sysroot",
            )
        )

        self.assertEqual(result, ("failed", None))

    def test_signal_only_native_crash_is_reported_as_blocked(self) -> None:
        result = check_source_compiler_acceptance.classify_step(
            subprocess.CompletedProcess(
                ("dekk", "agents", "test-program"),
                256 - signal.SIGSEGV,
                stdout="",
                stderr="",
            )
        )

        self.assertEqual(result, ("blocked", "native_toolchain_crash"))

    def test_native_crash_with_assertion_output_remains_a_failure(self) -> None:
        result = check_source_compiler_acceptance.classify_step(
            subprocess.CompletedProcess(
                ("dekk", "agents", "test-program"),
                256 - signal.SIGSEGV,
                stdout="source assertion failed",
                stderr="",
            )
        )

        self.assertEqual(result, ("failed", None))

    def test_a_real_failure_takes_precedence_over_a_blocked_step(self) -> None:
        def mixed(command, cwd, capture_output, text):  # noqa: ANN001
            if command[2] == "test-program":
                return subprocess.CompletedProcess(
                    command,
                    2,
                    stdout="",
                    stderr="error: APXM native toolchain readiness failed.",
                )
            if command[2] == "test-compiler":
                return subprocess.CompletedProcess(
                    command,
                    1,
                    stdout="",
                    stderr="source assertion failed",
                )
            return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

        with tempfile.TemporaryDirectory() as temporary:
            report = check_source_compiler_acceptance.run_acceptance(
                Path(temporary) / "logs", runner=mixed
            )

        self.assertEqual(report["overall_status"], "failed")

    def test_run_acceptance_reports_missing_launcher_as_blocked(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            logs_dir = Path(temporary) / "logs"

            def missing_launcher(command, cwd, capture_output, text):  # noqa: ANN001
                raise FileNotFoundError(2, "No such file or directory", command[0])

            report = check_source_compiler_acceptance.run_acceptance(
                logs_dir, runner=missing_launcher
            )

        self.assertEqual(report["overall_status"], "blocked")
        self.assertTrue(
            all(step["status"] == "blocked" for step in report["steps"])
        )
        self.assertTrue(
            all(step["block_reason"] == "environment_unavailable" for step in report["steps"])
        )
        self.assertTrue(all(step["returncode"] is None for step in report["steps"]))

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

    def test_run_step_records_unavailable_authority_as_a_typed_failure(self) -> None:
        step = check_source_compiler_acceptance.acceptance_steps()[0]

        def unavailable(*args, **kwargs):  # noqa: ANN002, ANN003
            raise OSError("dekk unavailable")

        with tempfile.TemporaryDirectory() as temporary:
            result = check_source_compiler_acceptance.run_step(
                step,
                Path(temporary),
                runner=unavailable,
            )

        self.assertEqual(result.status, "blocked")
        self.assertIsNone(result.returncode)
        self.assertEqual(result.error, "dekk unavailable")
        self.assertEqual(result.block_reason, "environment_unavailable")
        self.assertTrue(result.log_digest.startswith("sha256:"))

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

    def test_main_fails_closed_when_execution_is_blocked(self) -> None:
        report = {
            "schema_version": "apxm.source-compiler-acceptance.v1",
            "generated_at": "2026-08-04T00:00:00+00:00",
            "issue_scope": {"issue": 35, "plan_tags": ["P-002", "G2"]},
            "overall_status": "blocked",
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


if __name__ == "__main__":
    unittest.main()
