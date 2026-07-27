"""Guard the Agents build graph against retired execution surfaces."""

from __future__ import annotations

import importlib.util
import re
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
RETIRED_PACKAGE_DIRECTORIES = (Path("crates/compiler/frontend/python/apxm"),)
RETIRED_FILES = (
    Path("crates/compiler/frontend/python/apxm_program/conversational.py"),
    Path("crates/compiler/frontend/python/apxm_program/gao.py"),
    Path("crates/compiler/frontend/native/typescript/js/conversational.ts"),
    Path("crates/compiler/frontend/native/typescript/js/gao.ts"),
)
PACKAGE_HANDLER_ROOTS = (
    Path("crates/machine/ais/src"),
    Path("crates/machine/contracts/src"),
    Path("crates/compiler/frontend/python/apxm_program"),
    Path("crates/compiler/frontend/typescript/src"),
    Path("crates/tools/cli/generated"),
)
PYTHON_PACKAGE_HANDLER_MARKERS = (
    "PythonHandler",
    "python_handler_id",
    "PYTHON_TOOL_MANIFEST",
)
GENERIC_SEMANTIC_ROOTS = (
    Path("crates/compiler/frontend/python/apxm_program"),
    Path("crates/compiler/frontend/typescript/src"),
    Path("crates/compiler/pipeline/src"),
    Path("crates/machine/program/src"),
    Path("crates/runtime/execution/src"),
)
FORBIDDEN_NAMED_SEMANTICS = (
    "ConversationalAgent",
    "SpecialistComposition",
    "TurnSpec",
    "conversational_loop",
)
EXAMPLE_RUNTIME_PROOF_FIXTURES = (
    Path("crates/machine/program/tests/fixtures/example-artifacts/conversational.v1.json"),
    Path("crates/machine/program/tests/fixtures/example-artifacts/gao.v1.json"),
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

    def test_python_authoring_ships_exactly_the_canonical_package(self) -> None:
        """`apxm_program` is the whole Python authoring frontend."""
        for path in RETIRED_PACKAGE_DIRECTORIES:
            self.assertFalse(
                (REPOSITORY_ROOT / path).exists(),
                f"retired authoring-frontend package remains reachable: {path}",
            )
        frontend_root = REPOSITORY_ROOT / "crates/compiler/frontend/python"
        packages = sorted(
            entry.name
            for entry in frontend_root.iterdir()
            if entry.is_dir() and (entry / "__init__.py").is_file()
        )
        self.assertEqual(packages, ["apxm_program"])

    def test_no_source_imports_the_retired_apxm_package(self) -> None:
        """Operator tooling imports `apxm_vllm`; authoring imports `apxm_program`."""
        pattern = re.compile(r"^\s*(?:from|import)\s+apxm(?:\.|\s|$)", re.MULTILINE)
        offenders: list[str] = []
        for root in ("tools", "crates/compiler/frontend/python", "examples"):
            for path in (REPOSITORY_ROOT / root).rglob("*.py"):
                if pattern.search(path.read_text(errors="ignore")):
                    offenders.append(str(path.relative_to(REPOSITORY_ROOT)))
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_operator_contract_resolves_from_the_declared_import_root(self) -> None:
        """`apxm_vllm` is importable from `tools/`, the declared PYTHONPATH entry."""
        package = REPOSITORY_ROOT / "tools" / "apxm_vllm"
        for module in ("__init__.py", "contract.py", "data_config.py"):
            self.assertTrue((package / module).is_file(), f"missing operator module: {module}")
        spec = importlib.util.find_spec("apxm_vllm.contract")
        self.assertIsNotNone(spec, "apxm_vllm.contract is not importable")
        self.assertEqual(Path(spec.origin).resolve(), (package / "contract.py").resolve())

    def test_no_python_package_local_handler_surface_remains(self) -> None:
        """Package-local handlers are TypeScript-only across every owned surface."""
        offenders: list[str] = []
        for root in PACKAGE_HANDLER_ROOTS:
            for path in (REPOSITORY_ROOT / root).rglob("*"):
                if not path.is_file() or path.suffix not in {".py", ".rs", ".ts", ".json"}:
                    continue
                text = path.read_text(errors="ignore")
                markers = [
                    marker for marker in PYTHON_PACKAGE_HANDLER_MARKERS if marker in text
                ]
                if markers:
                    offenders.append(f"{path.relative_to(REPOSITORY_ROOT)}: {', '.join(markers)}")
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_generic_compiler_and_runtime_sources_have_no_named_example_semantics(self) -> None:
        offenders: list[str] = []
        for root in GENERIC_SEMANTIC_ROOTS:
            for path in (REPOSITORY_ROOT / root).rglob("*"):
                if not path.is_file() or path.suffix not in {".py", ".rs", ".ts"}:
                    continue
                text = path.read_text(errors="ignore")
                markers = [marker for marker in FORBIDDEN_NAMED_SEMANTICS if marker in text]
                if markers:
                    offenders.append(f"{path.relative_to(REPOSITORY_ROOT)}: {', '.join(markers)}")
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_examples_have_immutable_generic_runtime_proof_fixtures(self) -> None:
        for path in EXAMPLE_RUNTIME_PROOF_FIXTURES:
            fixture = REPOSITORY_ROOT / path
            self.assertTrue(fixture.is_file(), f"missing example runtime-proof fixture: {path}")
            text = fixture.read_text()
            self.assertIn('"schema_version":"apxm.executable-artifact.v1"', text)
            self.assertIn('"kind":"ais.loop"', text)
            self.assertNotIn("conversational_loop", text)

    def test_example_local_named_sources_remain_allowed(self) -> None:
        conversational = (
            REPOSITORY_ROOT
            / "examples/agents/conversational/src/conversational-agent.ts"
        ).read_text()
        gao = (REPOSITORY_ROOT / "examples/agents/gao/src/gao.ts").read_text()
        self.assertIn("ConversationalExample", conversational)
        self.assertIn("Gao", gao)

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
