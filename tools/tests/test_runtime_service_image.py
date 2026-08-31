"""Verify the Runtime Service image's non-root durable-state contract."""

from __future__ import annotations

import json
from pathlib import Path
import unittest


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
DOCKERFILE = REPOSITORY_ROOT / "deploy" / "Dockerfile.runtime-service"
RUNTIME_STATE_DIR = "/var/lib/apxm/runtime-state"


class RuntimeServiceImageTests(unittest.TestCase):
    """Pin the image metadata required for fresh named volumes."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.dockerfile = DOCKERFILE.read_text(encoding="utf-8")

    def test_runtime_state_directory_is_initialized_for_service_user(self) -> None:
        setup = next(
            line
            for line in self.dockerfile.splitlines()
            if "mkdir -p" in line and RUNTIME_STATE_DIR in line
        )
        self.assertIn(RUNTIME_STATE_DIR, setup)
        self.assertIn(
            "chown -R 65532:65532 /run/apxm /var/lib/apxm/artifacts "
            f"{RUNTIME_STATE_DIR}",
            self.dockerfile,
        )
        self.assertLess(
            self.dockerfile.index("mkdir -p"),
            self.dockerfile.index("VOLUME ["),
            "the image must seed ownership before Docker initializes a named volume",
        )

    def test_runtime_state_volume_and_default_are_declared(self) -> None:
        self.assertIn(
            f"APXM_RUNTIME_STATE_DIR={RUNTIME_STATE_DIR}",
            self.dockerfile,
        )
        volume_line = next(
            line for line in self.dockerfile.splitlines() if line.startswith("VOLUME ")
        )
        volumes = json.loads(volume_line.removeprefix("VOLUME "))
        self.assertIn(RUNTIME_STATE_DIR, volumes)
        self.assertEqual(volumes, ["/run/apxm", "/var/lib/apxm/artifacts", RUNTIME_STATE_DIR])

    def test_runtime_image_stays_non_root(self) -> None:
        self.assertIn("USER 65532:65532", self.dockerfile)


if __name__ == "__main__":
    unittest.main()
