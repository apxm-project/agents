#!/usr/bin/env python3
"""Unit tests for ``apxm_libs`` — the pack registry client.

Covers:

* ``_verify_install`` re-checks every skill's ``artifact_hash`` against
  the on-disk ``skill.apxmobj`` bytes (install-time tamper gate).
* ``_write_toml_field`` is the canonical writer for
  ``skill.toml.artifact_hash`` and ``pack.toml.pack_hash``.
* ``cmd_build`` resolves the per-pack opt level and refuses to write
  drift between artifact bytes and declared hash.

The compile path itself (which shells out to ``dekk apxm compile``) is
mocked — that integration is exercised by the Rust ``--embed-manifest``
test.
"""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

try:
    import tomllib
except ModuleNotFoundError:  # Python <3.11 fallback
    import tomli as tomllib  # type: ignore[no-redef]


SCRIPT_PATH = Path(__file__).with_name("apxm_libs.py")
SPEC = importlib.util.spec_from_file_location("apxm_libs", SCRIPT_PATH)
assert SPEC and SPEC.loader
libs = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = libs
SPEC.loader.exec_module(libs)


def _write_skill_pack(root: Path, *, artifact_bytes: bytes) -> tuple[Path, str]:
    """Build a tiny installed-style pack tree under ``root`` with one
    skill whose ``artifact_hash`` matches ``artifact_bytes``. Returns
    ``(pack_dir, declared_hash)``.
    """
    pack_dir = root / "demo-pack"
    skill_dir = pack_dir / "skills" / "demo-skill"
    skill_dir.mkdir(parents=True)
    apxmobj = skill_dir / "skill.apxmobj"
    apxmobj.write_bytes(artifact_bytes)
    declared = libs._tagged_blake3(artifact_bytes)
    (skill_dir / "skill.toml").write_text(
        '\n'.join(
            [
                'skill_id = "demo-skill"',
                'version = "0.1.0"',
                'entry_flow = "main"',
                f'artifact_hash = "{declared}"',
                "",
            ]
        )
    )
    return pack_dir, declared


@unittest.skipUnless(libs.HAVE_BLAKE3, "blake3 not installed")
class VerifySkillArtifactHashesTests(unittest.TestCase):
    def test_clean_install_passes(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            artifact_bytes = b"PRETEND_APXMOBJ_BYTES"
            pack_dir, _ = _write_skill_pack(root, artifact_bytes=artifact_bytes)
            # Must not raise.
            libs._verify_skill_artifact_hashes(pack_dir, "demo-pack")

    def test_tamper_one_byte_fails_clean(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            artifact_bytes = b"PRETEND_APXMOBJ_BYTES"
            pack_dir, _ = _write_skill_pack(root, artifact_bytes=artifact_bytes)
            apxmobj = pack_dir / "skills" / "demo-skill" / "skill.apxmobj"
            tampered = bytearray(apxmobj.read_bytes())
            tampered[0] ^= 0x01
            apxmobj.write_bytes(bytes(tampered))
            with self.assertRaises(SystemExit) as ctx:
                libs._verify_skill_artifact_hashes(pack_dir, "demo-pack")
            self.assertIn("artifact_hash mismatch", str(ctx.exception))
            self.assertIn("demo-pack", str(ctx.exception))
            self.assertIn("demo-skill", str(ctx.exception))

    def test_missing_artifact_fails_clean(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            artifact_bytes = b"X"
            pack_dir, _ = _write_skill_pack(root, artifact_bytes=artifact_bytes)
            (pack_dir / "skills" / "demo-skill" / "skill.apxmobj").unlink()
            with self.assertRaises(SystemExit) as ctx:
                libs._verify_skill_artifact_hashes(pack_dir, "demo-pack")
            self.assertIn("skill artifact missing", str(ctx.exception))


class WriteTomlFieldTests(unittest.TestCase):
    def test_writes_top_level_field(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "skill.toml"
            path.write_text('skill_id = "demo"\nversion = "0.1.0"\n')
            libs._write_toml_field(path, None, "artifact_hash", "blake3:abc")
            parsed = tomllib.loads(path.read_text())
            self.assertEqual(parsed["artifact_hash"], "blake3:abc")
            self.assertEqual(parsed["skill_id"], "demo")

    def test_overwrites_existing_field(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "skill.toml"
            path.write_text(
                'skill_id = "demo"\nartifact_hash = "blake3:old"\n'
            )
            libs._write_toml_field(path, None, "artifact_hash", "blake3:new")
            parsed = tomllib.loads(path.read_text())
            self.assertEqual(parsed["artifact_hash"], "blake3:new")

    def test_writes_nested_table_field(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "pack.toml"
            path.write_text(
                'pack_id = "demo-pack"\nversion = "0.1.0"\n'
            )
            libs._write_toml_field(path, "integrity", "pack_hash", "blake3:xyz")
            parsed = tomllib.loads(path.read_text())
            self.assertEqual(parsed["integrity"]["pack_hash"], "blake3:xyz")


class ParseOptLevelTests(unittest.TestCase):
    def test_o2_default(self) -> None:
        self.assertEqual(libs._parse_opt_level(None), 2)

    def test_named_levels(self) -> None:
        self.assertEqual(libs._parse_opt_level("O0"), 0)
        self.assertEqual(libs._parse_opt_level("O3"), 3)

    def test_numeric(self) -> None:
        self.assertEqual(libs._parse_opt_level("1"), 1)

    def test_rejects_garbage(self) -> None:
        with self.assertRaises(SystemExit):
            libs._parse_opt_level("turbo")


@unittest.skipUnless(libs.HAVE_BLAKE3, "blake3 not installed")
class CmdBuildTests(unittest.TestCase):
    def test_build_writes_canonical_hashes(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            pack_dir = root / "demo-pack"
            skill_dir = pack_dir / "skills" / "demo-skill"
            skill_dir.mkdir(parents=True)
            (skill_dir / "skill.air").write_text("; placeholder\n")
            (skill_dir / "skill.toml").write_text(
                '\n'.join(
                    [
                        'skill_id = "demo-skill"',
                        'version = "0.1.0"',
                        'entry_flow = "main"',
                        "",
                    ]
                )
            )
            (pack_dir / "pack.toml").write_text(
                '\n'.join(
                    [
                        'pack_id = "demo-pack"',
                        'version = "0.1.0"',
                        'skill = "demo-skill"',
                        "",
                        "[compile]",
                        'opt_level = "O2"',
                        "",
                    ]
                )
            )

            artifact_bytes = b"FAKE_O2_ARTIFACT_BYTES"

            def _fake_compile(skill_dir_arg, opt_level):
                self.assertEqual(opt_level, 2)
                apxmobj = skill_dir_arg / "skill.apxmobj"
                apxmobj.write_bytes(artifact_bytes)
                return skill_dir_arg / "skill.air", apxmobj

            args = mock.Mock()
            args.pack = "demo-pack"
            args.source = str(pack_dir)
            with mock.patch.object(libs, "_compile_skill_via_dekk", side_effect=_fake_compile):
                rc = libs.cmd_build(args)
            self.assertEqual(rc, 0)

            skill_parsed = tomllib.loads((skill_dir / "skill.toml").read_text())
            self.assertEqual(
                skill_parsed["artifact_hash"], libs._tagged_blake3(artifact_bytes)
            )
            pack_parsed = tomllib.loads((pack_dir / "pack.toml").read_text())
            self.assertIn("integrity", pack_parsed)
            self.assertIn("pack_hash", pack_parsed["integrity"])
            self.assertTrue(pack_parsed["integrity"]["pack_hash"].startswith("blake3:"))


if __name__ == "__main__":
    unittest.main()
