"""Guard the service-only gate and the service toolchain package set."""

from __future__ import annotations

import importlib.util
import tomllib
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SERVICE_GATE_PATH = REPOSITORY_ROOT / "tools" / "scripts" / "service_gate.py"
SERVICE_ENV_PATH = REPOSITORY_ROOT / "tools" / "scripts" / "service_env.py"
DEKK_MANIFEST_PATH = REPOSITORY_ROOT / ".dekk.toml"
COMPILER_MANIFEST_PATH = (
    REPOSITORY_ROOT / "crates" / "compiler" / "pipeline" / "Cargo.toml"
)
WORKSPACE_MANIFEST_PATH = REPOSITORY_ROOT / "Cargo.toml"


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


class ServiceGateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.gate = load_module(SERVICE_GATE_PATH, "service_gate")
        cls.env = load_module(SERVICE_ENV_PATH, "service_env")
        cls.manifest = read_toml(DEKK_MANIFEST_PATH)

    def test_every_gate_step_is_a_declared_dekk_command(self) -> None:
        commands = self.manifest["commands"]
        for name in self.gate.SERVICE_GATE_COMMANDS:
            self.assertIn(name, commands, f"{name} is not a .dekk.toml command")
            self.assertIn("run", commands[name])

    def test_gate_covers_the_services_protocols_frontends_and_qualification(self) -> None:
        required = {
            "fmt-check",
            "clippy",
            "test-compilation-protocol",
            "test-runtime-protocol",
            "test-compilation-service",
            "test-runtime-service",
            "test-python-frontend",
            "test-typescript-frontend",
            "test-source-port",
            "test-program",
            "test-kernel",
            "test-event-http",
            "test-execution",
            "test-release-qualification",
            "test-owner-qualification",
            "commit-lint",
        }
        self.assertTrue(required.issubset(set(self.gate.SERVICE_GATE_COMMANDS)))

    def test_gate_runs_no_step_that_needs_the_dialect(self) -> None:
        # These are the only gates that link MLIR. A service job that ran one
        # of them would need the toolchain this whole lane removes.
        forbidden = {"test-all", "test-compiler", "build-dialect", "build"}
        self.assertEqual(
            forbidden & set(self.gate.SERVICE_GATE_COMMANDS),
            set(),
        )

    def test_source_port_runs_after_both_frontends_are_built_and_installed(self) -> None:
        steps = [name for name, _ in self.gate.resolve_steps(self.gate.load_commands())]
        self.assertLess(
            steps.index("test-compilation-service"),
            steps.index("test-source-port"),
            "source-port conformance consumes the frontend build products",
        )

    def test_execution_runs_after_the_step_that_installs_the_frontend_lockfile(self) -> None:
        # `test-execution` runs `npm run build` but no `npm ci`, so it consumes
        # the locked dependencies `test-compilation-service` installs. Ordered
        # the other way it builds against whatever happens to be on disk.
        steps = [name for name, _ in self.gate.resolve_steps(self.gate.load_commands())]
        self.assertLess(
            steps.index("test-compilation-service"),
            steps.index("test-execution"),
        )
        self.assertNotIn(
            "npm ci", self.gate.load_commands()["test-execution"],
            "test-execution now installs its own dependencies; drop this ordering",
        )

    def test_the_only_readers_of_the_published_vectors_are_gate_steps(self) -> None:
        # `contract_vector_conformance` (apxm-program) holds every published
        # vector file against its published schema, and
        # `contract_document_conformance` (apxm-execution) holds the same
        # vectors against the Rust carriers. Neither ran under this gate until
        # the suites that own them were added, which left `test-all` — an MLIR
        # gate — as the only thing that read the contracts at all.
        steps = set(self.gate.SERVICE_GATE_COMMANDS)
        for suite in ("test-program", "test-execution"):
            self.assertIn(suite, steps)
        commands = self.gate.load_commands()
        self.assertIn("-p apxm-program", commands["test-program"])
        self.assertIn("-p apxm-execution", commands["test-execution"])
        for suite in ("test-program", "test-kernel", "test-execution"):
            self.assertNotIn(
                "--test ", commands[suite],
                f"{suite} names individual test targets, so a new one joins no gate",
            )

    def test_steps_resolve_to_the_declared_run_strings(self) -> None:
        commands = self.gate.load_commands()
        steps = dict(self.gate.resolve_steps(commands))
        for name in self.gate.SERVICE_GATE_COMMANDS:
            if name == self.gate.COMMIT_LINT_COMMAND:
                continue
            self.assertEqual(steps[name], commands[name])

    def test_commit_lint_takes_current_by_default_and_a_range_when_given(self) -> None:
        commands = self.gate.load_commands()
        default = dict(self.gate.resolve_steps(commands))
        self.assertTrue(
            default[self.gate.COMMIT_LINT_COMMAND].endswith(" --current"),
            default[self.gate.COMMIT_LINT_COMMAND],
        )
        ranged = dict(
            self.gate.resolve_steps(commands, commit_lint_range="base..head")
        )
        self.assertTrue(
            ranged[self.gate.COMMIT_LINT_COMMAND].endswith(" --range base..head"),
            ranged[self.gate.COMMIT_LINT_COMMAND],
        )

    def test_skip_omits_a_step_and_rejects_a_name_the_gate_does_not_run(self) -> None:
        commands = self.gate.load_commands()
        kept = dict(
            self.gate.resolve_steps(commands, skip=(self.gate.COMMIT_LINT_COMMAND,))
        )
        self.assertNotIn(self.gate.COMMIT_LINT_COMMAND, kept)
        self.assertEqual(len(kept), len(self.gate.SERVICE_GATE_COMMANDS) - 1)
        with self.assertRaises(KeyError):
            self.gate.resolve_steps(commands, skip=("test-all",))

    def test_service_environment_drops_every_mlir_toolchain_variable(self) -> None:
        base = {key: "set" for key in self.gate.MLIR_ENVIRONMENT_KEYS}
        base["PATH"] = "/usr/bin"
        resolved = self.gate.service_environment(base)
        self.assertEqual(resolved, {"PATH": "/usr/bin"})

    def test_dekk_env_mlir_variables_are_all_stripped(self) -> None:
        # Anything `[env]` points into the conda prefix for MLIR must be in the
        # strip list, or a run would inherit a dialect the service env lacks.
        declared = self.manifest["env"]
        pointing_at_mlir = {
            key
            for key, value in declared.items()
            if isinstance(value, str)
            and ("MLIR" in key or "LLVM" in key or key in {"CC", "CXX"})
        }
        self.assertTrue(
            pointing_at_mlir.issubset(set(self.gate.MLIR_ENVIRONMENT_KEYS)),
            pointing_at_mlir - set(self.gate.MLIR_ENVIRONMENT_KEYS),
        )


class ServiceEnvironmentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.env = load_module(SERVICE_ENV_PATH, "service_env")
        cls.manifest = read_toml(DEKK_MANIFEST_PATH)

    def test_service_package_set_is_the_declared_set_minus_the_mlir_half(self) -> None:
        declared = set(self.manifest["environment"]["packages"])
        specs = self.env.service_packages(self.manifest["environment"])
        names = {spec.split("=", 1)[0] for spec in specs}
        self.assertEqual(names, declared - self.env.MLIR_ONLY_PACKAGES)

    def test_every_mlir_only_package_is_actually_declared(self) -> None:
        declared = set(self.manifest["environment"]["packages"])
        self.assertTrue(
            self.env.MLIR_ONLY_PACKAGES.issubset(declared),
            self.env.MLIR_ONLY_PACKAGES - declared,
        )

    def test_service_package_set_carries_python_node_and_no_toolchain(self) -> None:
        specs = self.env.service_packages(self.manifest["environment"])
        names = {spec.split("=", 1)[0] for spec in specs}
        self.assertIn("python", names)
        self.assertIn("nodejs", names)
        for absent in ("mlir", "libclang13", "clang", "cmake", "ninja"):
            self.assertNotIn(absent, names)

    def test_provision_installs_into_an_existing_prefix_and_creates_a_new_one(self) -> None:
        create = self.env.provision_command(
            "/opt/conda/bin/conda",
            Path("/tmp/prefix"),
            ["python=3.11"],
            ["conda-forge"],
            exists=False,
        )
        self.assertIn("create", create)
        install = self.env.provision_command(
            "/opt/conda/bin/conda",
            Path("/tmp/prefix"),
            ["python=3.11"],
            ["conda-forge"],
            exists=True,
        )
        self.assertIn("install", install)


class MlirFeatureGatingTests(unittest.TestCase):
    def test_only_the_compiler_crate_declares_the_mlir_feature(self) -> None:
        compiler = read_toml(COMPILER_MANIFEST_PATH)
        self.assertIn("mlir", compiler["features"])
        self.assertEqual(compiler["features"]["mlir"], ["dep:bindgen"])

    def test_bindgen_is_optional_so_a_service_build_never_needs_libclang(self) -> None:
        compiler = read_toml(COMPILER_MANIFEST_PATH)
        bindgen = compiler["build-dependencies"]["bindgen"]
        self.assertTrue(bindgen.get("optional"), bindgen)

    def test_no_crate_depends_on_the_compiler_pipeline(self) -> None:
        # The service build graph is MLIR-free only while this holds: the
        # dialect is reachable from `apxm-compiler` and nothing else.
        members = read_toml(WORKSPACE_MANIFEST_PATH)["workspace"]["members"]
        dependents = []
        for member in members:
            manifest_path = REPOSITORY_ROOT / member / "Cargo.toml"
            manifest = read_toml(manifest_path)
            if manifest["package"]["name"] == "apxm-compiler":
                continue
            for table in ("dependencies", "dev-dependencies", "build-dependencies"):
                if "apxm-compiler" in manifest.get(table, {}):
                    dependents.append(f"{member} ({table})")
        self.assertEqual(dependents, [])

    def test_the_aggregate_suite_asks_for_the_feature_by_name(self) -> None:
        manifest = read_toml(DEKK_MANIFEST_PATH)
        self.assertIn(
            "--features apxm-compiler/mlir", manifest["commands"]["test-all"]["run"]
        )
        self.assertIn("--features mlir", manifest["commands"]["test-compiler"]["run"])


if __name__ == "__main__":
    unittest.main()
