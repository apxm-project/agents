"""Reference-host build receipt stays exact and fail-closed."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPOSITORY_ROOT / "tools" / "scripts" / "reference_host_receipt.py"
VALIDATOR_PATH = REPOSITORY_ROOT / "contracts" / "tools" / "validate_owner_descriptor.py"


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReferenceHostBuildReceiptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = load_module(SCRIPT_PATH, "reference_host_receipt")
        cls.validator = load_module(VALIDATOR_PATH, "validate_owner_descriptor")

    def test_built_receipt_pins_exact_host_sdk_provenance(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            temp_dir = Path(temp_dir_name)
            source_path = temp_dir / "reference_host.rs"
            source_path.write_text("fn main() {}\n", encoding="utf-8")
            target_dir = temp_dir / "target"
            output_path = target_dir / "release" / "apxm-reference-host"
            output_path.parent.mkdir(parents=True)
            output_path.write_bytes(b"apxm-reference-host")
            receipt_path = temp_dir / "receipt.json"

            exit_code, receipt = self.module.write_reference_host_receipt(
                receipt_path=receipt_path,
                source_path=source_path,
                target_dir=target_dir,
                output_path=output_path,
                run_build=lambda _command: 0,
            )

        self.assertEqual(exit_code, 0)
        self.assertEqual(receipt["status"], "built")
        self.assertEqual(
            receipt["referenced_host_sdk"],
            {
                "name": self.validator.REFERENCE_HOST_DEPENDENCY_NAME,
                "git": self.validator.REFERENCE_HOST_DEPENDENCY_GIT,
                "source_revision": self.validator.REFERENCE_HOST_DESCRIPTOR["source_revision"],
                "descriptor_semantic_digest": self.validator.REFERENCE_HOST_DESCRIPTOR[
                    "descriptor_semantic_digest"
                ],
                "descriptor_exact_checksum": self.validator.REFERENCE_HOST_DESCRIPTOR[
                    "descriptor_exact_checksum"
                ],
            },
        )
        self.assertEqual(
            receipt["build"]["command"],
            self.module.BUILD_COMMAND,
        )

    def test_built_receipt_uses_exact_release_output_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            temp_dir = Path(temp_dir_name)
            source_path = temp_dir / "reference_host.rs"
            source_path.write_text("fn main() {}\n", encoding="utf-8")
            target_dir = temp_dir / "target"
            output_path = self.module.canonical_output_path(target_dir, "apxm-reference-host")
            output_path.parent.mkdir(parents=True)
            output_path.write_bytes(b"apxm-reference-host")
            receipt_path = temp_dir / "receipt.json"

            exit_code, receipt = self.module.write_reference_host_receipt(
                receipt_path=receipt_path,
                source_path=source_path,
                target_dir=target_dir,
                run_build=lambda _command: 0,
            )

        self.assertEqual(exit_code, 0)
        self.assertEqual(receipt["build"]["output_path"], str(output_path))
        self.assertEqual(receipt["output"]["path"], str(output_path))

    def test_missing_source_fails_closed_and_writes_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            temp_dir = Path(temp_dir_name)
            missing_source = temp_dir / "missing_reference_host.rs"
            target_dir = temp_dir / "target"
            receipt_path = temp_dir / "receipt.json"
            build_called = False

            def unexpected_build(_command: list[str]) -> int:
                nonlocal build_called
                build_called = True
                return 0

            exit_code, receipt = self.module.write_reference_host_receipt(
                receipt_path=receipt_path,
                source_path=missing_source,
                target_dir=target_dir,
                run_build=unexpected_build,
            )
            persisted = json.loads(receipt_path.read_text(encoding="utf-8"))

        self.assertEqual(exit_code, 1)
        self.assertFalse(build_called, "build must not run when the source file is absent")
        self.assertEqual(receipt["status"], "missing-source")
        self.assertEqual(receipt["error"]["code"], "missing_source")
        self.assertEqual(persisted, receipt)


if __name__ == "__main__":
    unittest.main()
