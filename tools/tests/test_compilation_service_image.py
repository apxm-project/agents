"""Verify the Compilation Service image bundles its Python native frontend."""

from __future__ import annotations

from pathlib import Path
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
DOCKERFILE = REPOSITORY_ROOT / "deploy" / "Dockerfile.compilation-service"
RELEASE_NATIVE_ARTIFACT = "target/release/lib_native.so"
PACKAGE_NATIVE_PATH = "/workspace/crates/compiler/frontend/python/apxm_program/_native.so"


class CompilationServiceImageTests(unittest.TestCase):
    """Pin the native frontend handoff from release packaging into the image."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.dockerfile = DOCKERFILE.read_text(encoding="utf-8")

    def test_release_native_artifact_is_copied_to_probe_path(self) -> None:
        copy_line = (
            f"COPY {RELEASE_NATIVE_ARTIFACT} "
            f"{PACKAGE_NATIVE_PATH}"
        )
        self.assertIn(copy_line, self.dockerfile)

    def test_native_artifact_is_required_and_non_empty(self) -> None:
        self.assertIn(
            f"test -s {PACKAGE_NATIVE_PATH}",
            self.dockerfile,
            "the image must fail closed when the bundled native module is absent",
        )
        self.assertLess(
            self.dockerfile.index(f"COPY {RELEASE_NATIVE_ARTIFACT}"),
            self.dockerfile.index(f"test -s {PACKAGE_NATIVE_PATH}"),
        )

    def test_native_copy_follows_python_frontend_root(self) -> None:
        root_copy = "COPY crates/compiler/frontend/python /workspace/crates/compiler/frontend/python"
        native_copy = f"COPY {RELEASE_NATIVE_ARTIFACT} {PACKAGE_NATIVE_PATH}"
        self.assertLess(
            self.dockerfile.index(root_copy),
            self.dockerfile.index(native_copy),
            "the package root must be present before the stable native module is copied",
        )

    def test_image_remains_non_root(self) -> None:
        self.assertIn("USER 65532:65532", self.dockerfile)


if __name__ == "__main__":
    unittest.main()
