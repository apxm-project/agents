"""Observed prompt evaluation records portable request and response evidence."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


SCRIPT = Path(__file__).parents[1] / "scripts" / "observed_prompt_evaluation.py"
SPEC = importlib.util.spec_from_file_location("observed_prompt_evaluation", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)

TEST_REVISION = "0123456789abcdef0123456789abcdef01234567"
CAPABILITIES = {
    "protocol": "anthropic",
    "model": "observed-model",
    "context_window": 200000,
    "supports_functions": True,
    "supports_vision": False,
    "supports_thinking": True,
    "supports_structured_outputs": False,
    "source": "registered-backend",
}


class ObservedPromptEvaluationTests(unittest.TestCase):
    def write_bundle(self, root: Path) -> dict[str, Path]:
        optimization = root / "optimization.jsonl"
        held_out = root / "held-out.jsonl"
        baseline_arm = root / "baseline-arm.json"
        candidate_arm = root / "candidate-arm.json"
        optimization.write_text('{"id":"optimize-one","prompt":"draft"}\n', encoding="utf-8")
        held_out.write_text(
            '{"id":"held-out-one","input":"Classify the contract.","expected":"answer"}\n',
            encoding="utf-8",
        )
        baseline_arm.write_text(
            json.dumps(
                {
                    "arm_id": "baseline-arm",
                    "system_prompt": "baseline system",
                    "user_template": "{input}",
                    "temperature": 0.0,
                    "max_tokens": 16,
                }
            ),
            encoding="utf-8",
        )
        candidate_arm.write_text(
            json.dumps(
                {
                    "arm_id": "candidate-arm",
                    "system_prompt": "candidate system",
                    "user_template": "{input}",
                    "temperature": 0.0,
                    "max_tokens": 16,
                }
            ),
            encoding="utf-8",
        )
        preregistration = root / "preregistration.json"
        preregistration.write_text(
            json.dumps(
                {
                    "schema_version": MODULE.evidence.EVIDENCE_SCHEMA_VERSION,
                    "scenario": "observed-workflow-optimization",
                    "dataset": {"id": "observed-fixture", "revision": "v1"},
                    "split_policy": {"kind": "fixed-disjoint-case-ids"},
                    "splits": {
                        "optimization": {
                            "sha256": MODULE.evidence.sha256_file(optimization),
                            "case_count": 1,
                        },
                        "held_out": {
                            "sha256": MODULE.evidence.sha256_file(held_out),
                            "case_count": 1,
                        },
                    },
                    "arms": {
                        "baseline": {"id": "baseline-arm"},
                        "candidate": {"id": "candidate-arm"},
                    },
                    "backend_evidence": {
                        "kind": "observed-backend-run",
                        "backend": {"id": "observed", "revision": "observed-model"},
                        "measurement_source": "backend-telemetry",
                        "capabilities": CAPABILITIES,
                    },
                    "provenance": {
                        "repository": "apxm-project/agents",
                        "revision": TEST_REVISION,
                        "bundle_id": "observed-workflow-optimization-v1",
                    },
                    "metric": "exact_match",
                    "decision_rule": {"minimum_candidate_mean": 1.0, "minimum_delta": 1.0},
                }
            ),
            encoding="utf-8",
        )
        config = root / "config.toml"
        config.write_text(
            """
[[backends]]
name = "observed"
protocol = "anthropic"
type = "cloud"
endpoint = "env:OBSERVED_ENDPOINT"
api_key = "env:OBSERVED_API_KEY"

[[backends.models]]
id = "observed-model"
context_window = 200000
supports_functions = true
supports_vision = false
supports_thinking = true
supports_structured_outputs = false
""".strip()
            + "\n",
            encoding="utf-8",
        )
        return {
            "optimization": optimization,
            "held_out": held_out,
            "baseline_arm": baseline_arm,
            "candidate_arm": candidate_arm,
            "preregistration": preregistration,
            "config": config,
        }

    def test_executes_both_arms_and_validates_recorded_backend_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            paths = self.write_bundle(root)
            evaluation_dir = root / ".apxm" / "evaluation"
            layout = SimpleNamespace(repo_root=root, evaluation_dir=evaluation_dir)
            working_tree = {
                "head_revision": TEST_REVISION,
                "dirty": False,
                "dirty_entry_count": 0,
                "dirty_content_sha256": MODULE.evidence.sha256_bytes(b"[]"),
                "entries": [],
            }

            def fake_request(
                backend: MODULE.BackendRegistration,
                payload: dict[str, object],
                timeout_seconds: float,
            ) -> tuple[dict[str, object], float]:
                self.assertEqual(backend.name, "observed")
                self.assertGreater(timeout_seconds, 0)
                candidate = payload["system"] == "candidate system"
                return (
                    {
                        "id": "msg-candidate" if candidate else "msg-baseline",
                        "model": "observed-model",
                        "content": [
                            {"type": "text", "text": "answer" if candidate else "wrong"}
                        ],
                        "usage": {"input_tokens": 5, "output_tokens": 1},
                    },
                    12.5 if candidate else 15.0,
                )

            with patch.dict(
                os.environ,
                {"OBSERVED_ENDPOINT": "https://backend.example", "OBSERVED_API_KEY": "secret"},
                clear=False,
            ), patch.object(MODULE, "build_layout", return_value=layout), patch.object(
                MODULE.evidence, "build_layout", return_value=layout
            ), patch.object(
                MODULE,
                "committed_preregistration_evidence",
                return_value={
                    "repository": "apxm-project/agents",
                    "source_path": "evaluation/preregistration.json",
                    "source_commit": TEST_REVISION,
                    "declared_revision": TEST_REVISION,
                    "evaluated_head": TEST_REVISION,
                    "preregistration_sha256": MODULE.evidence.sha256_file(
                        paths["preregistration"]
                    ),
                },
            ), patch.object(
                MODULE.evidence,
                "collect_working_tree_provenance",
                return_value=working_tree,
            ), patch.object(MODULE, "perform_request", side_effect=fake_request):
                result = MODULE.main(
                    [
                        "--preregistration",
                        str(paths["preregistration"]),
                        "--optimization",
                        str(paths["optimization"]),
                        "--held-out",
                        str(paths["held_out"]),
                        "--baseline-arm",
                        str(paths["baseline_arm"]),
                        "--candidate-arm",
                        str(paths["candidate_arm"]),
                        "--backend",
                        "observed",
                        "--config",
                        str(paths["config"]),
                        "--run-id",
                        "observed-test",
                    ]
                )

            self.assertEqual(result, 0)
            bundle = evaluation_dir / "observed-workflow-optimization" / "bundles" / "observed-test"
            run = evaluation_dir / "observed-workflow-optimization" / "runs" / "observed-test"
            manifest = json.loads((run / "manifest.json").read_text(encoding="utf-8"))
            summary = json.loads((run / "summary.json").read_text(encoding="utf-8"))
            candidate = json.loads((bundle / "candidate.jsonl").read_text(encoding="utf-8"))
            receipt_path = bundle / candidate["evidence"]["path"]
            receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
            execution_path = bundle / receipt["execution_evidence"]["path"]
            execution = json.loads(execution_path.read_text(encoding="utf-8"))

            self.assertTrue(summary["decision"]["accepted"])
            self.assertTrue(summary["measurements"]["is_backend_telemetry"])
            self.assertEqual(manifest["backend_evidence"]["capabilities"], CAPABILITIES)
            self.assertEqual(
                manifest["observed_bundle"]["preregistration_source"]["path"],
                "preregistration-source.json",
            )
            self.assertEqual(execution["provider_response_id"], "msg-candidate")
            self.assertEqual(execution["request"]["sha256"], receipt["request_sha256"])
            self.assertNotIn("secret", json.dumps(manifest))

    def test_rejects_literal_secret_and_capability_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            paths = self.write_bundle(root)
            preregistration = MODULE.evidence.validate_preregistration(
                MODULE.evidence.load_json(paths["preregistration"])
            )["backend_evidence"]
            config = paths["config"]
            config.write_text(
                config.read_text(encoding="utf-8").replace(
                    'api_key = "env:OBSERVED_API_KEY"', 'api_key = "literal-secret"'
                ),
                encoding="utf-8",
            )
            with patch.dict(os.environ, {"OBSERVED_ENDPOINT": "https://backend.example"}), self.assertRaisesRegex(
                ValueError, "must be an env: reference"
            ):
                MODULE.load_backend(config, "observed", preregistration)

            config.write_text(
                config.read_text(encoding="utf-8")
                .replace('api_key = "literal-secret"', 'api_key = "env:OBSERVED_API_KEY"')
                .replace("context_window = 200000", "context_window = 100000"),
                encoding="utf-8",
            )
            with patch.dict(
                os.environ,
                {"OBSERVED_ENDPOINT": "https://backend.example", "OBSERVED_API_KEY": "secret"},
            ), self.assertRaisesRegex(ValueError, "capabilities do not match"):
                MODULE.load_backend(config, "observed", preregistration)


if __name__ == "__main__":
    unittest.main()
