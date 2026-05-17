#!/usr/bin/env python3
"""Unit tests for APXM MCP agent config writer helpers."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


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


if __name__ == "__main__":
    unittest.main()
