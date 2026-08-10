"""Reference-host Linux/arm64 image recipe remains exact and fail-closed."""

from __future__ import annotations

import copy
import importlib.util
import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools/scripts/check_reference_host_image.py"
MANIFEST = ROOT / "deploy/reference-host/image-manifest.v1.json"


def load_checker():
    spec = importlib.util.spec_from_file_location("check_reference_host_image", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load reference-host image checker")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReferenceHostImageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.checker = load_checker()
        cls.manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))

    def test_exact_owner_recipe_is_valid(self) -> None:
        result = self.checker.validate(require_clean=False)
        self.assertRegex(result["manifest_digest"], r"sha256:[0-9a-f]{64}")
        self.assertEqual(self.manifest["semantic_owner"], "agents")
        self.assertEqual(self.manifest["platform"]["architecture"], "arm64")
        self.assertEqual(self.manifest["runtime"]["source_mount"], "absent")

    def test_dockerfile_substitution_fails_closed(self) -> None:
        mutated = copy.deepcopy(self.manifest)
        mutated["build_recipe"]["dockerfile_path"] = "README.md"
        with self.assertRaisesRegex(
            self.checker.ImageManifestError, "drifted from owner inputs"
        ):
            self.checker.validate_payload(mutated)

    def test_recipe_digest_mutation_fails_closed(self) -> None:
        mutated = copy.deepcopy(self.manifest)
        mutated["build_recipe"]["dockerfile_digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(
            self.checker.ImageManifestError, "drifted from owner inputs"
        ):
            self.checker.validate_payload(mutated)

    def test_clean_tree_is_required_by_default(self) -> None:
        original = self.checker.git_text
        self.checker.git_text = lambda *_args: "dirty"  # type: ignore[assignment]
        try:
            with self.assertRaisesRegex(
                self.checker.ImageManifestError, "owner checkout is dirty"
            ):
                self.checker.validate()
        finally:
            self.checker.git_text = original


if __name__ == "__main__":
    unittest.main()
