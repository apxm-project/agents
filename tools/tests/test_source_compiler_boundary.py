"""The source/compiler lane rejects product dependencies and alternate AIR paths."""

from __future__ import annotations

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from tools.scripts import check_source_compiler_boundary


class SourceCompilerBoundaryTests(unittest.TestCase):
    def test_checked_in_source_compiler_boundary_is_clean(self) -> None:
        self.assertEqual(check_source_compiler_boundary.find_violations(), [])

    def test_product_reference_pattern_is_case_insensitive(self) -> None:
        self.assertIsNotNone(
            check_source_compiler_boundary.FORBIDDEN_PRODUCT_REFERENCES.search(
                "studio.workflow"
            )
        )
        self.assertIsNotNone(
            check_source_compiler_boundary.FORBIDDEN_IMPORTS.search(
                "import { Agent } from '@apxm/host-sdk';"
            )
        )

    def test_mutated_forbidden_import_is_rejected(self) -> None:
        with TemporaryDirectory() as temporary:
            path = Path(temporary) / "mutated.py"
            path.write_text("from apxm_vllm import contract\n", encoding="utf-8")
            violations = check_source_compiler_boundary.scan_files((path,))

        self.assertEqual(len(violations), 1)
        self.assertIn("downstream dependency import", violations[0])

    def test_retired_paths_are_explicitly_rejected(self) -> None:
        paths = {str(path) for path in check_source_compiler_boundary.RETIRED_PATHS}
        self.assertIn("crates/compiler/pipeline/src/air_builder", paths)
        self.assertIn("crates/compiler/frontend/python/apxm", paths)

        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "retired/compiler").mkdir(parents=True)
            violations = check_source_compiler_boundary.retired_path_violations(
                root, (Path("retired/compiler"),)
            )

        self.assertEqual(violations, ["retired/compiler: retired source/compiler path remains"])

    def test_retired_frontend_generation_reference_is_rejected(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "docs" / "active.md"
            path.parent.mkdir(parents=True)
            path.write_text("The compiler accepts AIR v1.\n", encoding="utf-8")

            violations = (
                check_source_compiler_boundary.retired_generation_reference_violations(
                    (path,), root=root
                )
            )

        self.assertEqual(
            violations,
            ["docs/active.md:1: retired FrontendGraph/AIR generation reference"],
        )


if __name__ == "__main__":
    unittest.main()
