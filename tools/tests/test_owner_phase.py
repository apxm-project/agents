"""Validate APXM owner phase wrapper plans and structured terminal output."""

from __future__ import annotations

import unittest
import tomllib
import json
from pathlib import Path
import hashlib
import tempfile

from tools.scripts.owner_phase import (
    COMMAND_ARTIFACT_SCHEMA,
    COMMAND_EVIDENCE_SCHEMA,
    PHASES,
    _canonical_json_bytes,
    _command_evidence_entry,
    _frame_command_output,
    _last_json_object,
    _strict_command_evidence,
    _strict_qualification,
    final_result,
)


class OwnerPhaseTests(unittest.TestCase):
    def test_every_phase_runs_a_release_gate_once_without_recursive_owner_calls(self) -> None:
        self.assertEqual(set(PHASES), {
            "owner-e2e",
            "owner-compilation-runtime",
            "owner-negative-recovery",
            "owner-integrated-execution",
            "owner-protocol-clients",
            "owner-restart-reopen",
        })
        for commands in PHASES.values():
            self.assertEqual(commands[0], "release-qualification")
            self.assertEqual(commands.count("release-qualification"), 1)
            self.assertFalse(any(command.startswith("owner-") for command in commands))

    def test_terminal_result_preserves_actual_neutral_qualification(self) -> None:
        qualification = {
            "schema": "apxm.agents.release-qualification.v1",
            "owner": "agents",
            "evidence_root": str(Path(__file__).resolve().parents[2]),
            "qualified": True,
            "source_revision": "a" * 40,
            "source_descriptor_digest": "sha256:" + "a" * 64,
            "owner_descriptor_digest": "sha256:" + "b" * 64,
            "release_manifest_digest": "sha256:" + "c" * 64,
            "service_digests": {
                "compilation-service": "sha256:" + "d" * 64,
                "runtime-service": "sha256:" + "e" * 64,
            },
            "gates": [
                {"command": command, "returncode": 0}
                for command in (
                    "dekk agents check",
                    "dekk agents test",
                    "dekk agents test-compilation-protocol",
                    "dekk agents test-runtime-protocol",
                )
            ],
        }
        result = final_result("owner-e2e", qualification=qualification)
        self.assertEqual(result["qualification"], qualification)
        self.assertEqual(result["schema"], "apxm.agents.owner-phase-result.v1")
        self.assertNotIn("acceptance_index", result)

    def test_command_evidence_binds_actual_output_to_ignored_canonical_artifact(self) -> None:
        output = b"protocol output\n"
        framed = _frame_command_output(output, b"diagnostic \xff\n")
        command = "dekk agents release-qualification"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            entry = _command_evidence_entry(
                "owner-e2e", command, framed, 0, root=root
            )
            artifact = entry["evidence_artifact"]
            artifact_path = root / str(artifact["reference"])
            raw = artifact_path.read_bytes()
            document = json.loads(raw)
            self.assertEqual(raw, _canonical_json_bytes(document))
            self.assertEqual(
                document,
                {
                    "schema": COMMAND_ARTIFACT_SCHEMA,
                    "owner": "agents",
                    "command": command,
                    "status": "passed",
                    "output_digest": "sha256:" + hashlib.sha256(framed).hexdigest(),
                    "output_artifact": {
                        "reference": artifact["reference"].replace(".json", ".output"),
                        "digest": "sha256:" + hashlib.sha256(framed).hexdigest(),
                    },
                },
            )
            output_path = root / str(artifact["reference"]).replace(".json", ".output")
            self.assertEqual(output_path.read_bytes(), framed)
            output_path.write_bytes(b"tampered\n")
            entries = [
                entry,
                *[
                    _command_evidence_entry(
                        "owner-e2e", f"dekk agents {command}", b"ok\n", ordinal, root=root
                    )
                    for ordinal, command in enumerate(PHASES["owner-e2e"][1:], start=1)
                ],
            ]
            evidence = {
                "schema": COMMAND_EVIDENCE_SCHEMA,
                "owner": "agents",
                "phase": "owner-e2e",
                "status": "passed",
                "commands": entries,
            }
            with self.assertRaisesRegex(ValueError, "output artifact"):
                _strict_command_evidence(evidence, phase="owner-e2e", root=root)
            self.assertEqual(
                artifact["digest"],
                "sha256:" + hashlib.sha256(raw).hexdigest(),
            )

    def test_terminal_result_exposes_only_passed_executed_commands(self) -> None:
        qualification = {
            "schema": "apxm.agents.release-qualification.v1",
            "owner": "agents",
            "qualified": True,
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            entries = [
                _command_evidence_entry(
                    "owner-e2e", f"dekk agents {command}", b"ok\n", ordinal, root=root
                )
                for ordinal, command in enumerate(PHASES["owner-e2e"])
            ]
            evidence = {
                "schema": COMMAND_EVIDENCE_SCHEMA,
                "owner": "agents",
                "phase": "owner-e2e",
                "status": "passed",
                "commands": entries,
            }
            result = final_result(
                "owner-e2e", qualification=qualification, command_evidence=evidence, root=root
            )
            self.assertEqual(result["command_evidence"], evidence)
            self.assertTrue(result["command_evidence"]["schema"].startswith("apxm."))
            self.assertNotIn("owner_command_ledger", result)

    def test_qualification_digest_uses_utf8_canonical_json_and_rejects_nan(self) -> None:
        qualification = {"owner": "agents", "label": "café"}
        result = final_result("owner-e2e", qualification=qualification)
        self.assertEqual(
            result["qualification_digest"],
            "sha256:" + hashlib.sha256(_canonical_json_bytes(qualification)).hexdigest(),
        )
        with self.assertRaises(ValueError):
            _canonical_json_bytes({"value": float("nan")})
        with self.assertRaises(ValueError):
            final_result("owner-e2e", qualification={"value": float("nan")})

    def test_command_evidence_rejects_output_and_metadata_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            entries = [
                _command_evidence_entry(
                    "owner-e2e", f"dekk agents {command}", b"ok\n", ordinal, root=root
                )
                for ordinal, command in enumerate(PHASES["owner-e2e"])
            ]
            evidence = {
                "schema": COMMAND_EVIDENCE_SCHEMA,
                "owner": "agents",
                "phase": "owner-e2e",
                "status": "passed",
                "commands": entries,
            }
            output_path = root / str(entries[0]["output_artifact"]["reference"])
            output_path.unlink()
            output_path.symlink_to(root / "missing-output")
            with self.assertRaisesRegex(ValueError, "output artifact"):
                _strict_command_evidence(evidence, phase="owner-e2e", root=root)

            output_path.unlink()
            output_path.write_bytes(b"ok\n")
            metadata_path = root / str(entries[0]["evidence_artifact"]["reference"])
            metadata_path.unlink()
            metadata_path.symlink_to(root / "missing-metadata")
            with self.assertRaisesRegex(ValueError, "evidence artifact"):
                _strict_command_evidence(evidence, phase="owner-e2e", root=root)

    def test_command_evidence_writer_rejects_symlink_targets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            initial = _command_evidence_entry(
                "owner-e2e", "dekk agents release-qualification", b"ok\n", 0, root=root
            )
            output_path = root / str(initial["output_artifact"]["reference"])
            output_path.unlink()
            output_path.symlink_to(root / "missing-output")
            with self.assertRaisesRegex(ValueError, "regular file"):
                _command_evidence_entry(
                    "owner-e2e", "dekk agents release-qualification", b"new\n", 0, root=root
                )

            output_path.unlink()
            output_path.write_bytes(b"ok\n")
            metadata_path = root / str(initial["evidence_artifact"]["reference"])
            metadata_path.unlink()
            metadata_path.symlink_to(root / "missing-metadata")
            with self.assertRaisesRegex(ValueError, "regular file"):
                _command_evidence_entry(
                    "owner-e2e", "dekk agents release-qualification", b"new\n", 0, root=root
                )

    def test_qualification_parser_rejects_forged_or_incomplete_evidence(self) -> None:
        nested = {
            "schema": "apxm.agents.release-qualification.v1",
            "services": {"runtime-service": {"digest": "sha256:" + "a" * 64}},
            "manifest_services": [{"name": "runtime-service", "path": "artifacts/runtime", "digest": "sha256:" + "a" * 64}],
            "gates": [{"command": "dekk agents test", "returncode": 0}],
            "diagnostics": [],
        }
        rendered = "qualification log\n" + json.dumps(nested, indent=2) + "\n"
        self.assertEqual(_last_json_object(rendered), nested)
        with self.assertRaisesRegex(ValueError, "result schema"):
            _strict_qualification({"qualified": True})

        qualification = {
            "schema": "apxm.agents.release-qualification.v1",
            "owner": "agents",
            "evidence_root": str(Path(__file__).resolve().parents[2]),
            "qualified": True,
            "source_revision": "a" * 40,
            "source_descriptor_digest": "sha256:" + "a" * 64,
            "owner_descriptor_digest": "sha256:" + "b" * 64,
            "release_manifest_digest": "sha256:" + "c" * 64,
            "service_digests": {"compilation-service": "sha256:" + "d" * 64},
            "gates": [],
        }
        with self.assertRaisesRegex(ValueError, "exactly the two"):
            _strict_qualification(qualification)

    def test_dekk_manifest_uses_the_owner_wrapper(self) -> None:
        manifest = tomllib.loads((Path(__file__).resolve().parents[2] / ".dekk.toml").read_text())
        for phase in PHASES:
            run = manifest["commands"][phase]["run"]
            self.assertEqual(run, f"python tools/scripts/owner_phase.py {phase}")
            self.assertNotIn("&&", run)


if __name__ == "__main__":
    unittest.main()
