"""Reference-host startup-input generation stays exact and fail-closed."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPOSITORY_ROOT / "tools" / "scripts" / "reference_host_startup_input.py"


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def exact_digest(label: str) -> str:
    return "sha256:" + hashlib.sha256(label.encode("utf-8")).hexdigest()


class ReferenceHostStartupInputTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.module = load_module(SCRIPT_PATH, "reference_host_startup_input")

    def test_build_writes_exact_startup_input_for_clean_revision(self) -> None:
        original_git_stdout = self.module.git_stdout
        with tempfile.TemporaryDirectory() as temp_dir_name:
            temp_dir = Path(temp_dir_name)
            output_path = temp_dir / "startup-input.json"
            self.module.git_stdout = lambda *args: {
                ("rev-parse", "HEAD"): "1" * 40,
                ("status", "--porcelain", "--ignored=matching"): "",
            }[args]
            try:
                payload = self.module.build_startup_input(
                    owner_revision="1" * 40,
                    release_digest=exact_digest("release"),
                    port_bindings_digest=exact_digest("port-bindings"),
                    resource_ceiling_digest=exact_digest("resource-ceiling"),
                    output_path=output_path,
                )
            finally:
                self.module.git_stdout = original_git_stdout
            persisted = json.loads(output_path.read_text(encoding="utf-8"))

        self.assertEqual(payload, persisted)
        self.assertEqual(payload["schema_version"], self.module.STARTUP_INPUT_SCHEMA)
        self.assertEqual(payload["semantic_owner"], "agents")
        self.assertEqual(payload["owner_executable"], self.module.OWNER_EXECUTABLE)
        self.assertEqual(
            payload["reference_host_release_manifest"],
            {
                "path": str(self.module.RELEASE_MANIFEST_PATH.resolve()),
                "digest": self.module.file_digest(self.module.RELEASE_MANIFEST_PATH),
            },
        )
        self.assertEqual(
            payload["fail_closed_on"],
            self.module.FAIL_CLOSED_ON,
        )
        self.assertFalse(payload["provenance"]["dirty"])

    def test_rejects_mismatched_owner_revision(self) -> None:
        original_git_stdout = self.module.git_stdout
        self.module.git_stdout = lambda *args: {
            ("rev-parse", "HEAD"): "b" * 40,
            ("status", "--porcelain", "--ignored=matching"): "",
        }[args]
        try:
            with self.assertRaisesRegex(RuntimeError, "owner revision mismatch"):
                self.module.build_startup_input(
                    owner_revision="a" * 40,
                    release_digest=exact_digest("release"),
                    port_bindings_digest=exact_digest("port-bindings"),
                    resource_ceiling_digest=exact_digest("resource-ceiling"),
                )
        finally:
            self.module.git_stdout = original_git_stdout

    def test_rejects_dirty_owner_checkout(self) -> None:
        original_git_stdout = self.module.git_stdout
        self.module.git_stdout = lambda *args: {
            ("rev-parse", "HEAD"): "a" * 40,
            ("status", "--porcelain", "--ignored=matching"): " M tools/scripts/reference_host.rs",
        }[args]
        try:
            with self.assertRaisesRegex(RuntimeError, "owner checkout is dirty"):
                self.module.build_startup_input(
                    owner_revision="a" * 40,
                    release_digest=exact_digest("release"),
                    port_bindings_digest=exact_digest("port-bindings"),
                    resource_ceiling_digest=exact_digest("resource-ceiling"),
                )
        finally:
            self.module.git_stdout = original_git_stdout

    def test_rejects_placeholder_operator_digest(self) -> None:
        with self.assertRaisesRegex(ValueError, "release_digest must not be a placeholder digest"):
            self.module.build_startup_input(
                owner_revision="a" * 40,
                release_digest="sha256:" + ("0" * 64),
                port_bindings_digest=exact_digest("port-bindings"),
                resource_ceiling_digest=exact_digest("resource-ceiling"),
            )

    def test_parse_args_requires_explicit_values(self) -> None:
        with self.assertRaises(SystemExit):
            self.module.parse_args([])


if __name__ == "__main__":
    unittest.main()
