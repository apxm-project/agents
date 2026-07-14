"""Offline prompt evaluation writes preregistered, portable held-out evidence."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


SCRIPT = Path(__file__).parents[1] / "scripts" / "offline_prompt_evaluation.py"
REPOSITORY_ROOT = SCRIPT.parents[2]
CANONICAL_BUNDLE = REPOSITORY_ROOT / "evaluation" / "workflow-optimization"
SPEC = importlib.util.spec_from_file_location("offline_prompt_evaluation", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)

TEST_REVISION = "0123456789abcdef0123456789abcdef01234567"
TEST_DIRTY_CONTENT_SHA256 = "a" * 64
TEST_FILE_SHA256 = "b" * 64
TEST_CAPABILITIES = {
    "protocol": "anthropic",
    "model": "observed-model",
    "context_window": 200000,
    "supports_functions": True,
    "supports_vision": False,
    "supports_thinking": True,
    "supports_structured_outputs": False,
    "source": "registered-backend",
}
TEST_WORKING_TREE = {
    "head_revision": TEST_REVISION,
    "dirty": True,
    "dirty_entry_count": 1,
    "dirty_content_sha256": TEST_DIRTY_CONTENT_SHA256,
    "entries": [
        {
            "path": "tools/scripts/offline_prompt_evaluation.py",
            "status": ".M",
            "kind": "file",
            "sha256": TEST_FILE_SHA256,
        }
    ],
}


class OfflinePromptEvaluationTests(unittest.TestCase):
    def write_preregistration(
        self,
        root: Path,
        optimization: Path,
        held_out: Path,
        optimization_case_count: int = 1,
        held_out_case_count: int = 1,
        **overrides: object,
    ) -> Path:
        preregistration = {
            "schema_version": MODULE.EVIDENCE_SCHEMA_VERSION,
            "scenario": "held-out-prompt",
            "dataset": {"id": "fixture-dataset", "revision": "fixture-v1"},
            "split_policy": {"kind": "fixed-case-ids"},
            "splits": {
                "optimization": {
                    "sha256": MODULE.sha256_file(optimization),
                    "case_count": optimization_case_count,
                },
                "held_out": {
                    "sha256": MODULE.sha256_file(held_out),
                    "case_count": held_out_case_count,
                },
            },
            "arms": {
                "baseline": {"id": "baseline-fixture"},
                "candidate": {"id": "candidate-fixture"},
            },
            "backend_evidence": {
                "kind": "deterministic-fixture",
                "backend": {"id": "fixture-backend", "revision": "fixture-v1"},
                "measurement_source": "fixture-values",
            },
            "provenance": {
                "repository": "apxm-project/agents",
                "revision": TEST_REVISION,
                "bundle_id": "held-out-prompt-fixture-v1",
            },
            "metric": "exact_match",
            "decision_rule": {"minimum_candidate_mean": 1.0, "minimum_delta": 0.5},
        }
        preregistration.update(overrides)
        path = root / "preregistration.json"
        path.write_text(json.dumps(preregistration), encoding="utf-8")
        return path

    def run_evaluation(
        self,
        bundle_root: Path,
        preregistration: Path,
        *,
        artifact_root: Path | None = None,
        run_id: str = "test-run",
        working_tree: dict[str, object] | None = None,
    ) -> int:
        artifact_root = artifact_root or bundle_root
        evaluation_dir = artifact_root / ".apxm" / "evaluation"
        with patch.object(
            MODULE,
            "build_layout",
            return_value=SimpleNamespace(repo_root=artifact_root, evaluation_dir=evaluation_dir),
        ), patch.object(
            MODULE,
            "collect_working_tree_provenance",
            return_value=working_tree or TEST_WORKING_TREE,
        ):
            return MODULE.main(
                [
                    "--preregistration",
                    str(preregistration),
                    "--bundle-root",
                    str(bundle_root),
                    "--optimization",
                    str(bundle_root / "optimization.jsonl"),
                    "--held-out",
                    str(bundle_root / "held-out.jsonl"),
                    "--baseline",
                    str(bundle_root / "baseline.jsonl"),
                    "--candidate",
                    str(bundle_root / "candidate.jsonl"),
                    "--run-id",
                    run_id,
                ]
            )

    def write_observed_receipt(
        self,
        root: Path,
        *,
        backend: dict[str, str],
        case_id: str,
        arm_id: str,
        output: str,
        token_count: int,
        latency_ms: float,
    ) -> dict[str, str]:
        request_path = root / "observed-requests" / f"{arm_id}-{case_id}.json"
        request_path.parent.mkdir(exist_ok=True)
        request_path.write_text(
            json.dumps({"model": TEST_CAPABILITIES["model"], "prompt": case_id}, sort_keys=True)
            + "\n",
            encoding="utf-8",
        )
        response_path = root / "observed-responses" / f"{arm_id}-{case_id}.json"
        response_path.parent.mkdir(exist_ok=True)
        response_path.write_text(
            json.dumps(
                {
                    "id": f"response-{arm_id}-{case_id}",
                    "model": TEST_CAPABILITIES["model"],
                    "output": output,
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        request_reference = {
            "path": request_path.relative_to(root).as_posix(),
            "sha256": MODULE.sha256_file(request_path),
        }
        response_reference = {
            "path": response_path.relative_to(root).as_posix(),
            "sha256": MODULE.sha256_file(response_path),
        }
        execution = {
            "schema_version": MODULE.OBSERVED_BACKEND_EXECUTION_SCHEMA_VERSION,
            "kind": "recorded-backend-execution",
            "backend": backend,
            "capabilities_sha256": MODULE.sha256_json(TEST_CAPABILITIES),
            "case_id": case_id,
            "arm_id": arm_id,
            "request": request_reference,
            "response": response_reference,
            "provider_response_id": f"response-{arm_id}-{case_id}",
            "provider_model": TEST_CAPABILITIES["model"],
            "output_sha256": MODULE.sha256_text(output),
            "token_count": token_count,
            "latency_ms": latency_ms,
            "observed_at_utc": "2026-07-13T12:00:00Z",
        }
        execution_path = root / "observed-executions" / f"{arm_id}-{case_id}.json"
        execution_path.parent.mkdir(exist_ok=True)
        execution_path.write_text(json.dumps(execution, sort_keys=True) + "\n", encoding="utf-8")
        execution_reference = {
            "path": execution_path.relative_to(root).as_posix(),
            "sha256": MODULE.sha256_file(execution_path),
        }
        receipt = {
            "schema_version": MODULE.OBSERVED_BACKEND_RECEIPT_SCHEMA_VERSION,
            "kind": "observed-backend-receipt",
            "backend": backend,
            "case_id": case_id,
            "arm_id": arm_id,
            "request_sha256": request_reference["sha256"],
            "output_sha256": MODULE.sha256_text(output),
            "token_count": token_count,
            "latency_ms": latency_ms,
            "execution_evidence": execution_reference,
        }
        path = root / "observed-receipts" / f"{arm_id}-{case_id}.json"
        path.parent.mkdir(exist_ok=True)
        path.write_text(json.dumps(receipt, sort_keys=True) + "\n", encoding="utf-8")
        return {
            "path": path.relative_to(root).as_posix(),
            "sha256": MODULE.sha256_file(path),
        }

    def test_writes_portable_preregistered_evidence_and_accepts_candidate(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            optimization = root / "optimization.jsonl"
            held_out = root / "held-out.jsonl"
            baseline = root / "baseline.jsonl"
            candidate = root / "candidate.jsonl"
            optimization.write_text(
                '{"id":"optimization-one","prompt":"draft"}\n'
                '{"id":"optimization-two","prompt":"refine"}\n',
                encoding="utf-8",
            )
            held_out.write_text(
                '{"id":"held-out-one","expected":"answer"}\n'
                '{"id":"held-out-two","expected":"second answer"}\n',
                encoding="utf-8",
            )
            baseline.write_text(
                '{"id":"held-out-one","output":"wrong","arm_id":"baseline-fixture",'
                '"token_count":3,"latency_ms":10.0}\n'
                '{"id":"held-out-two","output":"wrong","arm_id":"baseline-fixture",'
                '"token_count":5,"latency_ms":30.0}\n',
                encoding="utf-8",
            )
            candidate.write_text(
                '{"id":"held-out-one","output":"answer","arm_id":"candidate-fixture",'
                '"token_count":2,"latency_ms":8.0}\n'
                '{"id":"held-out-two","output":"second answer","arm_id":"candidate-fixture",'
                '"token_count":4,"latency_ms":12.0}\n',
                encoding="utf-8",
            )
            preregistration = self.write_preregistration(
                root,
                optimization,
                held_out,
                optimization_case_count=2,
                held_out_case_count=2,
            )

            self.assertEqual(self.run_evaluation(root, preregistration), 0)

            run_dir = root / ".apxm" / "evaluation" / "held-out-prompt" / "runs" / "test-run"
            manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
            summary = json.loads((run_dir / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["inputs"]["held_out"]["path"], "held-out.jsonl")
            self.assertNotIn(str(root), json.dumps(manifest))
            self.assertEqual(manifest["splits"]["optimization"]["case_count"], 2)
            self.assertEqual(manifest["arms"]["candidate"]["id"], "candidate-fixture")
            self.assertEqual(
                manifest["backend_evidence"],
                {
                    "kind": "deterministic-fixture",
                    "backend": {"id": "fixture-backend", "revision": "fixture-v1"},
                    "measurement_source": "fixture-values",
                },
            )
            self.assertEqual(
                manifest["provenance"]["declared"],
                {
                    "repository": "apxm-project/agents",
                    "revision": TEST_REVISION,
                    "bundle_id": "held-out-prompt-fixture-v1",
                },
            )
            self.assertEqual(
                manifest["provenance"]["preregistration"],
                {"path": "preregistration.json", "sha256": MODULE.sha256_file(preregistration)},
            )
            self.assertEqual(manifest["provenance"]["runner_sha256"], MODULE.sha256_file(SCRIPT))
            self.assertEqual(manifest["provenance"]["working_tree"], TEST_WORKING_TREE)
            self.assertEqual(summary["candidate_mean"], 1.0)
            self.assertTrue(summary["decision"]["accepted"])
            self.assertEqual(
                summary["measurements"],
                {
                    "source": "fixture-values",
                    "is_backend_telemetry": False,
                    "baseline": {
                        "total_token_count": 8,
                        "mean_token_count": 4.0,
                        "total_latency_ms": 40.0,
                        "mean_latency_ms": 20.0,
                    },
                    "candidate": {
                        "total_token_count": 6,
                        "mean_token_count": 3.0,
                        "total_latency_ms": 20.0,
                        "mean_latency_ms": 10.0,
                    },
                },
            )
            self.assertEqual(summary["scores"][1]["candidate_latency_ms"], 12.0)

    def test_canonical_repository_fixture_is_a_deterministic_contract_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            artifact_root = Path(temporary)
            preregistration = CANONICAL_BUNDLE / "preregistration.json"
            declared_revision = json.loads(preregistration.read_text(encoding="utf-8"))["provenance"][
                "revision"
            ]
            canonical_working_tree = {**TEST_WORKING_TREE, "head_revision": declared_revision}

            self.assertEqual(
                self.run_evaluation(
                    CANONICAL_BUNDLE,
                    preregistration,
                    artifact_root=artifact_root,
                    run_id="canonical-fixture",
                    working_tree=canonical_working_tree,
                ),
                0,
            )

            run_dir = (
                artifact_root
                / ".apxm"
                / "evaluation"
                / "workflow-optimization-contracts"
                / "runs"
                / "canonical-fixture"
            )
            manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
            summary = json.loads((run_dir / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["schema_version"], MODULE.EVIDENCE_SCHEMA_VERSION)
            self.assertEqual(manifest["backend_evidence"]["kind"], "deterministic-fixture")
            self.assertEqual(manifest["backend_evidence"]["measurement_source"], "fixture-values")
            self.assertNotIn("runtime_gate", manifest["backend_evidence"])
            self.assertNotIn("observed_backend_receipts", manifest)
            self.assertFalse(summary["measurements"]["is_backend_telemetry"])
            self.assertNotIn(str(CANONICAL_BUNDLE), json.dumps(manifest))

    def test_rejects_overlapping_optimization_and_held_out_cases(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            optimization = root / "optimization.jsonl"
            held_out = root / "held-out.jsonl"
            baseline = root / "baseline.jsonl"
            candidate = root / "candidate.jsonl"
            optimization.write_text('{"id":"shared","prompt":"draft"}\n', encoding="utf-8")
            held_out.write_text('{"id":"shared","expected":"answer"}\n', encoding="utf-8")
            baseline.write_text(
                '{"id":"shared","output":"wrong","arm_id":"baseline-fixture",'
                '"token_count":3,"latency_ms":10.0}\n',
                encoding="utf-8",
            )
            candidate.write_text(
                '{"id":"shared","output":"answer","arm_id":"candidate-fixture",'
                '"token_count":2,"latency_ms":8.0}\n',
                encoding="utf-8",
            )
            preregistration = self.write_preregistration(root, optimization, held_out)

            with self.assertRaisesRegex(ValueError, "must be disjoint"):
                self.run_evaluation(root, preregistration)

    def test_rejects_unregistered_arm_identity_and_failing_decision_rule(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            optimization = root / "optimization.jsonl"
            held_out = root / "held-out.jsonl"
            baseline = root / "baseline.jsonl"
            candidate = root / "candidate.jsonl"
            optimization.write_text('{"id":"optimization-one","prompt":"draft"}\n', encoding="utf-8")
            held_out.write_text('{"id":"held-out-one","expected":"answer"}\n', encoding="utf-8")
            baseline.write_text(
                '{"id":"held-out-one","output":"wrong","arm_id":"baseline-fixture",'
                '"token_count":3,"latency_ms":10.0}\n',
                encoding="utf-8",
            )
            candidate.write_text(
                '{"id":"held-out-one","output":"wrong","arm_id":"wrong-candidate",'
                '"token_count":2,"latency_ms":8.0}\n',
                encoding="utf-8",
            )
            preregistration = self.write_preregistration(root, optimization, held_out)

            with self.assertRaisesRegex(ValueError, "preregistered arm_id"):
                self.run_evaluation(root, preregistration)

            candidate.write_text(
                '{"id":"held-out-one","output":"wrong","arm_id":"candidate-fixture",'
                '"token_count":2,"latency_ms":8.0}\n',
                encoding="utf-8",
            )
            self.assertEqual(self.run_evaluation(root, preregistration), 0)
            summary_path = root / ".apxm" / "evaluation" / "held-out-prompt" / "runs" / "test-run" / "summary.json"
            summary = json.loads(summary_path.read_text(encoding="utf-8"))
            self.assertFalse(summary["decision"]["accepted"])

    def test_rejects_unstructured_or_mislabeled_backend_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            optimization = root / "optimization.jsonl"
            held_out = root / "held-out.jsonl"
            baseline = root / "baseline.jsonl"
            candidate = root / "candidate.jsonl"
            optimization.write_text('{"id":"optimization-one","prompt":"draft"}\n', encoding="utf-8")
            held_out.write_text('{"id":"held-out-one","expected":"answer"}\n', encoding="utf-8")
            baseline.write_text(
                '{"id":"held-out-one","output":"wrong","arm_id":"baseline-fixture",'
                '"token_count":3,"latency_ms":10.0}\n',
                encoding="utf-8",
            )
            candidate.write_text(
                '{"id":"held-out-one","output":"answer","arm_id":"candidate-fixture",'
                '"token_count":2,"latency_ms":8.0}\n',
                encoding="utf-8",
            )

            for backend_evidence, error in (
                ({"kind": "deterministic-fixture"}, "backend_evidence missing"),
                (
                    {
                        "kind": "deterministic-fixture",
                        "backend": {"id": "fixture-backend", "revision": "fixture-v1"},
                        "measurement_source": "backend-telemetry",
                    },
                    "must declare measurement_source 'fixture-values'",
                ),
                (
                    {
                        "kind": "observed-backend-run",
                        "backend": {"id": "backend", "revision": "revision-v1"},
                        "measurement_source": "fixture-values",
                        "capabilities": TEST_CAPABILITIES,
                    },
                    "must declare measurement_source 'backend-telemetry'",
                ),
                (
                    {
                        "kind": "deterministic-fixture",
                        "backend": {"id": "fixture-backend", "revision": "fixture-v1"},
                        "measurement_source": "fixture-values",
                        "runtime_gate": "forged completion claim",
                    },
                    r"has unexpected keys \['runtime_gate'\]",
                ),
            ):
                with self.subTest(backend_evidence=backend_evidence):
                    preregistration = self.write_preregistration(
                        root,
                        optimization,
                        held_out,
                        backend_evidence=backend_evidence,
                    )

                    with self.assertRaisesRegex(ValueError, error):
                        self.run_evaluation(root, preregistration)

    def test_rejects_nonportable_or_incomplete_preregistration_provenance(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            optimization = root / "optimization.jsonl"
            held_out = root / "held-out.jsonl"
            baseline = root / "baseline.jsonl"
            candidate = root / "candidate.jsonl"
            optimization.write_text('{"id":"optimization-one","prompt":"draft"}\n', encoding="utf-8")
            held_out.write_text('{"id":"held-out-one","expected":"answer"}\n', encoding="utf-8")
            baseline.write_text(
                '{"id":"held-out-one","output":"wrong","arm_id":"baseline-fixture",'
                '"token_count":3,"latency_ms":10.0}\n',
                encoding="utf-8",
            )
            candidate.write_text(
                '{"id":"held-out-one","output":"answer","arm_id":"candidate-fixture",'
                '"token_count":2,"latency_ms":8.0}\n',
                encoding="utf-8",
            )

            for provenance, error in (
                ({}, "provenance missing"),
                (
                    {
                        "repository": "apxm-project/agents",
                        "revision": "",
                        "bundle_id": "held-out-prompt-fixture-v1",
                    },
                    "provenance.revision must be a non-empty string",
                ),
                (
                    {
                        "repository": "/home/evaluator/agents",
                        "revision": TEST_REVISION,
                        "bundle_id": "held-out-prompt-fixture-v1",
                    },
                    "provenance.repository must be a portable owner/repository identifier",
                ),
                (
                    {
                        "repository": "apxm-project/agents",
                        "revision": "fixture-revision",
                        "bundle_id": "held-out-prompt-fixture-v1",
                    },
                    "provenance.revision must be a lowercase revision digest",
                ),
            ):
                with self.subTest(provenance=provenance):
                    preregistration = self.write_preregistration(
                        root,
                        optimization,
                        held_out,
                        provenance=provenance,
                    )

                    with self.assertRaisesRegex(ValueError, error):
                        self.run_evaluation(root, preregistration)

    def test_records_portable_dirty_working_tree_content_fingerprints(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tracked = root / "tracked.txt"
            tracked.write_text("committed\n", encoding="utf-8")
            subprocess.run(
                ["git", "init", "--quiet"],
                cwd=root,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            subprocess.run(
                ["git", "add", "tracked.txt"],
                cwd=root,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            subprocess.run(
                [
                    "git",
                    "-c",
                    "user.name=APXM Evaluator",
                    "-c",
                    "user.email=evaluator@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "fixture",
                ],
                cwd=root,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            tracked.write_text("dirty bytes\n", encoding="utf-8")
            untracked = root / "untracked.txt"
            untracked.write_text("untracked bytes\n", encoding="utf-8")

            provenance = MODULE.collect_working_tree_provenance(root)

            entries = {entry["path"]: entry for entry in provenance["entries"]}
            self.assertTrue(provenance["dirty"])
            self.assertEqual(provenance["dirty_entry_count"], 2)
            self.assertEqual(entries["tracked.txt"]["sha256"], MODULE.sha256_file(tracked))
            self.assertEqual(entries["untracked.txt"]["sha256"], MODULE.sha256_file(untracked))
            canonical_entries = json.dumps(
                provenance["entries"],
                ensure_ascii=True,
                separators=(",", ":"),
                sort_keys=True,
            ).encode("utf-8")
            self.assertEqual(
                provenance["dirty_content_sha256"], MODULE.sha256_bytes(canonical_entries)
            )
            self.assertNotIn(str(root), json.dumps(provenance))

    def test_rejects_machine_local_working_tree_paths(self) -> None:
        for value in ("/home/evaluator/file", "../outside", r"C:\\workspace\\file"):
            with self.subTest(value=value), self.assertRaisesRegex(
                ValueError, "repository-relative path"
            ):
                MODULE.validate_repo_relative_path(value, "working-tree path")

    def test_rejects_declared_revision_that_differs_from_working_tree_head(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            optimization = root / "optimization.jsonl"
            held_out = root / "held-out.jsonl"
            baseline = root / "baseline.jsonl"
            candidate = root / "candidate.jsonl"
            optimization.write_text('{"id":"optimization-one","prompt":"draft"}\n', encoding="utf-8")
            held_out.write_text('{"id":"held-out-one","expected":"answer"}\n', encoding="utf-8")
            baseline.write_text(
                '{"id":"held-out-one","output":"wrong","arm_id":"baseline-fixture",'
                '"token_count":3,"latency_ms":10.0}\n',
                encoding="utf-8",
            )
            candidate.write_text(
                '{"id":"held-out-one","output":"answer","arm_id":"candidate-fixture",'
                '"token_count":2,"latency_ms":8.0}\n',
                encoding="utf-8",
            )
            preregistration = self.write_preregistration(root, optimization, held_out)
            mismatched_working_tree = {**TEST_WORKING_TREE, "head_revision": "f" * 40}
            evaluation_dir = root / ".apxm" / "evaluation"

            with patch.object(
                MODULE,
                "build_layout",
                return_value=SimpleNamespace(repo_root=root, evaluation_dir=evaluation_dir),
            ), patch.object(
                MODULE,
                "collect_working_tree_provenance",
                return_value=mismatched_working_tree,
            ), self.assertRaisesRegex(ValueError, "does not match"):
                MODULE.main(
                    [
                        "--preregistration",
                        str(preregistration),
                        "--bundle-root",
                        str(root),
                        "--optimization",
                        str(optimization),
                        "--held-out",
                        str(held_out),
                        "--baseline",
                        str(baseline),
                        "--candidate",
                        str(candidate),
                        "--run-id",
                        "test-run",
                    ]
                )

    def test_rejects_missing_or_invalid_observed_metrics(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            optimization = root / "optimization.jsonl"
            held_out = root / "held-out.jsonl"
            baseline = root / "baseline.jsonl"
            candidate = root / "candidate.jsonl"
            optimization.write_text('{"id":"optimization-one","prompt":"draft"}\n', encoding="utf-8")
            held_out.write_text('{"id":"held-out-one","expected":"answer"}\n', encoding="utf-8")
            baseline.write_text(
                '{"id":"held-out-one","output":"wrong","arm_id":"baseline-fixture",'
                '"token_count":3,"latency_ms":10.0}\n',
                encoding="utf-8",
            )
            preregistration = self.write_preregistration(root, optimization, held_out)

            for candidate_row, error in (
                (
                    '{"id":"held-out-one","output":"answer","arm_id":"candidate-fixture",'
                    '"latency_ms":8.0}\n',
                    "missing \\['token_count'\\]",
                ),
                (
                    '{"id":"held-out-one","output":"answer","arm_id":"candidate-fixture",'
                    '"token_count":-1,"latency_ms":8.0}\n',
                    "candidate token_count",
                ),
                (
                    '{"id":"held-out-one","output":"answer","arm_id":"candidate-fixture",'
                    '"token_count":2,"latency_ms":NaN}\n',
                    "candidate latency_ms",
                ),
            ):
                with self.subTest(candidate_row=candidate_row):
                    candidate.write_text(candidate_row, encoding="utf-8")

                    with self.assertRaisesRegex(ValueError, error):
                        self.run_evaluation(root, preregistration)

    def test_rejects_arbitrary_emitted_output_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            optimization = root / "optimization.jsonl"
            held_out = root / "held-out.jsonl"
            baseline = root / "baseline.jsonl"
            candidate = root / "candidate.jsonl"
            optimization.write_text('{"id":"optimization-one","prompt":"draft"}\n', encoding="utf-8")
            held_out.write_text('{"id":"held-out-one","expected":"answer"}\n', encoding="utf-8")
            baseline.write_text(
                '{"id":"held-out-one","output":"wrong","arm_id":"baseline-fixture",'
                '"token_count":3,"latency_ms":10.0}\n',
                encoding="utf-8",
            )
            candidate.write_text(
                '{"id":"held-out-one","output":"answer","arm_id":"candidate-fixture",'
                '"token_count":2,"latency_ms":8.0,"claimed_backend":"forged"}\n',
                encoding="utf-8",
            )
            preregistration = self.write_preregistration(root, optimization, held_out)

            with self.assertRaisesRegex(ValueError, "has unexpected keys \\['claimed_backend'\\]"):
                self.run_evaluation(root, preregistration)

    def test_observed_backend_rows_require_portable_bound_receipts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            optimization = root / "optimization.jsonl"
            held_out = root / "held-out.jsonl"
            baseline = root / "baseline.jsonl"
            candidate = root / "candidate.jsonl"
            optimization.write_text('{"id":"optimization-one","prompt":"draft"}\n', encoding="utf-8")
            held_out.write_text('{"id":"held-out-one","expected":"answer"}\n', encoding="utf-8")
            backend = {"id": "observed-backend", "revision": "deployment-20260713"}
            baseline_evidence = self.write_observed_receipt(
                root,
                backend=backend,
                case_id="held-out-one",
                arm_id="baseline-fixture",
                output="wrong",
                token_count=3,
                latency_ms=10.0,
            )
            candidate_evidence = self.write_observed_receipt(
                root,
                backend=backend,
                case_id="held-out-one",
                arm_id="candidate-fixture",
                output="answer",
                token_count=2,
                latency_ms=8.0,
            )
            baseline.write_text(
                json.dumps(
                    {
                        "id": "held-out-one",
                        "output": "wrong",
                        "arm_id": "baseline-fixture",
                        "token_count": 3,
                        "latency_ms": 10.0,
                        "evidence": baseline_evidence,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            candidate.write_text(
                json.dumps(
                    {
                        "id": "held-out-one",
                        "output": "answer",
                        "arm_id": "candidate-fixture",
                        "token_count": 2,
                        "latency_ms": 8.0,
                        "evidence": candidate_evidence,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            preregistration = self.write_preregistration(
                root,
                optimization,
                held_out,
                backend_evidence={
                    "kind": "observed-backend-run",
                    "backend": backend,
                    "measurement_source": "backend-telemetry",
                    "capabilities": TEST_CAPABILITIES,
                },
            )

            self.assertEqual(self.run_evaluation(root, preregistration, run_id="observed-valid"), 0)
            run_dir = root / ".apxm" / "evaluation" / "held-out-prompt" / "runs" / "observed-valid"
            manifest = json.loads((run_dir / "manifest.json").read_text(encoding="utf-8"))
            summary = json.loads((run_dir / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(
                manifest["observed_backend_receipts"]["candidate"],
                [{"case_id": "held-out-one", "evidence": candidate_evidence}],
            )
            self.assertTrue(summary["measurements"]["is_backend_telemetry"])

            candidate.write_text(
                json.dumps(
                    {
                        "id": "held-out-one",
                        "output": "forged answer",
                        "arm_id": "candidate-fixture",
                        "token_count": 2,
                        "latency_ms": 8.0,
                        "evidence": candidate_evidence,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "does not match output"):
                self.run_evaluation(root, preregistration, run_id="observed-forged-output")

            candidate.write_text(
                json.dumps(
                    {
                        "id": "held-out-one",
                        "output": "answer",
                        "arm_id": "candidate-fixture",
                        "token_count": 2,
                        "latency_ms": 8.0,
                        "evidence": {**candidate_evidence, "sha256": "0" * 64},
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "digest does not match referenced file"):
                self.run_evaluation(root, preregistration, run_id="observed-forged-reference")

            candidate.write_text(
                json.dumps(
                    {
                        "id": "held-out-one",
                        "output": "answer",
                        "arm_id": "candidate-fixture",
                        "token_count": 2,
                        "latency_ms": 8.0,
                        "evidence": {**candidate_evidence, "path": "../outside.json"},
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "must be a portable bundle-relative path"):
                self.run_evaluation(root, preregistration, run_id="observed-nonportable-reference")
