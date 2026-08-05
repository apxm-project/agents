"""Absence proof: no hosted durable checkpoint/output service.

Owner-local in-memory/filesystem ExecutionCommitPort adapters remain mandatory;
a hosted service must not appear as an empty abstraction, feature flag, Compose
service, or speculative crate.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Patterns that would indicate a hosted / speculative P-018 service surface.
FORBIDDEN = [
    re.compile(r"hosted[_-]?durable[_-]?checkpoint", re.I),
    re.compile(r"CheckpointHost(?:ed)?Service", re.I),
    re.compile(r"durable_checkpoint_service", re.I),
    re.compile(r"apxm\.hosted-checkpoint", re.I),
    re.compile(r"feature\s*=\s*[\"']hosted[_-]?checkpoint", re.I),
]

# Allowed decision/evidence prose may mention the capability name when rejecting it.
ALLOW_PATH_PREFIXES = (
    "docs/plans/p018-hosted-durable-decision.md",
    "docs/evidence/p018-g3-no-build.md",
    "tools/tests/test_p018_no_hosted_durable.py",
    "tools/tests/test_p018_no_hosted_durable_verifier.py",
    "tools/scripts/verify_p018_no_hosted_durable.py",
    "crates/runtime/commit-local/",
)

P018_ARTIFACT_PREFIXES = (
    "docs/plans/p018-hosted-durable-decision.md",
    "docs/evidence/p018-g3-no-build.md",
    "crates/runtime/commit-local/",
)

DOWNSTREAM_PRODUCT_NAMES = [
    re.compile(r"\bCLIC\b", re.I),
    re.compile(r"\bStudio\b"),
    re.compile(r"\bAuth\b"),
    re.compile(r"\bWidget\b"),
    re.compile(r"\bHost SDK\b", re.I),
]

REMOTE_STORAGE_DEPENDENCIES = [
    re.compile(r"\b(?:axum|hyper|reqwest|tonic|sqlx|redis|postgres|mysql)\b", re.I),
    re.compile(r"\[features\]", re.I),
]

SCAN_SUFFIXES = {".rs", ".toml", ".json", ".yml", ".yaml", ".md", ".py"}
SKIP_DIR_NAMES = {
    ".git",
    "target",
    ".dekk",
    "node_modules",
    ".apxm",
    "__pycache__",
    "external",
}


def _allowed(path: Path) -> bool:
    rel = path.relative_to(ROOT).as_posix()
    return any(rel == p or rel.startswith(p) for p in ALLOW_PATH_PREFIXES)


class TestP018NoHostedDurable(unittest.TestCase):
    def test_no_hosted_durable_service_surface(self) -> None:
        hits: list[str] = []
        for path in ROOT.rglob("*"):
            if not path.is_file():
                continue
            if any(part in SKIP_DIR_NAMES for part in path.parts):
                continue
            if path.suffix not in SCAN_SUFFIXES:
                continue
            if _allowed(path):
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for pattern in FORBIDDEN:
                for match in pattern.finditer(text):
                    rel = path.relative_to(ROOT).as_posix()
                    line = text.count("\n", 0, match.start()) + 1
                    hits.append(f"{rel}:{line}: {match.group(0)}")
        self.assertEqual(
            hits,
            [],
            "hosted durable checkpoint/output surface must remain absent:\n"
            + "\n".join(hits),
        )

    def test_owner_local_adapter_crate_exists(self) -> None:
        cargo = ROOT / "crates/runtime/commit-local/Cargo.toml"
        self.assertTrue(cargo.is_file(), "apxm-commit-local crate missing")
        text = cargo.read_text(encoding="utf-8")
        self.assertIn('name = "apxm-commit-local"', text)
        self.assertIn("not a hosted durable service", text)

    def test_decision_dossier_records_no_build(self) -> None:
        dossier = ROOT / "docs/plans/p018-hosted-durable-decision.md"
        text = dossier.read_text(encoding="utf-8")
        self.assertRegex(text, r"Status:\s*\*\*no-build\*\*")
        self.assertIn("No unmet product-neutral job", text)
        self.assertNotIn("After this branch merges", text)
        self.assertNotIn("PR #41", text)

    def test_g3_checklist_points_at_post_merge_main_target(self) -> None:
        checklist = ROOT / "docs/plans/g3-admission-runtime-checklist.md"
        text = checklist.read_text(encoding="utf-8")
        self.assertIn("Post-merge target: **Published no-build decision on `main`**", text)
        self.assertNotIn("G3 gate blocker remains open", text)

    def test_p018_artifacts_have_no_downstream_product_names(self) -> None:
        hits: list[str] = []
        for path in ROOT.rglob("*"):
            if not path.is_file() or any(part in SKIP_DIR_NAMES for part in path.parts):
                continue
            rel = path.relative_to(ROOT).as_posix()
            if not any(rel == prefix or rel.startswith(prefix) for prefix in P018_ARTIFACT_PREFIXES):
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for pattern in DOWNSTREAM_PRODUCT_NAMES:
                if pattern.search(text):
                    hits.append(f"{rel}: {pattern.pattern}")
        self.assertEqual(hits, [], "P-018 artifacts must remain product-neutral:\n" + "\n".join(hits))

    def test_owner_local_crate_has_no_remote_storage_or_feature_surface(self) -> None:
        manifest = ROOT / "crates/runtime/commit-local/Cargo.toml"
        text = manifest.read_text(encoding="utf-8")
        hits = [pattern.pattern for pattern in REMOTE_STORAGE_DEPENDENCIES if pattern.search(text)]
        self.assertEqual(hits, [], "owner-local adapter must not grow a remote/feature surface")


if __name__ == "__main__":
    unittest.main()
