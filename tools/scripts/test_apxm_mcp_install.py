#!/usr/bin/env python3
"""Unit tests for APXM MCP agent config writer helpers."""

from __future__ import annotations

import argparse
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock


SCRIPT_PATH = Path(__file__).with_name("apxm_mcp_install.py")
SPEC = importlib.util.spec_from_file_location("apxm_mcp_install", SCRIPT_PATH)
assert SPEC and SPEC.loader
installer = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = installer
SPEC.loader.exec_module(installer)


class ApxmMcpInstallTests(unittest.TestCase):
    def test_claude_json_install_and_uninstall_preserve_other_servers(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "settings.json"
            path.write_text(
                json.dumps(
                    {
                        installer.JsonKey.MCP_SERVERS.value: {
                            "existing": {
                                installer.JsonKey.COMMAND.value: "existing-server",
                                installer.JsonKey.ARGS.value: [],
                            }
                        }
                    }
                ),
                encoding="utf-8",
            )
            config = installer.AgentConfig(
                agent=installer.Agent.CLAUDE,
                path=path,
                detected=True,
                reason="test",
            )

            installer.install_json_config(
                config,
                installer.DEFAULT_SERVER_COMMAND,
                dry_run=False,
            )
            data = json.loads(path.read_text(encoding="utf-8"))
            servers = data[installer.JsonKey.MCP_SERVERS.value]
            self.assertIn("existing", servers)
            self.assertEqual(
                servers[installer.JsonKey.APXM.value][installer.JsonKey.COMMAND.value],
                installer.DEFAULT_SERVER_COMMAND,
            )
            self.assertEqual(
                servers[installer.JsonKey.APXM.value][installer.JsonKey.TYPE.value],
                installer.JsonKey.STDIO.value,
            )

            installer.uninstall_json_config(config, dry_run=False)
            data = json.loads(path.read_text(encoding="utf-8"))
            servers = data[installer.JsonKey.MCP_SERVERS.value]
            self.assertIn("existing", servers)
            self.assertNotIn(installer.JsonKey.APXM.value, servers)

    def test_codex_toml_install_replaces_only_apxm_section(self) -> None:
        existing = "\n".join(
            [
                "[profile.default]",
                'model = "gpt-test"',
                "",
                f"[{installer.TomlKey.MCP_SERVERS_APXM.value}]",
                'command = "old-apxm"',
                "args = []",
                "",
                "[mcp_servers.other]",
                'command = "other"',
                "",
            ]
        )

        replaced = installer.replace_toml_section(
            existing,
            installer.codex_section(installer.DEFAULT_SERVER_COMMAND),
        )

        self.assertIn("[profile.default]", replaced)
        self.assertIn("[mcp_servers.other]", replaced)
        self.assertIn(f'command = "{installer.DEFAULT_SERVER_COMMAND}"', replaced)
        self.assertNotIn('command = "old-apxm"', replaced)
        self.assertEqual(replaced.count(f"[{installer.TomlKey.MCP_SERVERS_APXM.value}]"), 1)

    def test_parse_agent_list_rejects_unknown_agent(self) -> None:
        self.assertEqual(
            installer.parse_agent_list("claude,codex"),
            [installer.Agent.CLAUDE, installer.Agent.CODEX],
        )
        with self.assertRaises(SystemExit):
            installer.parse_agent_list("unknown")


class GeminiInstallTests(unittest.TestCase):
    """Mock-based coverage for the Gemini CLI install code path.

    Gemini CLI is not present on the development machine; these tests fake
    its presence by creating a tempdir ~/.gemini and patching Path.home().
    """

    def setUp(self) -> None:
        self._tempdir = tempfile.TemporaryDirectory()
        self.home = Path(self._tempdir.name)
        (self.home / installer.GEMINI_DIR).mkdir()
        self.settings_path = self.home / installer.GEMINI_DIR / installer.GEMINI_SETTINGS
        # Patch Path.home() and shutil.which so neither the user's real $HOME
        # nor a real gemini binary on PATH leaks into the test.
        self._home_patch = mock.patch.object(installer.Path, "home", return_value=self.home)
        self._which_patch = mock.patch.object(installer.shutil, "which", return_value=None)
        self._home_patch.start()
        self._which_patch.start()

    def tearDown(self) -> None:
        self._home_patch.stop()
        self._which_patch.stop()
        self._tempdir.cleanup()

    def _gemini_args(self, command: str, *, dry_run: bool = False) -> argparse.Namespace:
        return argparse.Namespace(
            command=command,
            auto_detect=False,
            only="gemini",
            dry_run=dry_run,
            server_command="apxm-mcp-server",
        )

    def test_gemini_install_writes_settings_json_with_apxm_mcp_server(self) -> None:
        rc = installer.command_install(self._gemini_args(installer.Command.INSTALL.value))
        self.assertEqual(rc, 0)
        self.assertTrue(self.settings_path.exists())
        data = json.loads(self.settings_path.read_text(encoding="utf-8"))
        servers = data[installer.JsonKey.MCP_SERVERS.value]
        apxm_entry = servers[installer.JsonKey.APXM.value]
        self.assertEqual(apxm_entry[installer.JsonKey.COMMAND.value], "apxm-mcp-server")
        self.assertEqual(apxm_entry[installer.JsonKey.ARGS.value], [])
        # Gemini settings.json must NOT include the Claude-only "type": "stdio" field.
        self.assertNotIn(installer.JsonKey.TYPE.value, apxm_entry)

    def test_gemini_install_merges_with_existing_settings_json(self) -> None:
        pre_existing = {
            "theme": "dark",
            installer.JsonKey.MCP_SERVERS.value: {
                "other-server": {
                    installer.JsonKey.COMMAND.value: "other-cmd",
                    installer.JsonKey.ARGS.value: ["--flag"],
                }
            },
        }
        self.settings_path.write_text(json.dumps(pre_existing), encoding="utf-8")

        rc = installer.command_install(self._gemini_args(installer.Command.INSTALL.value))
        self.assertEqual(rc, 0)
        data = json.loads(self.settings_path.read_text(encoding="utf-8"))
        # Unrelated top-level key preserved.
        self.assertEqual(data["theme"], "dark")
        servers = data[installer.JsonKey.MCP_SERVERS.value]
        # Pre-existing server preserved verbatim.
        self.assertEqual(
            servers["other-server"][installer.JsonKey.COMMAND.value], "other-cmd"
        )
        self.assertEqual(servers["other-server"][installer.JsonKey.ARGS.value], ["--flag"])
        # APXM entry added alongside.
        self.assertIn(installer.JsonKey.APXM.value, servers)
        self.assertEqual(
            servers[installer.JsonKey.APXM.value][installer.JsonKey.COMMAND.value],
            "apxm-mcp-server",
        )

    def test_gemini_install_uninstall_round_trip(self) -> None:
        pre_existing = {
            installer.JsonKey.MCP_SERVERS.value: {
                "keep-me": {
                    installer.JsonKey.COMMAND.value: "keep-cmd",
                    installer.JsonKey.ARGS.value: [],
                }
            }
        }
        self.settings_path.write_text(json.dumps(pre_existing), encoding="utf-8")

        install_rc = installer.command_install(
            self._gemini_args(installer.Command.INSTALL.value)
        )
        self.assertEqual(install_rc, 0)
        data = json.loads(self.settings_path.read_text(encoding="utf-8"))
        self.assertIn(
            installer.JsonKey.APXM.value, data[installer.JsonKey.MCP_SERVERS.value]
        )

        uninstall_rc = installer.command_uninstall(
            self._gemini_args(installer.Command.UNINSTALL.value)
        )
        self.assertEqual(uninstall_rc, 0)
        data = json.loads(self.settings_path.read_text(encoding="utf-8"))
        servers = data[installer.JsonKey.MCP_SERVERS.value]
        self.assertNotIn(installer.JsonKey.APXM.value, servers)
        # Unrelated server still present.
        self.assertIn("keep-me", servers)
        self.assertEqual(servers["keep-me"][installer.JsonKey.COMMAND.value], "keep-cmd")

    def test_gemini_install_dry_run_does_not_touch_filesystem(self) -> None:
        # No pre-existing settings.json; --dry-run must not create one.
        self.assertFalse(self.settings_path.exists())
        buffer = io.StringIO()
        with redirect_stdout(buffer):
            rc = installer.command_install(
                self._gemini_args(installer.Command.INSTALL.value, dry_run=True)
            )
        self.assertEqual(rc, 0)
        self.assertFalse(
            self.settings_path.exists(),
            "dry-run must not create settings.json",
        )
        output = buffer.getvalue()
        self.assertIn("DRY-RUN", output)
        self.assertIn(str(self.settings_path), output)


if __name__ == "__main__":
    unittest.main()
