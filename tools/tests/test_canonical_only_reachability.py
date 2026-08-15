"""Guard the Agents build graph against retired execution surfaces."""

from __future__ import annotations

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
    Path("examples/agents/gao"),
)
RETIRED_PACKAGE_DIRECTORIES = (Path("crates/compiler/frontend/python/apxm"),)
RETIRED_FILES = (
    Path("crates/compiler/frontend/python/apxm_program/conversational.py"),
    Path("crates/compiler/frontend/python/apxm_program/gao.py"),
    Path("crates/compiler/frontend/native/typescript/js/conversational.ts"),
    Path("crates/compiler/frontend/native/typescript/js/gao.ts"),
    # ADR-0019: no Agents crate holds a durable timer, wall-clock firing
    # schedule, persisted schedule lifecycle, or process-global wake bridge.
    Path("crates/runtime/capability/src/builtins/schedule.rs"),
    Path("crates/runtime/capability/src/builtins/store.rs"),
    Path("crates/runtime/capability-iface/src/host.rs"),
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
    "Gao",
    "SpecialistComposition",
    "TurnSpec",
    "conversational_loop",
)
COMMON_CONTRACT_ROOTS = (
    Path("crates/machine/ais/src"),
    Path("crates/machine/contracts/src"),
    Path("crates/machine/program/src"),
    Path("crates/runtime/inference/src"),
)
# ADR-0021: the common graph-hint contract owns facts and intents. A backend's
# scheduler, cache representation, queue, worker, slot, block, handle, or
# extension envelope is owned by the adapter that implements it, so none of
# these names may reappear in a common contract module.
FORBIDDEN_PROVIDER_MECHANISM_SEMANTICS = (
    "vllm_xargs",
    "id_slot",
    "pin_policy",
    "pinned_handles",
    "pinned_blocks",
    "PinMode",
    "PinPolicy",
    "PriorityClass",
    "supports_priority",
    "supports_pin_release",
    "REQUEST_PRIORITY",
    "MECHANISM_VLLM",
    "MECHANISM_LLAMA",
    "cache_salt",
)
# §12A.4: graph hints are compiler/runtime metadata about an admitted graph,
# never authored program structure. An authoring frontend that could name them
# would let a program request backend behavior, which is precisely what makes
# hints advisory rather than authority-bearing.
AUTHORING_SURFACE_ROOTS = (
    Path("crates/compiler/frontend/python/apxm_program"),
    Path("crates/compiler/frontend/typescript/src"),
    Path("crates/compiler/frontend/native"),
    Path("examples"),
)
FORBIDDEN_AUTHORED_GRAPH_HINT_SEMANTICS = (
    "apxm.inference-graph-hints",
    "ApxmGraphHints",
    "apxm_hints",
    "graph_hints",
    "graphHints",
    "successor_refs",
    "affinity_ref",
    "coexecution_group_ref",
    "prefix_warmup_eligible",
    "benefit_horizon_ms",
    "expected_shared_prefix_tokens",
)
# ADR-0021: an adapter records what it planned and what it projected. A
# provider acknowledgement and an outcome measurement are separate claims, and
# no provider response any adapter parses states either, so the machine has no
# vocabulary for declaring or recording them. These names may return only
# alongside the response parsing that produces one on a real dispatch path —
# a constructor that can only ever report "nothing" is not that.
UNPRODUCIBLE_EVIDENCE_ROOTS = (
    Path("crates/machine/contracts/src"),
    Path("crates/runtime/backends/src"),
    Path("crates/runtime/inference/src"),
    Path("crates/runtime/execution/src"),
)
UNPRODUCIBLE_EVIDENCE_MARKERS = (
    "GraphHintRealization",
    "FieldRealization",
    "GraphHintMeasurement",
    "MeasurementName",
    "ReusedInputTokens",
    "BackendAcknowledged",
    "BackendRejected",
    "BackendAcknowledgement",
    "OutcomeMeasurement",
    # The lifecycle capability a binding declares is `GraphLifecycleCapability`,
    # which rides on the capability digest. This boolean gated nothing.
    "supports_graph_extensions",
)
# ADR-0021 §3.3: projection is exact-binding local. There is no broadcast,
# provider discovery, graph-aware backend search, ranking, or fallback. These
# names were the broadcast chain; a graph lifecycle that returns must be bound
# to the one admitted adapter, not fanned out across a registry.
RETIRED_GRAPH_BROADCAST_MARKERS = (
    "register_graph_all",
    "release_graph_all",
    "find_graph_aware_backends",
    "pre_release_status_all",
    "GraphLifecycleOutcome",
)
EXAMPLE_RUNTIME_PROOF_FIXTURES = (
    Path("crates/machine/program/tests/fixtures/example-artifacts/conversational-python.json"),
    Path("crates/machine/program/tests/fixtures/example-artifacts/conversational-typescript.json"),
)
RETIRED_OPERATION_MARKERS = (
    "prototype_retired",
    "LegacyOperationType",
    "get_all_legacy_operations",
    "get_legacy_operation_spec",
)


def buildable_source(text: str) -> str:
    """The part of a Rust source file that ships.

    Unit tests may name a provider mechanism to prove it is rejected, and a doc
    comment may cite one as an example, so both are dropped before the scan.
    """
    shipped, _, _ = text.partition("#[cfg(test)]")
    return "\n".join(
        line
        for line in shipped.splitlines()
        if not line.lstrip().startswith(("//", "#"))
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
        """Authoring imports only the canonical `apxm_program` package."""
        pattern = re.compile(r"^\s*(?:from|import)\s+apxm(?:\.|\s|$)", re.MULTILINE)
        offenders: list[str] = []
        for root in ("tools", "crates/compiler/frontend/python", "examples"):
            for path in (REPOSITORY_ROOT / root).rglob("*.py"):
                if pattern.search(path.read_text(errors="ignore")):
                    offenders.append(str(path.relative_to(REPOSITORY_ROOT)))
        self.assertEqual(offenders, [], "\n".join(offenders))

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

    def test_common_contracts_name_no_provider_mechanism(self) -> None:
        """ADR-0021 Phase F: mechanisms live with the adapter that owns them."""
        offenders: list[str] = []
        for root in COMMON_CONTRACT_ROOTS:
            for path in (REPOSITORY_ROOT / root).rglob("*.rs"):
                text = buildable_source(path.read_text(errors="ignore"))
                markers = [
                    marker
                    for marker in FORBIDDEN_PROVIDER_MECHANISM_SEMANTICS
                    if marker in text
                ]
                if markers:
                    offenders.append(
                        f"{path.relative_to(REPOSITORY_ROOT)}: {', '.join(markers)}"
                    )
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_graph_hints_cannot_originate_in_authored_source(self) -> None:
        """Graph hints are compiler/runtime metadata, never authored structure."""
        offenders: list[str] = []
        for root in AUTHORING_SURFACE_ROOTS:
            root_path = REPOSITORY_ROOT / root
            if not root_path.exists():
                continue
            for path in root_path.rglob("*"):
                if not path.is_file() or path.suffix not in {".py", ".ts", ".rs", ".json"}:
                    continue
                if "node_modules" in path.parts or "dist" in path.parts:
                    continue
                text = path.read_text(errors="ignore")
                markers = [
                    marker
                    for marker in FORBIDDEN_AUTHORED_GRAPH_HINT_SEMANTICS
                    if marker in text
                ]
                if markers:
                    offenders.append(
                        f"{path.relative_to(REPOSITORY_ROOT)}: {', '.join(markers)}"
                    )
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_no_graph_hint_evidence_layer_reports_only_nothing(self) -> None:
        """ADR-0021: a declarable evidence layer must have a producer."""
        offenders: list[str] = []
        for root in UNPRODUCIBLE_EVIDENCE_ROOTS:
            for path in (REPOSITORY_ROOT / root).rglob("*.rs"):
                text = buildable_source(path.read_text(errors="ignore"))
                markers = [
                    marker for marker in UNPRODUCIBLE_EVIDENCE_MARKERS if marker in text
                ]
                if markers:
                    offenders.append(
                        f"{path.relative_to(REPOSITORY_ROOT)}: {', '.join(markers)}"
                    )
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_no_graph_lifecycle_is_broadcast_across_the_registry(self) -> None:
        """ADR-0021 §3.3: graph projection and lifecycle are exact-binding local."""
        offenders: list[str] = []
        for root in UNPRODUCIBLE_EVIDENCE_ROOTS:
            for path in (REPOSITORY_ROOT / root).rglob("*.rs"):
                text = buildable_source(path.read_text(errors="ignore"))
                markers = [
                    marker
                    for marker in RETIRED_GRAPH_BROADCAST_MARKERS
                    if marker in text
                ]
                if markers:
                    offenders.append(
                        f"{path.relative_to(REPOSITORY_ROOT)}: {', '.join(markers)}"
                    )
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_examples_have_immutable_generic_runtime_proof_fixtures(self) -> None:
        for path in EXAMPLE_RUNTIME_PROOF_FIXTURES:
            fixture = REPOSITORY_ROOT / path
            self.assertTrue(fixture.is_file(), f"missing example runtime-proof fixture: {path}")
            text = fixture.read_text()
            self.assertIn('"schema_version":"apxm.executable-artifact"', text)
            self.assertIn('"kind":"ais.loop"', text)
            self.assertNotIn("conversational_loop", text)

    def test_repository_example_named_sources_remain_outside_packages(self) -> None:
        conversational = (
            REPOSITORY_ROOT
            / "examples/agents/conversational/src/conversational-agent.ts"
        ).read_text()
        coder = (REPOSITORY_ROOT / "examples/agents/coder/src/main.ts").read_text()
        self.assertIn("ConversationalExample", conversational)
        self.assertIn("Coder", coder)

    def test_workspace_has_no_retired_execution_members(self) -> None:
        cargo_toml = tomllib.loads((REPOSITORY_ROOT / "Cargo.toml").read_text())
        members = cargo_toml["workspace"]["members"]
        self.assertNotIn("crates/runtime/engine", members)
        self.assertNotIn("crates/orchestration/acp", members)
        self.assertNotIn("crates/runtime/aam", members)

    def test_dekk_publishes_the_canonical_execution_command(self) -> None:
        dekk_toml = tomllib.loads((REPOSITORY_ROOT / ".dekk.toml").read_text())
        command = dekk_toml["commands"]["execute-canonical"]
        run = command["run"]

        self.assertEqual(command["group"], "Compilation")
        self.assertIn("build -p apxm-cli --features dev --bin apxm-dev", run)
        self.assertIn('debug/apxm-dev\" execute-canonical', run)
        self.assertIn("--invocation-admission", run)
        self.assertIn("--release", run)
        self.assertIn("--provenance", run)
        self.assertNotIn("apxm_cli.py", run)
        self.assertNotIn("--features driver", run)

    def test_production_cli_does_not_own_compiler_runtime_composition_roots(self) -> None:
        cargo = (REPOSITORY_ROOT / "crates/tools/cli/Cargo.toml").read_text()
        self.assertIn('name = "apxm-dev"', cargo)
        self.assertIn('required-features = ["dev"]', cargo)
        self.assertIn("apxm-execution = { workspace = true, optional = true }", cargo)
        self.assertIn("apxm-kernel = { workspace = true, optional = true }", cargo)
        self.assertIn("apxm-source-port = { workspace = true, optional = true }", cargo)
        cli = (REPOSITORY_ROOT / "crates/tools/cli/src/commands/cli.rs").read_text()
        self.assertNotIn("/// Regenerate package metadata and integrity.toml.", cli)
        self.assertNotIn("    Build {\n        /// Agent directory to build", cli)

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

    def test_no_named_product_semantic_path_exists(self) -> None:
        op_spec = (
            REPOSITORY_ROOT / "crates/machine/ais/generated/op-spec.json"
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

    def test_retired_chat_watch_rollout_commands_are_gone(self) -> None:
        cli = (REPOSITORY_ROOT / "crates/tools/cli/src/commands/cli.rs").read_text()
        for marker in ("Commands::Chat", "enum Commands {\n    Chat", "pub enum RolloutAction"):
            self.assertNotIn("pub enum RolloutAction", cli)
        self.assertNotIn("Chat {", cli)
        self.assertNotIn("Watch {", cli)
        self.assertNotIn("Rollout {", cli)
        driver = (REPOSITORY_ROOT / "crates/runtime/execution/src/lib.rs").read_text()
        self.assertNotIn("resume_event,", driver)


if __name__ == "__main__":
    unittest.main()
