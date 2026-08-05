"""Pin the Agents owner descriptor/reference-host cohort to one exact Host SDK tuple."""

from __future__ import annotations

import copy
import importlib.util
import json
import os
import platform
import re
import subprocess
import tomllib
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
VALIDATOR_PATH = REPOSITORY_ROOT / "contracts" / "tools" / "validate_owner_descriptor.py"
DESCRIPTOR_PATH = (
    REPOSITORY_ROOT / "contracts" / "descriptors" / "apxm.agents-owner-descriptor.v1.json"
)
CONTRACTS_README = REPOSITORY_ROOT / "contracts" / "README.md"
CARGO_TOML = REPOSITORY_ROOT / "Cargo.toml"
CARGO_LOCK = REPOSITORY_ROOT / "Cargo.lock"
REFERENCE_HOST_RELEASE_MANIFEST = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "manifests"
    / "apxm.reference-host-release-manifest.v1.json"
)
REFERENCE_HOST_EXECUTION_MANIFEST = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "manifests"
    / "apxm.reference-host-execution-manifest.v1.json"
)
REFERENCE_HOST_LIFECYCLE_VECTOR = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "vectors"
    / "apxm.reference-host.lifecycle-parity.v1.json"
)
REFERENCE_HOST_SOURCE = (
    REPOSITORY_ROOT / "crates" / "tools" / "cli" / "src" / "bin" / "reference_host.rs"
)
REFERENCE_HOST_STARTUP_INPUT_FIXTURE = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "fixtures"
    / "apxm.reference-host.startup-input.test.json"
)

README_REFERENCE_HOST = re.compile(
    r"This consumer is pinned to Host SDK source revision\s+"
    r"`(?P<revision>[0-9a-f]{40})`\.\s+Its\s+"
    r"`(?P<schema_version>apxm\.host-sdk-owner-descriptor\.v1)` semantic digest is\s+"
    r"`(?P<semantic_digest>sha256:[0-9a-f]{64})`,\s+and the SHA-256 checksum of the exact "
    r"descriptor repository bytes is\s+`(?P<exact_checksum>sha256:[0-9a-f]{64})`\.",
    re.MULTILINE,
)
FORBIDDEN_BOUNDARY_TOKENS = (
    "apxm.coordinator-owner-descriptor.v1",
    "apxm.clic-owner-descriptor.v1",
    "apxm.host-owner-descriptor.v1",
    "apxm.hostsdk-owner-descriptor.v1",
    "apxm.host_sdk-owner-descriptor.v1",
    '"semantic_owner": "coordinator"',
    '"semantic_owner": "clic"',
)


def load_validator_module():
    spec = importlib.util.spec_from_file_location("validate_owner_descriptor", VALIDATOR_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load validate_owner_descriptor.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_json(path: Path) -> dict[str, object]:
    return json.loads(path.read_text(encoding="utf-8"))


def git_head_revision() -> str:
    env = os.environ.copy()
    if platform.system() == "Darwin":
        env.pop("DYLD_LIBRARY_PATH", None)
        env.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
        env.pop("LD_LIBRARY_PATH", None)
    return subprocess.check_output(
        ["git", "rev-parse", "HEAD"],
        cwd=REPOSITORY_ROOT,
        text=True,
        env=env,
    ).strip()


class OwnerDescriptorReferenceHostTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.validator = load_validator_module()
        cls.expected_reference = dict(cls.validator.REFERENCE_HOST_DESCRIPTOR)

    def test_descriptor_reference_host_cohort_is_exact(self) -> None:
        descriptor = json.loads(DESCRIPTOR_PATH.read_text(encoding="utf-8"))
        self.assertEqual(
            descriptor["referenced_owner_descriptors"],
            [self.expected_reference],
            "the owner descriptor must reference exactly one canonical Host SDK cohort",
        )

    def test_workspace_host_sdk_dependency_matches_the_reference_host(self) -> None:
        manifest = tomllib.loads(CARGO_TOML.read_text(encoding="utf-8"))
        dependency = manifest["workspace"]["dependencies"]["apxm-host-sdk"]
        self.assertEqual(dependency["git"], self.validator.REFERENCE_HOST_DEPENDENCY_GIT)
        self.assertEqual(dependency["rev"], self.expected_reference["source_revision"])

    def test_lockfile_host_sdk_source_matches_the_reference_host(self) -> None:
        match = re.search(
            r'name = "apxm-host-sdk"\nversion = "0\.1\.0"\nsource = '
            r'"git\+https://github\.com/apxm-project/host-sdk\.git\?rev='
            r'(?P<rev>[0-9a-f]{40})#(?P=rev)"',
            CARGO_LOCK.read_text(encoding="utf-8"),
        )
        self.assertIsNotNone(match, "Cargo.lock must pin apxm-host-sdk to a git revision")
        assert match is not None
        self.assertEqual(match.group("rev"), self.expected_reference["source_revision"])

    def test_contracts_readme_matches_the_reference_host_cohort(self) -> None:
        readme = CONTRACTS_README.read_text(encoding="utf-8")
        match = README_REFERENCE_HOST.search(readme)
        self.assertIsNotNone(
            match,
            "contracts/README.md must spell out the exact referenced Host SDK cohort",
        )
        assert match is not None
        self.assertEqual(match.group("revision"), self.expected_reference["source_revision"])
        self.assertEqual(match.group("schema_version"), self.expected_reference["schema_version"])
        self.assertEqual(
            match.group("semantic_digest"),
            self.expected_reference["descriptor_semantic_digest"],
        )
        self.assertEqual(
            match.group("exact_checksum"),
            self.expected_reference["descriptor_exact_checksum"],
        )

    def test_boundary_sources_do_not_name_retired_or_foreign_reference_hosts(self) -> None:
        offenders: list[str] = []
        for path in (DESCRIPTOR_PATH, CONTRACTS_README, CARGO_TOML):
            text = path.read_text(encoding="utf-8")
            matches = [token for token in FORBIDDEN_BOUNDARY_TOKENS if token in text]
            if re.search(r"\bCLIC\b", text):
                matches.append("CLIC")
            if matches:
                offenders.append(
                    f"{path.relative_to(REPOSITORY_ROOT)}: {', '.join(sorted(set(matches)))}"
                )
        self.assertEqual(offenders, [], "\n".join(offenders))

    def test_reference_host_release_manifest_attests_exact_lifecycle_cohort(self) -> None:
        descriptor = load_json(DESCRIPTOR_PATH)
        release_manifest = load_json(REFERENCE_HOST_RELEASE_MANIFEST)
        execution_manifest = load_json(REFERENCE_HOST_EXECUTION_MANIFEST)
        lifecycle_vector = load_json(REFERENCE_HOST_LIFECYCLE_VECTOR)
        expected_release_manifest_ref = self.validator.reference_host_release_manifest_ref()
        expected_execution_manifest_ref = self.validator.reference_host_execution_manifest_ref()
        expected_publication_cohort = self.validator.reference_host_publication_cohort(
            execution_manifest
        )
        expected_lifecycle_vector_ref = self.validator.reference_host_lifecycle_vector_ref(
            lifecycle_vector
        )
        expected_profile = self.validator.reference_host_published_lifecycle_profile(
            release_manifest,
            execution_manifest,
            lifecycle_vector,
        )
        expected_attestation = {
            "profile_cohort": list(self.validator.REFERENCE_HOST_PROFILE_COHORT),
            "execution_manifest": expected_execution_manifest_ref,
            "lifecycle_vector": expected_lifecycle_vector_ref,
            "shared_contracts": lifecycle_vector["shared_contracts"],
            "required_cases": [case["name"] for case in lifecycle_vector["cases"]],
        }

        self.assertEqual(
            release_manifest["profile_cohort"],
            list(self.validator.REFERENCE_HOST_PROFILE_COHORT),
        )
        self.assertEqual(execution_manifest["semantic_owner"], "agents")
        for field in ("owner_executable", "owner_executable_path", "transport_protocol"):
            self.assertEqual(execution_manifest[field], release_manifest[field])
        self.assertEqual(
            release_manifest["publication_cohort"],
            expected_publication_cohort,
            "the release manifest must publish the exact schema/vector cohort from the execution manifest",
        )
        self.assertEqual(
            release_manifest["lifecycle_cohort_attestation"],
            expected_attestation,
            "the reference-host release manifest must attest the exact lifecycle cohort",
        )
        self.assertEqual(
            descriptor["published_host_lifecycle_profiles"],
            [expected_profile],
            "the owner descriptor must publish the exact reference-host lifecycle profile",
        )
        self.assertEqual(
            descriptor["published_host_lifecycle_profiles"][0]["release_manifest"],
            expected_release_manifest_ref,
        )

    def test_reference_host_release_manifest_rejects_retired_admission_alias(self) -> None:
        release_manifest_text = REFERENCE_HOST_RELEASE_MANIFEST.read_text(encoding="utf-8")
        self.assertNotIn(
            self.validator.RETIRED_REFERENCE_HOST_ADMISSION_ALIAS,
            release_manifest_text,
        )

    def test_reference_host_release_attestation_matches_committed_owner_artifacts(self) -> None:
        revision = git_head_revision()
        descriptor_digest = self.validator.descriptor_exact_checksum()
        attestation = self.validator.reference_host_release_attestation(
            owner_revision=revision,
            owner_descriptor_digest=descriptor_digest,
        )

        self.assertEqual(
            attestation["cohort"],
            {
                "revision": revision,
                "descriptor_digest": descriptor_digest,
                "manifest_digest": self.validator.file_digest(REFERENCE_HOST_RELEASE_MANIFEST),
            },
        )
        self.assertEqual(
            attestation["release_manifest"],
            {
                "path": "contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json",
                "exact_bytes_digest": self.validator.file_digest(REFERENCE_HOST_RELEASE_MANIFEST),
            },
        )
        self.assertEqual(
            [entry["path"] for entry in attestation["golden_vectors"]],
            [
                "contracts/reference-host/vectors/apxm.reference-host.invoke-parity.v1.json",
                "contracts/reference-host/vectors/apxm.reference-host.lifecycle-parity.v1.json",
            ],
        )
        self.assertIn(
            {
                "kind": "owned-schema",
                "contract_id": "apxm.host-execution-manifest.v1",
                "path": "contracts/schemas/apxm.host-execution-manifest.v1.json",
                "exact_bytes_digest": self.validator.file_digest(
                    REPOSITORY_ROOT
                    / "contracts"
                    / "schemas"
                    / "apxm.host-execution-manifest.v1.json"
                ),
            },
            attestation["owner_source_contracts"],
        )
        self.assertIn(
            {
                "schema_version": "apxm.reference-host-execution-manifest.v1",
                "path": "contracts/reference-host/manifests/apxm.reference-host-execution-manifest.v1.json",
                "exact_bytes_digest": self.validator.file_digest(REFERENCE_HOST_EXECUTION_MANIFEST),
            },
            attestation["publication_cohort"]["manifests"],
        )

    def test_reference_host_source_requires_explicit_startup_input_and_no_placeholders(self) -> None:
        source = REFERENCE_HOST_SOURCE.read_text(encoding="utf-8")
        self.assertIn("--startup-input", source)
        for placeholder in (
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        ):
            self.assertNotIn(placeholder, source)

    def test_reference_host_execution_manifest_pins_test_only_startup_input_fixture(self) -> None:
        execution_manifest = load_json(REFERENCE_HOST_EXECUTION_MANIFEST)
        expected = {
            "scope": "test-only",
            "artifact_path": "reference-host/fixtures/apxm.reference-host.startup-input.test.json",
            "artifact_digest": self.validator.file_digest(REFERENCE_HOST_STARTUP_INPUT_FIXTURE),
            "owner_revision": self.expected_reference["source_revision"],
            "fail_closed_on": ["missing", "stale", "dirty", "mismatched"],
        }

        self.assertEqual(
            execution_manifest["startup_input_preflight_test_fixture"],
            expected,
            "the reference-host execution manifest must pin one exact test-scoped startup input fixture",
        )
        self.validator.check_reference_host_startup_input_preflight(execution_manifest)

    def test_reference_host_startup_input_preflight_rejects_missing_fixture(self) -> None:
        execution_manifest = load_json(REFERENCE_HOST_EXECUTION_MANIFEST)
        missing = copy.deepcopy(execution_manifest)
        missing["startup_input_preflight_test_fixture"]["artifact_path"] = (
            "reference-host/fixtures/does-not-exist.json"
        )

        with self.assertRaisesRegex(
            self.validator.ValidationError,
            "startup-input preflight fixture is missing",
        ):
            self.validator.check_reference_host_startup_input_preflight(missing)

    def test_reference_host_startup_input_preflight_rejects_digest_mismatch(self) -> None:
        execution_manifest = load_json(REFERENCE_HOST_EXECUTION_MANIFEST)
        mismatched = copy.deepcopy(execution_manifest)
        mismatched["startup_input_preflight_test_fixture"]["artifact_digest"] = (
            "sha256:" + ("0" * 64)
        )

        with self.assertRaisesRegex(
            self.validator.ValidationError,
            "startup-input preflight fixture digest mismatched",
        ):
            self.validator.check_reference_host_startup_input_preflight(mismatched)


if __name__ == "__main__":
    unittest.main()
