"""Guard the Agents build graph against retired execution surfaces."""

from __future__ import annotations

import tomllib
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
RETIRED_DIRECTORIES = (
    Path("crates/runtime/engine"),
    Path("crates/orchestration/acp"),
    Path("crates/runtime/aam"),
    Path("crates/compiler/pipeline/src/air_builder"),
)
RETIRED_FILES = (
    Path("crates/compiler/frontend/python/apxm/proxy.py"),
    Path("crates/compiler/frontend/python/apxm_program/gao.py"),
    Path("crates/compiler/frontend/native/typescript/js/gao.ts"),
)
RETIRED_OPERATION_MARKERS = (
    "prototype_retired",
    "LegacyOperationType",
    "get_all_legacy_operations",
    "get_legacy_operation_spec",
)


class CanonicalOnlyReachabilityTests(unittest.TestCase):
    """Prove buildable source reaches only the canonical execution spine."""

    def test_retired_execution_directories_are_absent(self) -> None:
        for path in RETIRED_DIRECTORIES:
            self.assertFalse(
                (REPOSITORY_ROOT / path).exists(),
                f"retired execution directory remains reachable: {path}",
            )

    def test_retired_frontend_builder_files_are_absent(self) -> None:
        for path in RETIRED_FILES:
            self.assertFalse(
                (REPOSITORY_ROOT / path).exists(),
                f"retired frontend builder remains reachable: {path}",
            )

    def test_workspace_has_no_retired_execution_members(self) -> None:
        cargo_toml = tomllib.loads((REPOSITORY_ROOT / "Cargo.toml").read_text())
        members = cargo_toml["workspace"]["members"]
        self.assertNotIn("crates/runtime/engine", members)
        self.assertNotIn("crates/orchestration/acp", members)
        self.assertNotIn("crates/runtime/aam", members)

    def test_machine_operation_contract_has_no_retired_catalogue(self) -> None:
        offenders: list[str] = []
        for root in ("crates/machine/ais", "crates/machine/contracts"):
            for path in (REPOSITORY_ROOT / root).rglob("*.rs"):
                text = path.read_text(errors="ignore")
                markers = [marker for marker in RETIRED_OPERATION_MARKERS if marker in text]
                if markers:
                    offenders.append(f"{path.relative_to(REPOSITORY_ROOT)}: {', '.join(markers)}")
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_mlir_dialect_includes_only_generated_semantic_operations(self) -> None:
        dialect = (
            REPOSITORY_ROOT
            / "crates/compiler/pipeline/mlir/include/ais/Dialect/AIS/IR/AISOps.td"
        )
        text = dialect.read_text()
        forbidden = ("AIS_AskOp", "AIS_AwaitInputOp", "AIS_RegisterHookOp", "AIS_FlowCallOp")
        for marker in forbidden:
            self.assertNotIn(marker, text)

    def test_gao_has_no_builder_or_special_semantic_path(self) -> None:
        gao_sources = (
            REPOSITORY_ROOT
            / "examples/agents/gao/src/gao.ts",
        )
        for path in gao_sources:
            text = path.read_text()
            self.assertNotIn("GraphBuilder", text, f"Gao bypasses public authoring in {path}")

        op_spec = (
            REPOSITORY_ROOT / "crates/machine/ais/generated/op-spec.v1.json"
        ).read_text()
        for marker in ('"gao"', '"conversation"', '"turn"'):
            self.assertNotIn(marker, op_spec.lower())

        forbidden_runtime_markers = (
            "SemanticOpKind::Gao",
            "SemanticOpKind::Conversation",
            "SemanticOpKind::Turn",
        )
        for root in (
            REPOSITORY_ROOT / "crates/machine/program/src",
            REPOSITORY_ROOT / "crates/runtime/execution/src",
        ):
            for path in root.rglob("*.rs"):
                text = path.read_text(errors="ignore")
                for marker in forbidden_runtime_markers:
                    self.assertNotIn(marker, text, f"special Gao path in {path}")


if __name__ == "__main__":
    unittest.main()
