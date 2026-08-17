"""Validate APXM owner P80 wrapper plans and structured terminal output."""

from __future__ import annotations

import unittest
import tomllib
import json
from pathlib import Path

from tools.scripts.p80_owner import PHASES, _last_json_object, _strict_qualification, final_result


class P80OwnerTests(unittest.TestCase):
    def test_every_phase_runs_a_release_gate_once_without_recursive_p80_calls(self) -> None:
        self.assertEqual(set(PHASES), {
            "p80-e2e",
            "p80-journey-c",
            "p80-negative-recovery",
            "p80-journey-g",
            "p80-journey-h",
            "p80-restart-reopen",
        })
        for commands in PHASES.values():
            self.assertEqual(commands[0], "release-qualification")
            self.assertEqual(commands.count("release-qualification"), 1)
            self.assertFalse(any(command.startswith("p80-") for command in commands))

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
        result = final_result("p80-e2e", qualification=qualification)
        self.assertEqual(result["qualification"], qualification)
        self.assertEqual(result["schema"], "apxm.agents.owner-phase-result.v1")
        self.assertNotIn("acceptance_index", result)

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
            self.assertEqual(run, f"python tools/scripts/p80_owner.py {phase}")
            self.assertNotIn("&&", run)


if __name__ == "__main__":
    unittest.main()
