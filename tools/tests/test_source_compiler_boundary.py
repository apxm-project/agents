"""The source/compiler lane rejects product dependencies and alternate AIR paths."""

from __future__ import annotations

import unittest

from tools.scripts import check_source_compiler_boundary


class SourceCompilerBoundaryTests(unittest.TestCase):
    def test_checked_in_source_compiler_boundary_is_clean(self) -> None:
        self.assertEqual(check_source_compiler_boundary.find_violations(), [])

    def test_product_reference_pattern_is_case_insensitive(self) -> None:
        self.assertIsNotNone(check_source_compiler_boundary.FORBIDDEN_PRODUCT_REFERENCES.search("Studio"))
        self.assertIsNotNone(check_source_compiler_boundary.FORBIDDEN_PRODUCT_REFERENCES.search("CLIC"))

    def test_retired_paths_are_explicitly_rejected(self) -> None:
        paths = {str(path) for path in check_source_compiler_boundary.RETIRED_PATHS}
        self.assertIn("crates/compiler/pipeline/src/air_builder", paths)
        self.assertIn("crates/compiler/frontend/python/apxm", paths)


if __name__ == "__main__":
    unittest.main()
