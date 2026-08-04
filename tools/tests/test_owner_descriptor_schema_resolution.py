"""Regression coverage for owner-descriptor cross-schema JSON Schema refs."""

from __future__ import annotations

import importlib.util
import json
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
VALIDATOR_PATH = REPOSITORY_ROOT / "contracts" / "tools" / "validate_owner_descriptor.py"
AIR_VECTORS_PATH = REPOSITORY_ROOT / "contracts" / "vectors" / "apxm.air.v1.json"


def load_validator_module():
    spec = importlib.util.spec_from_file_location("validate_owner_descriptor", VALIDATOR_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load validate_owner_descriptor.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class OwnerDescriptorSchemaResolutionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.validator = load_validator_module()
        cls.schema_ids = cls.validator.load_schema_ids()

    def test_load_schema_ids_includes_canonical_contract_common_schema(self) -> None:
        self.assertIn("apxm.contract-common.v1", self.schema_ids)

    def test_air_vector_resolves_contract_common_identifier_refs(self) -> None:
        schema = self.schema_ids["apxm.air.v1"]
        cases = json.loads(AIR_VECTORS_PATH.read_text(encoding="utf-8"))
        case = next(
            item
            for item in cases
            if item["name"] == "valid-air-five-semantic-ops-and-structural-ir"
        )

        errors = self.validator.schema_instance_errors(
            schema,
            case["input"],
            self.schema_ids,
            schema,
        )

        self.assertEqual(errors, [])

    def test_unknown_external_ref_stays_fail_closed(self) -> None:
        schema = self.schema_ids["apxm.air.v1"]
        errors = self.validator.schema_instance_errors(
            {"$ref": "apxm.contract-common.v1#/$defs/DoesNotExist"},
            "node_0",
            self.schema_ids,
            schema,
        )

        self.assertEqual(
            errors,
            ["unresolved $ref 'apxm.contract-common.v1#/$defs/DoesNotExist'"],
        )


if __name__ == "__main__":
    unittest.main()
