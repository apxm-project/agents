"""Pin the service-image contract without needing a container runtime.

The images are the consumer boundary, so the properties that matter are the
ones a consumer relies on: every label a pin names is stamped, every stamped
digest is re-proved inside the build against the bytes the image carries, the
manifest path in the image is the path the release manifest publishes, and the
expensive stages of the two Dockerfiles are identical so one compile serves
both. `dekk agents verify-images` checks a built image; this checks the
contract that makes such an image verifiable, and runs in the MLIR-free gate.
"""

from __future__ import annotations

import importlib.util
import json
import tomllib
import unittest
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
DEKK_MANIFEST_PATH = REPOSITORY_ROOT / ".dekk.toml"
SERVICE_IMAGES_PATH = REPOSITORY_ROOT / "tools" / "scripts" / "service_images.py"
RELEASE_INSIDE_IMAGE = REPOSITORY_ROOT / "deploy" / "release-inside-image.sh"
RELEASE_MANIFEST = (
    REPOSITORY_ROOT
    / "contracts"
    / "services"
    / "manifests"
    / "apxm.agents-service-release-manifest.v1.json"
)

#: The instructions both Dockerfiles must share, so BuildKit compiles the
#: workspace once and both images carry bytes from the same compilation.
SHARED_STAGE_MARKERS = (
    "FROM ${APXM_BUILDER_IMAGE} AS builder",
    "FROM builder AS release",
)


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def instructions(text: str) -> list[str]:
    """Return one logical Dockerfile instruction per element, comments dropped."""

    joined = text.replace("\\\n", " ")
    return [
        " ".join(line.split())
        for line in joined.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]


class ServiceImageContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.images = load_module(SERVICE_IMAGES_PATH, "service_images")
        with DEKK_MANIFEST_PATH.open("rb") as handle:
            cls.manifest = tomllib.load(handle)
        cls.dockerfiles = {
            service: (REPOSITORY_ROOT / spec["dockerfile"]).read_text(encoding="utf-8")
            for service, spec in cls.images.SERVICES.items()
        }

    def test_every_image_builds_its_executable_from_the_checkout(self) -> None:
        for service, text in self.dockerfiles.items():
            with self.subTest(service=service):
                self.assertIn("COPY . /workspace", text)
                self.assertIn("cargo build --release --locked", text)
                self.assertNotIn(
                    "COPY target/release/",
                    text,
                    "a service image must not inherit a host build output",
                )

    def test_release_stage_regenerates_the_cohort_from_the_bytes_it_built(self) -> None:
        for service, text in self.dockerfiles.items():
            with self.subTest(service=service):
                self.assertIn("sh deploy/release-inside-image.sh /workspace /out", text)
        script = RELEASE_INSIDE_IMAGE.read_text(encoding="utf-8")
        self.assertIn("image-descriptors", script)
        for spec in self.images.SERVICES.values():
            self.assertIn(spec["executable"], script)

    def test_expensive_stages_are_identical_so_one_compile_serves_both(self) -> None:
        prefixes = []
        for text in self.dockerfiles.values():
            steps = instructions(text)
            end = steps.index("FROM ${APXM_BASE_IMAGE}")
            prefixes.append(steps[:end])
            for marker in SHARED_STAGE_MARKERS:
                self.assertIn(marker, steps)
        self.assertEqual(prefixes[0], prefixes[1])

    def test_every_pinned_label_is_stamped_and_re_proved_inside_the_build(self) -> None:
        expected = {
            "compilation-service": (
                self.images.SERVICE_DIGEST_LABEL,
                self.images.RELEASE_MANIFEST_DIGEST_LABEL,
                self.images.FRONTEND_NATIVE_DIGEST_LABEL,
                self.images.SERVICE_LABEL,
                self.images.REVISION_LABEL,
            ),
            "runtime-service": (
                self.images.SERVICE_DIGEST_LABEL,
                self.images.RELEASE_MANIFEST_DIGEST_LABEL,
                self.images.SERVICE_LABEL,
                self.images.REVISION_LABEL,
            ),
        }
        for service, labels in expected.items():
            text = self.dockerfiles[service]
            with self.subTest(service=service):
                for label in labels:
                    self.assertIn(f'{label}="', text)
                argument = self.images.SERVICES[service]["digest_arg"]
                self.assertIn(f"ARG {argument}", text)
                self.assertIn("ARG APXM_RELEASE_MANIFEST_DIGEST", text)
                # The stamped digest is only a claim until the build hashes the
                # bytes it stamped it from.
                self.assertIn(
                    f'| cut -d\' \' -f1)" = "${argument}" ]',
                    text,
                    "the build must reject an executable whose bytes drift from its label",
                )
                self.assertIn(
                    '| cut -d\' \' -f1)" = "$APXM_RELEASE_MANIFEST_DIGEST" ]',
                    text,
                    "the build must reject a manifest whose bytes drift from its label",
                )

    def test_base_images_are_immutable_digest_references(self) -> None:
        for service, text in self.dockerfiles.items():
            with self.subTest(service=service):
                for argument in ("APXM_BUILDER_IMAGE", "APXM_NODE_IMAGE", "APXM_BASE_IMAGE"):
                    self.assertIn(f"ARG {argument}=", text)
                self.assertIn("must be an immutable digest reference", text)

    def test_image_executable_path_is_the_path_the_release_manifest_publishes(self) -> None:
        manifest = json.loads(RELEASE_MANIFEST.read_text(encoding="utf-8"))
        published = {item["name"]: item["path"] for item in manifest["services"]}
        for service, spec in self.images.SERVICES.items():
            with self.subTest(service=service):
                self.assertEqual(published[service], spec["executable"])
                self.assertIn(
                    f'ENTRYPOINT ["/workspace/{spec["executable"]}"]',
                    self.dockerfiles[service],
                )

    def test_dekk_manifest_exposes_the_image_commands_and_no_package_architecture_gate(self) -> None:
        commands = self.manifest["commands"]
        self.assertEqual(
            commands["build-images"]["run"], "python tools/scripts/service_images.py build"
        )
        self.assertEqual(
            commands["verify-images"]["run"], "python tools/scripts/service_images.py verify"
        )
        # Consumers verify Linux service bytes by reading the image that built
        # them, not by inspecting ELF headers in an owner-produced package.
        self.assertNotIn("verify-linux-package", commands)


if __name__ == "__main__":
    unittest.main()
