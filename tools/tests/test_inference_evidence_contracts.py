"""Validate the product-neutral inference driver and evidence contract gate."""

from __future__ import annotations

import copy
import importlib.util
import json
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
VALIDATOR_PATH = REPOSITORY_ROOT / "contracts" / "tools" / "validate_owner_descriptor.py"
VECTORS_ROOT = REPOSITORY_ROOT / "contracts" / "vectors"

INFERENCE_VECTORS = {
    "apxm.inference-driver-binding.v1.json": "apxm.inference-driver-binding.v1",
    "apxm.inference-credential-lease.v1.json": "apxm.inference-credential-lease.v1",
    "apxm.inference-usage-lineage.v1.json": "apxm.inference-usage-lineage.v1",
    "apxm.diagnostic-correlation.v1.json": "apxm.diagnostic-correlation.v1",
    "apxm.vllm-conformance-join.v1.json": "apxm.vllm-conformance-join.v1",
}


def load_validator_module():
    spec = importlib.util.spec_from_file_location("validate_owner_descriptor", VALIDATOR_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load validate_owner_descriptor.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class InferenceEvidenceContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.validator = load_validator_module()
        cls.schema_ids = cls.validator.load_schema_ids()
        cls.schema_ids["apxm.contract-common.v1"] = {
            "$id": "apxm.contract-common.v1",
            "$defs": {
                "Identifier": {"type": "string", "minLength": 1},
                "Digest": {
                    "type": "string",
                    "pattern": r"sha256:[0-9a-f]{64}",
                },
            },
        }

    def _cases(self, vector_name: str) -> list[dict]:
        return json.loads((VECTORS_ROOT / vector_name).read_text(encoding="utf-8"))

    def _errors(self, vector_name: str, case: dict) -> list[str]:
        schema_id = INFERENCE_VECTORS[vector_name]
        schema = self.schema_ids[schema_id]
        errors = self.validator.schema_instance_errors(
            schema, case["input"], self.schema_ids, schema
        )
        errors.extend(self.validator.semantic_errors(schema_id, case["input"]))
        return errors

    def test_all_inference_vectors_pass_the_owner_gate(self) -> None:
        for vector_name in INFERENCE_VECTORS:
            cases = self._cases(vector_name)
            self.assertTrue(cases, vector_name)
            self.assertTrue(any(case["expected_valid"] for case in cases), vector_name)
            self.assertTrue(any(not case["expected_valid"] for case in cases), vector_name)
            for case in cases:
                actual = not self._errors(vector_name, case)
                self.assertEqual(actual, case["expected_valid"], case["name"])

    def test_usage_and_lease_ids_are_content_bound(self) -> None:
        lease = next(
            case["input"]
            for case in self._cases("apxm.inference-credential-lease.v1.json")
            if case["name"] == "valid-target-bound-lease-identity"
        )
        mutated_lease = copy.deepcopy(lease)
        mutated_lease["model_target_ref"] = "model-target.other"
        self.assertTrue(
            self.validator.inference_credential_lease_errors(mutated_lease)
        )

        lineage = next(
            case["input"]
            for case in self._cases("apxm.inference-usage-lineage.v1.json")
            if case["name"] == "valid-sealed-native-usage-lineage"
        )
        mutated_lineage = copy.deepcopy(lineage)
        mutated_lineage["native_output_tokens"] += 1
        self.assertTrue(self.validator.inference_usage_lineage_errors(mutated_lineage))

    def test_evidence_binding_requires_both_commit_coordinates(self) -> None:
        partial = next(
            case["input"]
            for case in self._cases("apxm.inference-usage-lineage.v1.json")
            if case["name"] == "reject-partial-evidence-binding"
        )
        self.assertIn(
            "bound together",
            self.validator.inference_usage_lineage_errors(partial)[0],
        )

    def test_diagnostic_references_are_bounded_and_non_authoritative(self) -> None:
        schema = self.schema_ids["apxm.diagnostic-correlation.v1"]
        base = next(
            case["input"]
            for case in self._cases("apxm.diagnostic-correlation.v1.json")
            if case["name"] == "valid-diagnostic-only-correlation"
        )
        bounded = copy.deepcopy(base)
        bounded["log_refs"] = [f"log.{index}" for index in range(65)]
        errors = self.validator.schema_instance_errors(
            schema, bounded, self.schema_ids, schema
        )
        self.assertIn("maxItems", " ".join(errors))
        self.assertEqual(base["authority"], "diagnostic_only")

    def test_revocation_and_private_material_cannot_enter_lease_identity(self) -> None:
        schema = self.schema_ids["apxm.inference-credential-lease.v1"]
        for case_name in ("reject-lease-secret-material", "reject-revocation-state-in-public-identity"):
            case = next(
                case["input"]
                for case in self._cases("apxm.inference-credential-lease.v1.json")
                if case["name"] == case_name
            )
            errors = self.validator.schema_instance_errors(schema, case, self.schema_ids, schema)
            self.assertTrue(errors, case_name)

    def test_backend_join_stays_candidate_without_live_release_evidence(self) -> None:
        cases = self._cases("apxm.vllm-conformance-join.v1.json")
        self.assertTrue(
            all(case["input"]["join_status"] == "candidate_awaiting_vllm_release" for case in cases if case["expected_valid"])
        )
        unknown = next(
            case["input"]
            for case in cases
            if case["name"] == "reject-unpinned-vector-digest"
        )
        self.assertTrue(self.validator.vllm_conformance_join_errors(unknown))
        joined_without_release = next(
            case["input"]
            for case in cases
            if case["name"] == "reject-joined-without-release-evidence"
        )
        self.assertEqual(
            self.validator.vllm_conformance_join_errors(joined_without_release),
            ["join_status requires exact external vLLM release evidence"],
        )


if __name__ == "__main__":
    unittest.main()
