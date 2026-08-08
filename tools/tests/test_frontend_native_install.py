"""Verify native frontend artifacts install under stable package names."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
INSTALLER_PATH = REPOSITORY_ROOT / "tools" / "scripts" / "install_frontend_native.py"


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load module from {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FrontendNativeInstallTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.installer = load_module(INSTALLER_PATH, "frontend_native_installer")

    def test_macos_artifact_is_installed_as_stable_python_module_name(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            root = Path(temp_dir_name)
            release = root / "release"
            release.mkdir()
            source = release / "lib_native.dylib"
            source.write_bytes(b"native bridge")

            with mock.patch.object(self.installer, "REPO_ROOT", root):
                destination = self.installer.install_python(release)

            self.assertEqual(
                destination,
                root / "crates/compiler/frontend/python/apxm_program/_native.so",
            )
            self.assertEqual(destination.read_bytes(), b"native bridge")
