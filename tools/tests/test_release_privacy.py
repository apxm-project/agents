"""Release packaging remains private and fails closed without signed endpoints."""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPTS_ROOT = Path(__file__).parents[1] / "scripts"
REPOSITORY_ROOT = SCRIPTS_ROOT.parents[1]
sys.path.insert(0, str(SCRIPTS_ROOT))

from apxm_release import privacy, publish  # noqa: E402
from apxm_release.constants import (  # noqa: E402
    REGISTRY_MANIFEST_SCHEMA,
    REGISTRY_SIGNATURE_NAMESPACE,
)
from apxm_release.dist import release_artifacts  # noqa: E402


class ReleasePrivacyTests(unittest.TestCase):
    def sign_registry_manifest(
        self,
        root: Path,
        repository_url: str,
    ) -> tuple[Path, Path, Path]:
        manifest = root / "registry.json"
        manifest.write_text(
            json.dumps(
                {
                    "schema_version": REGISTRY_MANIFEST_SCHEMA,
                    "ecosystem": "python",
                    "visibility": "private",
                    "repository_url": repository_url,
                    "package_names": ["apxm-frontend"],
                },
                sort_keys=True,
            )
            + "\n"
        )
        key = root / "release-controller"
        subprocess.run(
            ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(key)],
            check=True,
        )
        allowed_signers = root / "allowed_signers"
        allowed_signers.write_text(
            f"release-controller {key.with_suffix('.pub').read_text().strip()}\n"
        )
        subprocess.run(
            [
                "ssh-keygen",
                "-Y",
                "sign",
                "-f",
                str(key),
                "-n",
                REGISTRY_SIGNATURE_NAMESPACE,
                str(manifest),
            ],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        return manifest, manifest.with_suffix(".json.sig"), allowed_signers

    def test_all_tracked_package_and_release_surfaces_pass_privacy_audit(self) -> None:
        manifests = privacy._tracked_paths(
            "Cargo.toml",
            "**/Cargo.toml",
            "package.json",
            "**/package.json",
            "pyproject.toml",
            "**/pyproject.toml",
            "setup.py",
            "**/setup.py",
            "setup.cfg",
            "**/setup.cfg",
        )
        self.assertEqual(len(manifests), 29)
        self.assertEqual(privacy.audit_release_privacy(), ())

    def test_manifest_classification_matches_current_owner_boundaries(self) -> None:
        frontend = json.loads(
            (REPOSITORY_ROOT / "crates/compiler/frontend/typescript/package.json").read_text()
        )
        handwritten_client = json.loads(
            (REPOSITORY_ROOT / "crates/tools/client/typescript/package.json").read_text()
        )
        python = privacy.load_toml(
            REPOSITORY_ROOT / "crates/compiler/frontend/python/pyproject.toml"
        )

        self.assertEqual(frontend["name"], "@apxm/frontend")
        self.assertIs(frontend["private"], True)
        self.assertNotIn("publishConfig", frontend)
        self.assertIs(handwritten_client["private"], True)
        self.assertNotIn("publishConfig", handwritten_client)
        self.assertEqual(python["project"]["name"], "apxm")
        self.assertIs(python["tool"]["apxm"]["release"]["publish"], False)

    def test_manifest_auditors_reject_public_or_default_publication(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cargo = root / "Cargo.toml"
            cargo.write_text('[package]\nname = "unsafe"\nversion = "0.1.0"\n')
            npm = root / "package.json"
            npm.write_text(
                json.dumps(
                    {
                        "name": "@apxm/unsafe",
                        "publishConfig": {"access": "public"},
                    }
                )
            )
            pyproject = root / "pyproject.toml"
            pyproject.write_text(
                '[project]\nname = "unsafe"\nversion = "0.1.0"\n'
                '[tool.apxm.release]\npublish = true\n'
            )

            with patch.object(privacy, "REPO_ROOT", root):
                self.assertTrue(privacy._audit_cargo_manifest(cargo))
                self.assertTrue(privacy._audit_npm_manifest(npm))
                self.assertTrue(privacy._audit_python_manifest(pyproject))

    def test_restricted_scope_does_not_replace_exact_registry_authority(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            npm = root / "package.json"
            npm.write_text(
                json.dumps(
                    {
                        "name": "@apxm/frontend",
                        "publishConfig": {"access": "restricted"},
                    }
                )
            )

            with patch.object(privacy, "REPO_ROOT", root):
                self.assertEqual(
                    privacy._audit_npm_manifest(npm),
                    [
                        "package.json: private must be true until signed private npm "
                        "registry authority exists"
                    ],
                )

    @unittest.skipUnless(shutil.which("ssh-keygen"), "ssh-keygen is required")
    def test_signed_private_registry_manifest_is_verified(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, signature, allowed_signers = self.sign_registry_manifest(
                root,
                "https://packages.example.invalid/python/",
            )

            result = privacy.load_private_python_registry(
                manifest,
                signature,
                allowed_signers,
                "release-controller",
            )

        self.assertEqual(result.repository_url, "https://packages.example.invalid/python/")
        self.assertEqual(result.package_names, ("apxm-frontend",))
        self.assertEqual(len(result.manifest_sha256), 64)

    @unittest.skipUnless(shutil.which("ssh-keygen"), "ssh-keygen is required")
    def test_signed_public_registry_manifest_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, signature, allowed_signers = self.sign_registry_manifest(
                root,
                "https://upload.pypi.org/legacy/",
            )

            with self.assertRaisesRegex(ValueError, "public Python package indexes"):
                privacy.load_private_python_registry(
                    manifest,
                    signature,
                    allowed_signers,
                    "release-controller",
                )

    def test_current_python_distribution_cannot_be_published(self) -> None:
        registry = privacy.PrivatePythonRegistry(
            repository_url="https://packages.example.invalid/python/",
            package_names=("apxm",),
            signer="release-controller",
            manifest_sha256="0" * 64,
        )
        with self.assertRaisesRegex(ValueError, "marked non-publishable"):
            privacy.require_publishable_python_distribution(registry)

    def test_python_upload_always_uses_signed_exact_repository_url(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            python_dir = output / "python"
            python_dir.mkdir()
            artifact = python_dir / "apxm_frontend-0.1.0-py3-none-any.whl"
            artifact.write_bytes(b"wheel")
            registry = privacy.PrivatePythonRegistry(
                repository_url="https://packages.example.invalid/python/",
                package_names=("apxm-frontend",),
                signer="release-controller",
                manifest_sha256="1" * 64,
            )
            args = argparse.Namespace(
                output_dir=str(output),
                registry_manifest="manifest.json",
                registry_signature="manifest.json.sig",
                allowed_signers="allowed_signers",
                signer="release-controller",
                yes=True,
            )
            completed = subprocess.CompletedProcess([], 0)
            with patch.object(publish, "release_version", return_value="0.1.0"), patch.object(
                publish, "release_dir", return_value=output
            ), patch.object(
                publish, "load_private_python_registry", return_value=registry
            ), patch.object(
                publish,
                "require_publishable_python_distribution",
                return_value="apxm-frontend",
            ), patch.object(
                publish.shutil, "which", return_value="/usr/bin/twine"
            ), patch.object(
                publish, "run", return_value=completed
            ) as run:
                result = publish.publish_python(args)

        self.assertEqual(result, 0)
        self.assertEqual(
            run.call_args.args[0],
            [
                "/usr/bin/twine",
                "upload",
                "--repository-url",
                "https://packages.example.invalid/python/",
                str(artifact),
            ],
        )

    def test_github_release_refuses_nonprivate_repository(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            artifact = output / "apxm-0.1.0.tar.gz"
            artifact.write_bytes(b"archive")
            args = argparse.Namespace(
                skip_dist=True,
                output_dir=str(output),
                tag=None,
                update=False,
                draft=False,
                prerelease=False,
                yes=True,
            )
            existing = subprocess.CompletedProcess([], 1)
            with patch.object(publish.shutil, "which", return_value="/usr/bin/gh"), patch.object(
                publish, "release_version", return_value="0.1.0"
            ), patch.object(
                publish, "release_dir", return_value=output
            ), patch.object(
                publish, "release_artifacts", return_value=[artifact]
            ), patch.object(
                publish, "stdout", return_value="PUBLIC"
            ), patch.object(
                publish, "run", return_value=existing
            ) as run:
                with contextlib.redirect_stderr(io.StringIO()):
                    result = publish.publish_github(args)

        self.assertEqual(result, 2)
        self.assertEqual(run.call_count, 1)

    def test_stale_internal_python_artifacts_are_not_release_assets(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            (output / "python").mkdir()
            archive = output / "apxm-0.1.0-source.tar.gz"
            checksum = output / "SHA256SUMS"
            internal = output / "python" / "apxm-0.1.0.tar.gz"
            for path in (archive, checksum, internal):
                path.write_bytes(b"artifact")
            with patch("apxm_release.dist.python_publish_enabled", return_value=False):
                artifacts = release_artifacts(output)

        self.assertEqual(artifacts, [archive, checksum])


if __name__ == "__main__":
    unittest.main()
