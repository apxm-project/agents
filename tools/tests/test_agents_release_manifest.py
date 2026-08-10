"""Focused checks for the committed Agents owner release cohort."""

from __future__ import annotations

import copy
import importlib.util
import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECKER_PATH = ROOT / "tools" / "scripts" / "check_agents_release_manifest.py"
MANIFEST_PATH = ROOT / "release" / "manifests" / "agents-release.v1.json"
EVIDENCE_PATH = ROOT / "evidence" / "release" / "agents-release-evidence.v1.json"


def load_checker():
    spec = importlib.util.spec_from_file_location("check_agents_release_manifest", CHECKER_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load owner release checker")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AgentsReleaseManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.checker = load_checker()
        cls.manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))

    def test_committed_owner_cohort_is_exact(self) -> None:
        result = self.checker.validate_manifest(require_clean=False)
        self.assertEqual(result["status"], "pass")
        self.assertEqual(self.manifest["semantic_owner"], "agents")
        revision = self.manifest["source"]["revision"]
        self.assertRegex(revision, r"[0-9a-f]{40}")
        artifact_revision = self.manifest["reference_host_image"]["artifact_revision"]
        self.assertEqual(
            self.manifest["release_id"],
            f"apxm-agents-{revision[:12]}-arm64-{artifact_revision[:12]}",
        )
        self.assertEqual(
            self.manifest["reference_host_release"]["profile_cohort"],
            ["embedded", "reference-host"],
        )

    def test_mutated_source_digest_fails_closed(self) -> None:
        mutated = copy.deepcopy(self.manifest)
        mutated["source"]["tree_digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(
            self.checker.ReleaseManifestError, "differs from committed source"
        ):
            self.checker.validate_manifest_payload(mutated, manifest_path=MANIFEST_PATH)

    def test_mutated_reference_host_digest_fails_closed(self) -> None:
        mutated = copy.deepcopy(self.manifest)
        mutated["reference_host_release"]["golden_vectors"][0]["exact_bytes_digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(
            self.checker.ReleaseManifestError, "differs from committed source"
        ):
            self.checker.validate_manifest_payload(mutated, manifest_path=MANIFEST_PATH)

    def test_linux_arm64_image_is_exactly_bound(self) -> None:
        image = self.manifest["reference_host_image"]
        self.assertEqual(
            image["platform"],
            {
                "os": "linux",
                "architecture": "arm64",
                "elf_machine": "AArch64",
                "rust_target": "aarch64-unknown-linux-gnu",
            },
        )
        self.assertEqual(image["built_artifact"]["elf_machine"], "AArch64")

    def test_mutated_image_manifest_digest_fails_closed(self) -> None:
        mutated = copy.deepcopy(self.manifest)
        mutated["reference_host_image"]["manifest"]["exact_bytes_digest"] = (
            "sha256:" + "0" * 64
        )
        with self.assertRaisesRegex(
            self.checker.ReleaseManifestError, "differs from committed source"
        ):
            self.checker.validate_manifest_payload(mutated, manifest_path=MANIFEST_PATH)

    def test_mutated_image_id_fails_closed(self) -> None:
        mutated = copy.deepcopy(self.manifest)
        mutated["artifacts"][1]["digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(
            self.checker.ReleaseManifestError, "differs from committed source"
        ):
            self.checker.validate_manifest_payload(mutated, manifest_path=MANIFEST_PATH)

    def test_unattested_trust_claims_are_not_in_cohort(self) -> None:
        encoded = json.dumps(self.manifest, sort_keys=True).lower()
        for term in ("signature", "trust root", "pki"):
            self.assertNotIn(term, encoded)

    def test_evidence_is_derived_from_the_manifest(self) -> None:
        evidence = json.loads(EVIDENCE_PATH.read_text(encoding="utf-8"))
        self.assertEqual(evidence["release_id"], self.manifest["release_id"])
        self.assertEqual(evidence["reference_host_release"], self.manifest["reference_host_release"])
        self.assertEqual(evidence["reference_host_image"], self.manifest["reference_host_image"])
        self.assertEqual(
            evidence["verification"]["runtime_execution"],
            "linux_arm64_lifecycle_vectors_passed",
        )

    def test_clean_tree_is_required_by_default(self) -> None:
        original = self.checker.git_text
        self.checker.git_text = lambda *_args: "uncommitted"  # type: ignore[assignment]
        try:
            with self.assertRaisesRegex(self.checker.ReleaseManifestError, "checkout is dirty"):
                self.checker.validate_manifest()
        finally:
            self.checker.git_text = original


if __name__ == "__main__":
    unittest.main()
