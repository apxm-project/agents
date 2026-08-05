"""Verifier coverage for the checked-in P-018 no-build evidence."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPOSITORY_ROOT / "tools" / "scripts" / "verify_p018_no_hosted_durable.py"


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class P018NoHostedDurableVerifierTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = load_module(SCRIPT_PATH, "verify_p018_no_hosted_durable")

    def test_digest_manifest_is_non_empty_and_stays_exact(self) -> None:
        manifest = self.module.parse_digest_manifest()
        self.assertGreater(len(manifest), 0)
        report = self.module.build_report()
        self.assertEqual(report["overall_status"], "passed")
        for row in report["digests"]:
            self.assertEqual(row["status"], "passed")

    def test_verifier_covers_default_branch_authority_artifacts(self) -> None:
        manifest = self.module.parse_digest_manifest()
        self.assertIn("docs/plans/p018-hosted-durable-decision.md", manifest)
        self.assertIn("docs/plans/g3-admission-runtime-checklist.md", manifest)
        self.assertIn("crates/runtime/commit-local/tests/p018_owner_local_conformance.rs", manifest)
        self.assertIn("tools/tests/test_p018_no_hosted_durable.py", manifest)


if __name__ == "__main__":
    unittest.main()
