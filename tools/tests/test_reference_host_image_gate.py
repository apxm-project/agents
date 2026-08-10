"""Adversarial checks for the dynamic reference-host image receipt."""

from __future__ import annotations

import copy
import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools/scripts/reference_host_image_gate.py"


def load_gate():
    spec = importlib.util.spec_from_file_location("reference_host_image_gate", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load reference-host image gate")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReferenceHostImageGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.gate = load_gate()
        cls.labels = {
            "io.apxm.reference-host-release-manifest-digest": "sha256:" + "1" * 64,
            "io.apxm.reviewed-carrier-revision": cls.gate.REVIEWED_CARRIER_REVISION,
            "org.opencontainers.image.revision": cls.gate.SOURCE_REVISION,
            "org.opencontainers.image.source": "https://github.com/apxm-project/agents.git",
            "org.opencontainers.image.title": "APXM reference host",
        }
        cls.gate.exact_gate_files = lambda _revision: [  # type: ignore[assignment]
            {"path": "gate", "digest": "sha256:" + "2" * 64}
        ]
        cls.gate.expected_labels = lambda: cls.labels  # type: ignore[assignment]

    def receipt(self) -> dict:
        image_id = "sha256:" + "3" * 64
        return {
            "schema_version": "apxm.reference-host-image-build-receipt.v1",
            "semantic_owner": "agents",
            "status": "complete",
            "source_revision": self.gate.SOURCE_REVISION,
            "reviewed_carrier_revision": self.gate.REVIEWED_CARRIER_REVISION,
            "recipe_revision": "4" * 40,
            "gate_revision": "4" * 40,
            "gate_files": [{"path": "gate", "digest": "sha256:" + "2" * 64}],
            "image_manifest": {
                "path": "deploy/reference-host/image-manifest.v1.json",
                "semantic_digest": "sha256:" + "5" * 64,
                "exact_bytes_digest": "sha256:" + "6" * 64,
            },
            "build": {
                "platform": "linux/arm64",
                "no_cache": True,
                "provenance": False,
                "run_count": 2,
                "image_ids": [image_id, image_id],
                "validated_archive_count": 2,
                "reproducible": True,
                "completion_evidence": [
                    "hash_valid_docker_oci_archive",
                    "docker_archive_load",
                    "loaded_engine_image_inspection",
                ],
            },
            "inspection": {
                "image_id": image_id,
                "os": "linux",
                "architecture": "arm64",
                "user": "65532:65532",
                "entrypoint": ["/usr/local/bin/apxm-reference-host"],
                "labels": self.labels,
            },
            "binary": {
                "path": "/usr/local/bin/apxm-reference-host",
                "exact_bytes_digest": "sha256:" + "7" * 64,
                "elf_class": "ELF64",
                "elf_type": "PIE",
                "elf_machine": "AArch64",
            },
            "lifecycle": {
                "required_cases": ["positive_commit_minimal_air", "boundary_fail_closed"],
                "executed_cases": ["positive_commit_minimal_air", "boundary_fail_closed"],
                "passed_case_count": 2,
            },
        }

    def test_complete_observation_is_accepted(self) -> None:
        self.gate.validate_receipt_payload(self.receipt())

    def test_forged_oci_identity_is_rejected(self) -> None:
        receipt = self.receipt()
        receipt["inspection"]["image_id"] = "sha256:" + "8" * 64
        with self.assertRaisesRegex(self.gate.ImageGateError, "different image"):
            self.gate.validate_receipt_payload(receipt)

    def test_non_reproducible_second_build_is_rejected(self) -> None:
        receipt = self.receipt()
        receipt["build"]["image_ids"][1] = "sha256:" + "8" * 64
        with self.assertRaisesRegex(self.gate.ImageGateError, "reproducible"):
            self.gate.validate_receipt_payload(receipt)

    def test_missing_machine_build_completion_is_rejected(self) -> None:
        receipt = self.receipt()
        receipt["build"]["completion_evidence"] = ["hash_valid_docker_oci_archive"]
        with self.assertRaisesRegex(self.gate.ImageGateError, "completion evidence"):
            self.gate.validate_receipt_payload(receipt)

    def test_incomplete_archive_count_is_rejected(self) -> None:
        receipt = self.receipt()
        receipt["build"]["validated_archive_count"] = 1
        with self.assertRaisesRegex(self.gate.ImageGateError, "two complete image archives"):
            self.gate.validate_receipt_payload(receipt)

    def test_forged_labels_are_rejected(self) -> None:
        receipt = self.receipt()
        receipt["inspection"]["labels"] = copy.deepcopy(self.labels)
        receipt["inspection"]["labels"]["org.opencontainers.image.revision"] = "0" * 40
        with self.assertRaisesRegex(self.gate.ImageGateError, "labels"):
            self.gate.validate_receipt_payload(receipt)

    def test_wrong_runtime_user_is_rejected(self) -> None:
        receipt = self.receipt()
        receipt["inspection"]["user"] = "0:0"
        with self.assertRaisesRegex(self.gate.ImageGateError, "User"):
            self.gate.validate_receipt_payload(receipt)

    def test_forged_elf_machine_is_rejected(self) -> None:
        receipt = self.receipt()
        receipt["binary"]["elf_machine"] = "x86-64"
        with self.assertRaisesRegex(self.gate.ImageGateError, "ELF machine"):
            self.gate.validate_receipt_payload(receipt)

    def test_incomplete_lifecycle_coverage_is_rejected(self) -> None:
        receipt = self.receipt()
        receipt["lifecycle"]["executed_cases"] = ["positive_commit_minimal_air"]
        receipt["lifecycle"]["passed_case_count"] = 1
        with self.assertRaisesRegex(self.gate.ImageGateError, "coverage"):
            self.gate.validate_receipt_payload(receipt)

    def test_forged_lifecycle_pass_count_is_rejected(self) -> None:
        receipt = self.receipt()
        receipt["lifecycle"]["passed_case_count"] = 99
        with self.assertRaisesRegex(self.gate.ImageGateError, "pass count"):
            self.gate.validate_receipt_payload(receipt)


if __name__ == "__main__":
    unittest.main()
