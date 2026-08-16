"""Tests for the product-neutral APXM owner release gate."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "scripts" / "release_qualification.py"


def git_environment() -> dict[str, str]:
    environment = dict(os.environ)
    if sys.platform == "darwin":
        environment.pop("DYLD_LIBRARY_PATH", None)
        environment.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
    return environment


def load_module():
    spec = importlib.util.spec_from_file_location("release_qualification", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load release qualification script")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def make_clean_owner_checkout(root: Path) -> tuple[str, dict[str, Path]]:
    """Create a tiny clean checkout with the two ignored APXM service binaries."""

    subprocess.run(["git", "init", "-q", str(root)], check=True, env=git_environment())
    subprocess.run(
        ["git", "-C", str(root), "config", "user.email", "tests@example.invalid"],
        check=True,
        env=git_environment(),
    )
    subprocess.run(
        ["git", "-C", str(root), "config", "user.name", "Release Qualification Tests"],
        check=True,
        env=git_environment(),
    )
    (root / ".gitignore").write_text("target/\n", encoding="utf-8")
    (root / "README.md").write_text("fixture\n", encoding="utf-8")
    subprocess.run(
        ["git", "-C", str(root), "add", ".gitignore", "README.md"],
        check=True,
        env=git_environment(),
    )
    subprocess.run(
        ["git", "-C", str(root), "commit", "-qm", "fixture"],
        check=True,
        env=git_environment(),
    )
    revision = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
        env=git_environment(),
    ).stdout.strip()
    artifacts = {
        "compilation-service": root / "target" / "release" / "apxm-compilation-service",
        "runtime-service": root / "target" / "release" / "apxm-runtime-service",
    }
    for index, artifact in enumerate(artifacts.values()):
        artifact.parent.mkdir(parents=True, exist_ok=True)
        artifact.write_bytes(f"real service bytes {index}\n".encode())
        artifact.chmod(0o755)
    return revision, artifacts


class ReleaseQualificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.qualification = load_module()

    def test_missing_publishable_artifact_fails_closed_with_actionable_diagnostic(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            result = self.qualification.qualify(root, run_gates=False)
        self.assertFalse(result.ok)
        rendered = "\n".join(item.render() for item in result.diagnostics)
        self.assertIn("missing-publishable-service-artifact", rendered)
        self.assertIn("apxm-compilation-service", rendered)
        self.assertIn("apxm-runtime-service", rendered)
        self.assertNotIn("reference-host", rendered)
        self.assertIn("runtime-activation-blocked", rendered)

    def test_gate_failures_are_classified_as_resource_abi_or_code(self) -> None:
        classify = self.qualification._classify_gate_failure
        self.assertEqual(classify(137, "Killed: 9"), "resource")
        self.assertEqual(classify(1, "error[E0514]: found crate compiled by an incompatible rustc"), "abi")
        self.assertEqual(classify(1, "assertion failed: expected artifact"), "code")

    def test_local_service_binaries_without_release_cohort_are_not_activation_ready(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            compilation = root / "apxm-compilation-service"
            runtime = root / "apxm-runtime-service"
            for artifact in (compilation, runtime):
                artifact.write_bytes(b"local build")
                artifact.chmod(0o755)
            result = self.qualification.qualify(
                root,
                compilation_service_path=str(compilation),
                runtime_service_path=str(runtime),
                run_gates=False,
            )
        self.assertFalse(result.ok)
        self.assertTrue(any(item.code == "unattested-service-artifacts" for item in result.diagnostics))

    def test_generation_uses_exact_service_bytes_and_writes_all_release_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            revision, artifacts = make_clean_owner_checkout(root)
            output = root / "out"
            source, owner, sidecar, manifest = self.qualification.generate_descriptors(
                root,
                compilation_service_path=str(artifacts["compilation-service"]),
                runtime_service_path=str(artifacts["runtime-service"]),
                output_dir=output,
                source_revision=revision,
            )
            payload = json.loads(manifest.read_text(encoding="utf-8"))
            expected = {
                name: "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
                for name, path in artifacts.items()
            }
            self.assertEqual(
                {item["name"]: item["digest"] for item in payload["services"]}, expected
            )
            self.assertNotIn("placeholder", sidecar.read_text(encoding="utf-8").lower())
            self.assertEqual(payload["source_revision"], revision)
            self.assertTrue(source.is_file())
            self.assertTrue(owner.is_file())

    def test_qualification_passes_only_after_publishing_exact_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            revision, artifacts = make_clean_owner_checkout(root)
            self.qualification.generate_descriptors(
                root,
                compilation_service_path=str(artifacts["compilation-service"]),
                runtime_service_path=str(artifacts["runtime-service"]),
                output_dir=root,
                source_revision=revision,
            )
            subprocess.run(
                ["git", "-C", str(root), "add", "deploy", "contracts"],
                check=True,
                env=git_environment(),
            )
            subprocess.run(
                ["git", "-C", str(root), "commit", "-qm", "publish fixture"],
                check=True,
                env=git_environment(),
            )
            result = self.qualification.qualify(
                root,
                compilation_service_path=str(artifacts["compilation-service"]),
                runtime_service_path=str(artifacts["runtime-service"]),
                run_gates=False,
            )
        self.assertTrue(result.ok, [item.render() for item in result.diagnostics])

    def test_generation_refuses_to_overwrite_different_release_input(self) -> None:
        with tempfile.TemporaryDirectory() as temporary, tempfile.TemporaryDirectory() as output_dir:
            root = Path(temporary)
            revision, artifacts = make_clean_owner_checkout(root)
            output = Path(output_dir)
            self.qualification.generate_descriptors(
                root,
                compilation_service_path=str(artifacts["compilation-service"]),
                runtime_service_path=str(artifacts["runtime-service"]),
                output_dir=output,
                source_revision=revision,
            )
            owner = output / self.qualification.OWNER_DESCRIPTOR_REL
            owner.write_text("{\"tampered\":true}\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "refusing to overwrite"):
                self.qualification.generate_descriptors(
                    root,
                    compilation_service_path=str(artifacts["compilation-service"]),
                    runtime_service_path=str(artifacts["runtime-service"]),
                    output_dir=output,
                    source_revision=revision,
                )

    def test_tampered_artifact_is_rejected_when_manifest_digest_is_present(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            revision, artifacts = make_clean_owner_checkout(root)
            self.qualification.generate_descriptors(
                root,
                compilation_service_path=str(artifacts["compilation-service"]),
                runtime_service_path=str(artifacts["runtime-service"]),
                output_dir=root,
                source_revision=revision,
            )
            artifacts["runtime-service"].write_bytes(b"tampered")
            result = self.qualification.qualify(
                root,
                compilation_service_path=str(artifacts["compilation-service"]),
                runtime_service_path=str(artifacts["runtime-service"]),
                run_gates=False,
            )
        self.assertFalse(result.ok)
        self.assertTrue(any(item.code == "service-artifact-digest-mismatch" for item in result.diagnostics))

    def test_manifest_must_bind_owner_descriptor_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            revision, artifacts = make_clean_owner_checkout(root)
            self.qualification.generate_descriptors(
                root,
                compilation_service_path=str(artifacts["compilation-service"]),
                runtime_service_path=str(artifacts["runtime-service"]),
                output_dir=root,
                source_revision=revision,
            )
            manifest_path = root / self.qualification.RELEASE_MANIFEST_REL
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            del manifest["owner_descriptor_digest"]
            manifest_path.write_text(json.dumps(manifest) + "\n", encoding="utf-8")
            result = self.qualification.qualify(
                root,
                compilation_service_path=str(artifacts["compilation-service"]),
                runtime_service_path=str(artifacts["runtime-service"]),
                run_gates=False,
            )
        self.assertFalse(result.ok)
        self.assertTrue(any(item.code == "invalid-schema" for item in result.diagnostics))

    def test_dekk_manifest_exposes_owner_qualification_commands(self) -> None:
        import tomllib

        manifest = tomllib.loads((ROOT / ".dekk.toml").read_text(encoding="utf-8"))
        commands = manifest["commands"]
        self.assertIn("release-qualification", commands)
        self.assertIn("release-descriptors", commands)
        self.assertIn("release_qualification.py", commands["release-qualification"]["run"])
        descriptor_command = commands["release-descriptors"]["run"]
        self.assertIn("APXM_COMPILATION_SERVICE_BINARY", descriptor_command)
        self.assertIn("APXM_RUNTIME_SERVICE_BINARY", descriptor_command)
        self.assertNotIn("target/release/apxm-compilation-service", descriptor_command)

    def test_dekk_manifest_exposes_distinct_p80_owner_declarations(self) -> None:
        import tomllib

        commands = tomllib.loads((ROOT / ".dekk.toml").read_text(encoding="utf-8"))["commands"]
        names = (
            "p80-e2e",
            "p80-journey-c",
            "p80-negative-recovery",
            "p80-journey-g",
            "p80-journey-h",
            "p80-restart-reopen",
        )
        runs = {name: commands[name]["run"] for name in names}
        self.assertEqual(len(set(runs.values())), len(names))
        for name, run in runs.items():
            self.assertIn("dekk agents release-qualification", run, name)
            self.assertIn("&&", run, name)


if __name__ == "__main__":
    unittest.main()
