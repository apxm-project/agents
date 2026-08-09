"""Verifier coverage for the checked-in P-018 no-build evidence."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
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

    def test_verifier_covers_p018_evidence_artifacts(self) -> None:
        manifest = self.module.parse_digest_manifest()
        self.assertIn("docs/plans/p018-hosted-durable-decision.md", manifest)
        self.assertIn("docs/plans/g3-admission-runtime-checklist.md", manifest)
        self.assertIn("crates/runtime/commit-local/src/filesystem.rs", manifest)
        self.assertIn("crates/runtime/commit-local/src/memory.rs", manifest)
        self.assertIn("crates/runtime/commit-local/tests/p018_owner_local_conformance.rs", manifest)
        self.assertIn("tools/tests/test_p018_no_hosted_durable.py", manifest)

    def test_owner_scope_rejects_forbidden_service_and_dependency_fixtures(self) -> None:
        fixtures = (
            ("hosted-or-service", "pub struct HostedService;", ""),
            ("network-or-remote", "pub fn open() { tokio::net::TcpListener::bind(\"127.0.0.1:1\"); }", ""),
            ("multi-tenant", "pub const TENANT_ID: &str = \"tenant\";", ""),
            ("downstream-or-product", "", 'downstream-binding = { workspace = true }'),
            ("alias", "use crate::store as legacy_store;", ""),
            ("fallback-or-speculative", "pub fn choose() { let fallback = true; }", ""),
            ("feature-flag", "#[cfg(feature = \"hosted\")] pub fn hosted() {}", ""),
            ("placeholder", "pub fn start() { todo!(\"placeholder\"); }", ""),
            ("second-writer", "pub const SECOND_WRITER: bool = true;", ""),
        )
        for category, rust_body, dependency in fixtures:
            with self.subTest(category=category), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                owner = root / "crates/runtime/commit-local"
                (owner / "src").mkdir(parents=True)
                (owner / "src/lib.rs").write_text(rust_body, encoding="utf-8")
                (owner / "Cargo.toml").write_text(
                    "[package]\nname = \"fixture\"\nversion = \"0.0.0\"\n"
                    "\n[dependencies]\n"
                    + dependency
                    + ("\n" if dependency else ""),
                    encoding="utf-8",
                )
                findings = self.module.owner_scope_findings(root)
                self.assertTrue(
                    any(finding["category"] == category for finding in findings),
                    f"fixture for {category} was accepted: {findings}",
                )

    def test_owner_scope_rejects_unscanned_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            owner = root / "crates/runtime/commit-local"
            (owner / "src").mkdir(parents=True)
            (owner / "src/lib.rs").write_text("pub struct Local;", encoding="utf-8")
            (owner / "README.txt").write_text("unscanned owner content", encoding="utf-8")
            (owner / "Cargo.toml").write_text(
                "[package]\nname = \"fixture\"\nversion = \"0.0.0\"\n", encoding="utf-8"
            )
            findings = self.module.owner_scope_findings(root)
            self.assertTrue(any(finding["category"] == "owner-scope" for finding in findings))

    def test_authoritative_owner_scope_check_passes_for_checked_in_scope(self) -> None:
        checks = self.module.owner_absence_checks()
        self.assertTrue(checks)
        self.assertTrue(all(check.status == "passed" for check in checks), checks)
        self.assertIn("owner-scope:single-writer-lock", {check.name for check in checks})

    def test_owner_scope_requires_the_exclusive_writer_lock(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            owner = root / "crates/runtime/commit-local"
            (owner / "src").mkdir(parents=True)
            (owner / "src/filesystem.rs").write_text("pub struct Filesystem;", encoding="utf-8")
            (owner / "Cargo.toml").write_text(
                "[package]\nname = \"fixture\"\nversion = \"0.0.0\"\n", encoding="utf-8"
            )
            checks = self.module.owner_absence_checks(root)
            lock_check = next(check for check in checks if check.name == "owner-scope:single-writer-lock")
            self.assertEqual(lock_check.status, "failed")


if __name__ == "__main__":
    unittest.main()
