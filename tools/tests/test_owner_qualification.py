"""Tests for the neutral APXM owner qualification and handoff surface."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import tomllib

from tools.scripts.owner_qualification import (
    EVIDENCE_ROOT,
    HANDOFF_SCHEMA,
    PHASES,
    _canonical_json,
    _classify_failure,
    _command_evidence,
    _handoff,
    _write_immutable,
)


ROOT = Path(__file__).resolve().parents[2]


def qualification(root: Path) -> dict[str, object]:
    return {
        "schema": "apxm.agents.release-qualification.v1",
        "owner": "agents",
        "evidence_root": str(root.resolve()),
        "qualified": True,
        "source_revision": "a" * 40,
        "source_descriptor_digest": "sha256:" + "b" * 64,
        "owner_descriptor_digest": "sha256:" + "c" * 64,
        "release_manifest_digest": "sha256:" + "d" * 64,
        "service_digests": {
            "compilation-service": "sha256:" + "e" * 64,
            "runtime-service": "sha256:" + "f" * 64,
        },
        "gates": [
            {"command": command, "returncode": 0}
            for command in (
                "dekk agents check",
                "dekk agents test",
                "dekk agents test-compilation-protocol",
                "dekk agents test-compilation-service",
                "dekk agents test-runtime-protocol",
                "dekk agents test-runtime-service",
            )
        ],
    }


class OwnerQualificationTests(unittest.TestCase):
    def test_declared_commands_are_neutral_and_cover_failure_restart(self) -> None:
        manifest = tomllib.loads((ROOT / ".dekk.toml").read_text(encoding="utf-8"))
        commands = manifest["commands"]
        self.assertEqual(
            commands["owner-qualification"]["run"],
            "python tools/scripts/owner_qualification.py owner-qualification",
        )
        self.assertEqual(
            commands["owner-failure-restart"]["run"],
            "python tools/scripts/owner_qualification.py owner-failure-restart",
        )
        self.assertIn("test-compilation-runtime-failure-restart", commands)
        declared = " ".join(
            commands[name]["run"]
            for name in ("owner-qualification", "owner-failure-restart")
        ).lower()
        self.assertNotRegex(declared, r"clic|p50|p80")
        self.assertIn("test-compilation-runtime-failure-restart", PHASES["owner-qualification"])

    def test_failure_evidence_binds_return_code_and_raw_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            entry = _command_evidence(
                root,
                "owner-failure-restart",
                0,
                "dekk agents test-compilation-runtime-failure-restart",
                "failed",
                1,
                b"assertion failed: durable restart\n",
            )
            self.assertEqual(entry["status"], "failed")
            self.assertEqual(entry["returncode"], 1)
            self.assertEqual(entry["failure_classification"], "code")
            output = root / str(entry["output_artifact"]["reference"])
            artifact = root / str(entry["evidence_artifact"]["reference"])
            self.assertEqual(
                entry["output_digest"],
                "sha256:" + hashlib.sha256(output.read_bytes()).hexdigest(),
            )
            self.assertEqual(json.loads(artifact.read_text(encoding="utf-8"))["status"], "failed")

    def test_handoff_is_canonical_and_write_once(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            command = _command_evidence(
                root,
                "owner-qualification",
                0,
                "dekk agents release-qualification",
                "passed",
                0,
                b"qualification output\n",
            )
            path, handoff_digest = _handoff(
                root,
                "owner-qualification",
                qualification(root),
                [command],
            )
            raw = path.read_bytes()
            document = json.loads(raw)
            self.assertEqual(raw, _canonical_json(document))
            self.assertEqual(document["schema"], HANDOFF_SCHEMA)
            self.assertEqual(
                handoff_digest,
                "sha256:" + hashlib.sha256(raw).hexdigest(),
            )
            _handoff(root, "owner-qualification", qualification(root), [command])
            with self.assertRaisesRegex(ValueError, "immutable"):
                _write_immutable(root, EVIDENCE_ROOT / "handoff" / "owner-qualification.json", b"tampered")

    def test_failure_classification_separates_resource_and_abi(self) -> None:
        self.assertEqual(_classify_failure(137, b"killed"), "resource")
        self.assertEqual(_classify_failure(1, b"error[E0514]: metadata version"), "abi")
        self.assertEqual(_classify_failure(1, b"test assertion failed"), "code")


if __name__ == "__main__":
    unittest.main()
