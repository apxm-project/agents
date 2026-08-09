"""Generic negative coverage for reference-host downstream neutrality."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPOSITORY_ROOT / "contracts" / "tools" / "check_reference_host_neutrality.py"


def load_module():
    spec = importlib.util.spec_from_file_location("check_reference_host_neutrality", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load reference-host neutrality scanner")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReferenceHostNeutralityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = load_module()
        cls.vector = json.loads(
            (REPOSITORY_ROOT / "contracts" / "vectors" / "apxm.reference-host.neutrality.v1.json").read_text(
                encoding="utf-8"
            )
        )

    def test_checked_in_reference_host_neutrality_is_clean(self) -> None:
        self.assertEqual(self.module.find_violations(), [])

    def test_generic_negative_cases_are_rejected(self) -> None:
        pattern = self.module._forbidden_pattern(self.vector)
        for case in self.vector["negative_cases"]:
            with self.subTest(case=case["name"]):
                self.assertEqual(
                    bool(self.module.scan_text(case["input"], pattern)),
                    case["expected_match"],
                )

    def test_scan_targets_are_exact_and_confined(self) -> None:
        mutated = dict(self.vector, scan_targets=["../../outside.txt"])
        with self.assertRaisesRegex(self.module.NeutralityError, "scan_targets drifted"):
            self.module.validate_vector(mutated)

    def test_generic_coordinate_shapes_are_rejected(self) -> None:
        pattern = self.module._forbidden_pattern(self.vector)
        for coordinate in (
            "acme-sdk",
            "@acme/host-sdk",
            "customer-service",
            "foo_client",
            "acme_client_service",
            "acme_sdk_dependency",
            "use AcmeClient::Port;",
            "use acme::client;",
            '{"dependency":"Acme"}',
            "Acme",
            "acme",
        ):
            with self.subTest(coordinate=coordinate):
                self.assertTrue(self.module.scan_text(coordinate, pattern))

    def test_product_neutral_contexts_are_not_scanned_as_prose(self) -> None:
        pattern = self.module._forbidden_pattern(self.vector)
        self.assertFalse(
            self.module.scan_text("Acme is mentioned in release prose.", pattern)
        )
        self.assertFalse(
            self.module.scan_text("use apxm_program::air::AirModule;", pattern)
        )

    def test_mutated_publication_is_rejected(self) -> None:
        target = REPOSITORY_ROOT / "contracts" / "reference-host" / "manifests" / "apxm.reference-host-release-manifest.v1.json"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in self.vector["scan_targets"]:
                destination = root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                source = REPOSITORY_ROOT / relative
                destination.write_bytes(source.read_bytes())
            (root / "contracts" / "vectors").mkdir(parents=True)
            (root / "contracts" / "reference-host" / "vectors").mkdir(parents=True)
            canonical = REPOSITORY_ROOT / "contracts" / "vectors" / "apxm.reference-host.neutrality.v1.json"
            published = REPOSITORY_ROOT / "contracts" / "reference-host" / "vectors" / "apxm.reference-host.neutrality.v1.json"
            (root / "contracts" / "vectors" / canonical.name).write_bytes(canonical.read_bytes())
            (root / "contracts" / "reference-host" / "vectors" / published.name).write_bytes(published.read_bytes())
            manifest = json.loads(target.read_text(encoding="utf-8"))
            manifest["constraints"]["downstream_product"] = "present"
            manifest_path = root / "contracts" / "reference-host" / "manifests" / target.name
            manifest_path.write_text(json.dumps(manifest) + "\n", encoding="utf-8")
            violations = self.module.find_violations(root)
            self.assertTrue(violations)


if __name__ == "__main__":
    unittest.main()
