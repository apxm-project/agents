"""Verify native frontend artifacts install under stable package names."""

from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import importlib.util
import stat
import subprocess
import tempfile
import threading
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

    def test_non_macho_artifact_is_installed_without_signing(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir_name:
            root = Path(temp_dir_name)
            release = root / "release"
            release.mkdir()
            source = release / "lib_native.so"
            source.write_bytes(b"not a Mach-O bridge")

            with (
                mock.patch.object(self.installer, "REPO_ROOT", root),
                mock.patch.object(self.installer.sys, "platform", "darwin"),
                mock.patch.object(self.installer.subprocess, "run") as run,
            ):
                destination = self.installer.install_python(release)

            self.assertEqual(
                destination,
                root / "crates/compiler/frontend/python/apxm_program/_native.so",
            )
            self.assertEqual(destination.read_bytes(), b"not a Mach-O bridge")
            self.assertEqual(stat.S_IMODE(destination.stat().st_mode), 0o755)
            run.assert_not_called()

    def test_macos_macho_artifact_signs_temporary_before_atomic_replace(self) -> None:
        if self.installer.sys.platform != "darwin":
            self.skipTest("requires a macOS host")
        source_path = REPOSITORY_ROOT / "target/release/lib_native.dylib"
        if not source_path.is_file() or not self.installer._is_macho(source_path):
            self.skipTest("requires the built macOS Mach-O frontend bridge")
        with tempfile.TemporaryDirectory() as temp_dir_name:
            root = Path(temp_dir_name)
            release = root / "release"
            release.mkdir()
            source = release / "lib_native.dylib"
            source.write_bytes(source_path.read_bytes())
            signed_paths: list[Path] = []

            def sign(command: list[str], **kwargs: object) -> None:
                signed_paths.append(Path(command[-1]))
                self.assertTrue(self.installer._is_macho(Path(command[-1])))
                self.assertEqual(stat.S_IMODE(Path(command[-1]).stat().st_mode), 0o755)

            with (
                mock.patch.object(self.installer, "REPO_ROOT", root),
                mock.patch.object(self.installer.sys, "platform", "darwin"),
                mock.patch.object(self.installer.subprocess, "run", side_effect=sign) as run,
            ):
                destination = self.installer.install_python(release)

            self.assertEqual(destination.read_bytes(), source_path.read_bytes())
            self.assertEqual(stat.S_IMODE(destination.stat().st_mode), 0o755)
            run.assert_called_once()
            self.assertEqual(len(signed_paths), 1)
            self.assertNotEqual(signed_paths[0], destination)
            self.assertFalse(signed_paths[0].exists())

    def test_macos_installs_and_verifies_a_real_macho_bridge(self) -> None:
        if self.installer.sys.platform != "darwin":
            self.skipTest("requires a macOS host")
        source_path = REPOSITORY_ROOT / "target/release/lib_native.dylib"
        if not source_path.is_file() or not self.installer._is_macho(source_path):
            self.skipTest("requires the built macOS Mach-O frontend bridge")
        with tempfile.TemporaryDirectory() as temp_dir_name:
            root = Path(temp_dir_name)
            release = root / "release"
            release.mkdir()
            (release / "lib_native.dylib").write_bytes(source_path.read_bytes())

            with mock.patch.object(self.installer, "REPO_ROOT", root):
                destination = self.installer.install_python(release)

            verified = subprocess.run(
                ["codesign", "--verify", "--strict", str(destination)],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(verified.returncode, 0, verified.stderr)
            self.assertTrue(self.installer._is_macho(destination))

    def test_macos_signing_failure_preserves_stderr_and_existing_destination(self) -> None:
        if self.installer.sys.platform != "darwin":
            self.skipTest("requires a macOS host")
        source_path = REPOSITORY_ROOT / "target/release/lib_native.dylib"
        if not source_path.is_file() or not self.installer._is_macho(source_path):
            self.skipTest("requires the built macOS Mach-O frontend bridge")
        with tempfile.TemporaryDirectory() as temp_dir_name:
            root = Path(temp_dir_name)
            release = root / "release"
            release.mkdir()
            (release / "lib_native.dylib").write_bytes(source_path.read_bytes())
            destination = root / "crates/compiler/frontend/python/apxm_program/_native.so"
            destination.parent.mkdir(parents=True)
            destination.write_bytes(b"previous bridge")
            failure = subprocess.CalledProcessError(
                1,
                ["codesign"],
                stderr="object file format unrecognized, invalid, or unsuitable",
            )

            with (
                mock.patch.object(self.installer, "REPO_ROOT", root),
                mock.patch.object(self.installer.sys, "platform", "darwin"),
                mock.patch.object(self.installer.subprocess, "run", side_effect=failure),
            ):
                with self.assertRaisesRegex(SystemExit, "object file format unrecognized"):
                    self.installer.install_python(release)

            self.assertEqual(destination.read_bytes(), b"previous bridge")

    def test_concurrent_macos_installs_sign_distinct_temporary_files(self) -> None:
        if self.installer.sys.platform != "darwin":
            self.skipTest("requires a macOS host")
        source_path = REPOSITORY_ROOT / "target/release/lib_native.dylib"
        if not source_path.is_file() or not self.installer._is_macho(source_path):
            self.skipTest("requires the built macOS Mach-O frontend bridge")
        with tempfile.TemporaryDirectory() as temp_dir_name:
            root = Path(temp_dir_name)
            release = root / "release"
            release.mkdir()
            (release / "lib_native.dylib").write_bytes(source_path.read_bytes())
            barrier = threading.Barrier(8)
            signed_paths: list[Path] = []
            signed_paths_lock = threading.Lock()

            def sign(command: list[str], **kwargs: object) -> None:
                path = Path(command[-1])
                barrier.wait(timeout=5)
                with signed_paths_lock:
                    signed_paths.append(path)
                self.assertTrue(self.installer._is_macho(path))

            with (
                mock.patch.object(self.installer, "REPO_ROOT", root),
                mock.patch.object(self.installer.sys, "platform", "darwin"),
                mock.patch.object(self.installer.subprocess, "run", side_effect=sign),
            ):
                with ThreadPoolExecutor(max_workers=8) as executor:
                    destinations = list(
                        executor.map(
                            lambda _: self.installer.install_python(release), range(8)
                        )
                    )

            self.assertEqual(len({path for path in destinations}), 1)
            self.assertEqual(len(set(signed_paths)), 8)
