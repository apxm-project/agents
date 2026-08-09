"""Executable, offline parity gate coverage for the reference-host contract cohort."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
GATE = REPOSITORY_ROOT / "contracts" / "tools" / "check_reference_host_contract_parity.py"


class ReferenceHostContractParityTests(unittest.TestCase):
    def run_gate_in_copy(self, mutate) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copytree(REPOSITORY_ROOT / "contracts", root / "contracts")
            source = REPOSITORY_ROOT / "crates" / "tools" / "cli" / "src" / "bin" / "reference_host.rs"
            source_target = root / "crates" / "tools" / "cli" / "src" / "bin" / "reference_host.rs"
            source_target.parent.mkdir(parents=True)
            shutil.copy2(source, source_target)
            harness = REPOSITORY_ROOT / "crates" / "tools" / "cli" / "tests" / "reference_host_jsonl.rs"
            harness_target = root / "crates" / "tools" / "cli" / "tests" / "reference_host_jsonl.rs"
            harness_target.parent.mkdir(parents=True)
            shutil.copy2(harness, harness_target)
            mutate(root)
            return subprocess.run(
                [sys.executable, str(GATE), "--root", str(root)],
                text=True,
                capture_output=True,
                check=False,
            )

    def test_gate_is_executable_and_passes_without_credentials_or_linux(self) -> None:
        result = subprocess.run(
            [sys.executable, str(GATE), "--root", str(REPOSITORY_ROOT)],
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS: reference-host contract parity", result.stdout)

    def test_gate_fails_closed_on_schema_copy_drift(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "contracts" / "reference-host" / "schemas" / "apxm.runtime.host-response.v1.json"
            path.write_bytes(path.read_bytes() + b"\n")

        result = self.run_gate_in_copy(mutate)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FAIL-CLOSED", result.stderr)
        self.assertIn("schema bytes drifted", result.stderr)

    def test_gate_fails_closed_on_manifest_digest_drift(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "contracts" / "reference-host" / "manifests" / "apxm.reference-host-execution-manifest.v1.json"
            text = path.read_text(encoding="utf-8")
            path.write_text(
                text.replace(
                    "sha256:abcd9707efe46826d858318bf0ae4e4e40d3839cde66c302d8805e4c6bc6577f",
                    "sha256:" + "0" * 64,
                ),
                encoding="utf-8",
            )

        result = self.run_gate_in_copy(mutate)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FAIL-CLOSED", result.stderr)
        self.assertIn("digest drifted", result.stderr)

    def test_gate_fails_closed_on_reference_host_constant_drift(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "crates" / "tools" / "cli" / "src" / "bin" / "reference_host.rs"
            text = path.read_text(encoding="utf-8")
            path.write_text(text.replace('const PRIVATE_TRANSPORT_PROTOCOL: &str = "jsonl-unix-stream";', 'const PRIVATE_TRANSPORT_PROTOCOL: &str = "wrong-protocol";'), encoding="utf-8")

        result = self.run_gate_in_copy(mutate)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FAIL-CLOSED", result.stderr)
        self.assertIn("PRIVATE_TRANSPORT_PROTOCOL drifted", result.stderr)

    def test_gate_fails_closed_on_release_parity_evidence_drift(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "contracts" / "reference-host" / "manifests" / "apxm.reference-host-release-manifest.v1.json"
            release = json.loads(path.read_text(encoding="utf-8"))
            release["executable_parity_evidence"]["harness"]["digest"] = "sha256:" + "0" * 64
            path.write_text(json.dumps(release, indent=2) + "\n", encoding="utf-8")

        result = self.run_gate_in_copy(mutate)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FAIL-CLOSED", result.stderr)
        self.assertIn("executable_parity_evidence drifted", result.stderr)


if __name__ == "__main__":
    unittest.main()
