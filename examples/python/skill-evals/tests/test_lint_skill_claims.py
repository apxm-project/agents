#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "lint_skill_claims.py"
SPEC = importlib.util.spec_from_file_location("lint_skill_claims", SCRIPT)
assert SPEC and SPEC.loader
lint_skill_claims = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lint_skill_claims)


class ClaimLintTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / ".apxm" / "evaluation" / "run").mkdir(parents=True)
        self.run = self.root / ".apxm" / "evaluation" / "run"
        self._write_good_evidence()

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def _write_good_evidence(self) -> None:
        (self.run / "results.csv").write_text(
            "cell_label,failed_tenants,batch_wall_ms\nD,0,100.0\n",
            encoding="utf-8",
        )
        (self.run / "results.tenants.csv").write_text(
            "cell_label,returncode,wall_ms\nD,0,100.0\n",
            encoding="utf-8",
        )
        (self.run / "capabilities.json").write_text(
            json.dumps({
                "backends": {
                    "graph_capabilities": {
                        "vllm-fork": {"supports_priority": True}
                    }
                }
            }),
            encoding="utf-8",
        )
        (self.run / "scheduler.json").write_text(
            json.dumps({"policy": "priority"}),
            encoding="utf-8",
        )
        (self.run / "service.json").write_text(
            json.dumps({
                "name": "gptoss120b",
                "model": "openai/gpt-oss-120b",
                "max_num_seqs": 64,
                "scheduling_policy": "priority",
            }),
            encoding="utf-8",
        )
        (self.run / "service.log").write_text("vLLM ready\n", encoding="utf-8")
        (self.run / "manifest.json").write_text(
            json.dumps({
                "csvs": [".apxm/evaluation/run/results.csv"],
                "tenant_csvs": [".apxm/evaluation/run/results.tenants.csv"],
                "capability_json": ".apxm/evaluation/run/capabilities.json",
                "scheduler_evidence": ".apxm/evaluation/run/scheduler.json",
                "service_state": ".apxm/evaluation/run/service.json",
                "logs": [".apxm/evaluation/run/service.log"],
                "required_scheduler_policy": "priority",
            }),
            encoding="utf-8",
        )

    def lint(self) -> list[str]:
        return lint_skill_claims.lint_claim(self.run / "manifest.json", self.root)

    def test_passing_claim(self) -> None:
        self.assertEqual(self.lint(), [])

    def test_missing_csv_fails(self) -> None:
        (self.run / "results.csv").unlink()
        self.assertTrue(any("missing file" in error for error in self.lint()))

    def test_failed_tenants_fail(self) -> None:
        (self.run / "results.tenants.csv").write_text(
            "cell_label,returncode,wall_ms\nD,1,100.0\n",
            encoding="utf-8",
        )
        self.assertTrue(any("returncode=1" in error for error in self.lint()))

    def test_missing_capabilities_fail(self) -> None:
        (self.run / "capabilities.json").write_text("{}", encoding="utf-8")
        self.assertTrue(any("no graph_capabilities" in error for error in self.lint()))

    def test_unsupported_scheduler_policy_fails(self) -> None:
        (self.run / "scheduler.json").write_text(
            json.dumps({"policy": "fcfs"}),
            encoding="utf-8",
        )
        self.assertTrue(any("expected 'priority'" in error for error in self.lint()))


if __name__ == "__main__":
    unittest.main()
