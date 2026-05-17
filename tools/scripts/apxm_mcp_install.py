#!/usr/bin/env python3
"""Install the APXM stdio MCP server into detected coding-agent configs."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Any


class Agent(str, Enum):
    CLAUDE = "claude"
    CODEX = "codex"
    GEMINI = "gemini"
    CURSOR = "cursor"


class JsonKey(str, Enum):
    MCP_SERVERS = "mcpServers"
    APXM = "apxm"
    TYPE = "type"
    STDIO = "stdio"
    COMMAND = "command"
    ARGS = "args"


class TomlKey(str, Enum):
    MCP_SERVERS_APXM = "mcp_servers.apxm"
    COMMAND = "command"
    ARGS = "args"
    TOOL_TIMEOUT_SEC = "tool_timeout_sec"
    ENABLED = "enabled"
    REQUIRED = "required"


class Command(str, Enum):
    INSTALL = "install"
    LIST = "list"
    UNINSTALL = "uninstall"


DEFAULT_SERVER_COMMAND = "apxm-mcp-server"
DEFAULT_TOOL_TIMEOUT_SEC = 600
CLAUDE_PROJECT_MCP = ".mcp.json"
CLAUDE_DIR = ".claude"
CODEX_DIR = ".codex"
GEMINI_DIR = ".gemini"
CURSOR_DIR = ".cursor"
CODEX_CONFIG = "config.toml"
GEMINI_SETTINGS = "settings.json"
CURSOR_MCP = "mcp.json"


@dataclass(frozen=True)
class AgentConfig:
    agent: Agent
    path: Path
    detected: bool
    reason: str


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def home_dir() -> Path:
    return Path.home()


def default_server_command() -> str:
    env_command = os.environ.get("APXM_MCP_SERVER_COMMAND")
    if env_command:
        return env_command
    if found := shutil.which(DEFAULT_SERVER_COMMAND):
        return str(Path(found).resolve())
    root = repo_root()
    for profile in ("release", "debug"):
        candidate = root / "target" / profile / DEFAULT_SERVER_COMMAND
        if candidate.exists():
            return str(candidate)
    return DEFAULT_SERVER_COMMAND


def detect_agents() -> dict[Agent, AgentConfig]:
    home = home_dir()
    root = repo_root()
    claude_project = root / CLAUDE_PROJECT_MCP
    cursor_project = root / CURSOR_DIR / CURSOR_MCP
    cursor_home = home / CURSOR_DIR / CURSOR_MCP
    cursor_path = cursor_project if (root / CURSOR_DIR).exists() else cursor_home

    candidates = {
        Agent.CLAUDE: (
            claude_project,
            (home / CLAUDE_DIR).exists() or shutil.which(Agent.CLAUDE.value),
            "found ~/.claude or claude on PATH; using project .mcp.json",
        ),
        Agent.CODEX: (
            home / CODEX_DIR / CODEX_CONFIG,
            (home / CODEX_DIR).exists() or shutil.which(Agent.CODEX.value),
            "found ~/.codex or codex on PATH",
        ),
        Agent.GEMINI: (
            home / GEMINI_DIR / GEMINI_SETTINGS,
            (home / GEMINI_DIR).exists() or shutil.which(Agent.GEMINI.value),
            "found ~/.gemini or gemini on PATH",
        ),
        Agent.CURSOR: (
            cursor_path,
            (root / CURSOR_DIR).exists()
            or (home / CURSOR_DIR).exists()
            or shutil.which(Agent.CURSOR.value),
            "found .cursor, ~/.cursor, or cursor on PATH",
        ),
    }
    return {
        agent: AgentConfig(agent=agent, path=path, detected=bool(detected), reason=reason)
        for agent, (path, detected, reason) in candidates.items()
    }


def selected_agents(args: argparse.Namespace) -> list[AgentConfig]:
    configs = detect_agents()
    if args.only:
        requested = parse_agent_list(args.only)
        return [configs[agent] for agent in requested]
    if args.auto_detect:
        return [config for config in configs.values() if config.detected]
    return list(configs.values())


def parse_agent_list(value: str) -> list[Agent]:
    agents: list[Agent] = []
    for raw in value.split(","):
        name = raw.strip().lower()
        if not name:
            continue
        try:
            agents.append(Agent(name))
        except ValueError as exc:
            valid = ", ".join(agent.value for agent in Agent)
            raise SystemExit(f"unknown agent '{name}'. Valid agents: {valid}") from exc
    return agents


def read_json(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise SystemExit(f"invalid JSON in {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise SystemExit(f"{path} must contain a JSON object")
    return value


def write_json(path: Path, data: dict[str, Any], *, dry_run: bool) -> None:
    text = json.dumps(data, indent=2, sort_keys=True) + "\n"
    write_text(path, text, dry_run=dry_run)


def write_text(path: Path, text: str, *, dry_run: bool) -> None:
    if dry_run:
        print(f"DRY-RUN write {path}")
        print(text.rstrip())
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def json_server_payload(server_command: str, *, include_type: bool) -> dict[str, Any]:
    payload: dict[str, Any] = {
        JsonKey.COMMAND.value: server_command,
        JsonKey.ARGS.value: [],
    }
    if include_type:
        payload[JsonKey.TYPE.value] = JsonKey.STDIO.value
    return payload


def install_json_config(config: AgentConfig, server_command: str, *, dry_run: bool) -> None:
    data = read_json(config.path)
    servers = data.setdefault(JsonKey.MCP_SERVERS.value, {})
    if not isinstance(servers, dict):
        raise SystemExit(f"{config.path}: {JsonKey.MCP_SERVERS.value} must be an object")
    servers[JsonKey.APXM.value] = json_server_payload(
        server_command,
        include_type=config.agent == Agent.CLAUDE,
    )
    write_json(config.path, data, dry_run=dry_run)


def uninstall_json_config(config: AgentConfig, *, dry_run: bool) -> None:
    data = read_json(config.path)
    servers = data.get(JsonKey.MCP_SERVERS.value)
    if isinstance(servers, dict):
        servers.pop(JsonKey.APXM.value, None)
    write_json(config.path, data, dry_run=dry_run)


def install_codex_config(config: AgentConfig, server_command: str, *, dry_run: bool) -> None:
    existing = config.path.read_text(encoding="utf-8") if config.path.exists() else ""
    text = replace_toml_section(existing, codex_section(server_command))
    write_text(config.path, text, dry_run=dry_run)


def uninstall_codex_config(config: AgentConfig, *, dry_run: bool) -> None:
    existing = config.path.read_text(encoding="utf-8") if config.path.exists() else ""
    text = replace_toml_section(existing, None)
    write_text(config.path, text, dry_run=dry_run)


def codex_section(server_command: str) -> str:
    return "\n".join(
        [
            f"[{TomlKey.MCP_SERVERS_APXM.value}]",
            f'{TomlKey.COMMAND.value} = "{escape_toml_string(server_command)}"',
            f"{TomlKey.ARGS.value} = []",
            f"{TomlKey.TOOL_TIMEOUT_SEC.value} = {DEFAULT_TOOL_TIMEOUT_SEC}",
            f"{TomlKey.ENABLED.value} = true",
            f"{TomlKey.REQUIRED.value} = false",
            "",
        ]
    )


def replace_toml_section(existing: str, replacement: str | None) -> str:
    header = f"[{TomlKey.MCP_SERVERS_APXM.value}]"
    lines = existing.splitlines()
    output: list[str] = []
    index = 0
    removed = False
    while index < len(lines):
        line = lines[index]
        if line.strip() == header:
            removed = True
            index += 1
            while index < len(lines):
                stripped = lines[index].strip()
                if stripped.startswith("[") and stripped.endswith("]"):
                    break
                index += 1
            continue
        output.append(line)
        index += 1

    text = "\n".join(output).rstrip()
    if replacement is None:
        return text + ("\n" if text else "")
    prefix = text + "\n\n" if text else ""
    suffix = "" if removed else ""
    return prefix + replacement + suffix


def escape_toml_string(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def install_agent(config: AgentConfig, server_command: str, *, dry_run: bool) -> None:
    if config.agent == Agent.CODEX:
        install_codex_config(config, server_command, dry_run=dry_run)
    else:
        install_json_config(config, server_command, dry_run=dry_run)
    print(f"installed {config.agent.value}: {config.path}")


def uninstall_agent(config: AgentConfig, *, dry_run: bool) -> None:
    if config.agent == Agent.CODEX:
        uninstall_codex_config(config, dry_run=dry_run)
    else:
        uninstall_json_config(config, dry_run=dry_run)
    print(f"uninstalled {config.agent.value}: {config.path}")


def list_agents(args: argparse.Namespace) -> int:
    configs = selected_agents(args)
    if not configs:
        print("No agents selected.")
        return 0
    for config in configs:
        state = "detected" if config.detected else "not-detected"
        installed = is_installed(config)
        print(
            f"{config.agent.value}: {state}, installed={str(installed).lower()}, "
            f"path={config.path}, reason={config.reason}"
        )
    return 0


def is_installed(config: AgentConfig) -> bool:
    if not config.path.exists():
        return False
    if config.agent == Agent.CODEX:
        return f"[{TomlKey.MCP_SERVERS_APXM.value}]" in config.path.read_text(
            encoding="utf-8"
        )
    data = read_json(config.path)
    servers = data.get(JsonKey.MCP_SERVERS.value)
    return isinstance(servers, dict) and JsonKey.APXM.value in servers


def command_install(args: argparse.Namespace) -> int:
    configs = selected_agents(args)
    if not configs:
        print("No matching agents detected. Use --only to force a specific target.")
        return 1
    for config in configs:
        install_agent(config, args.server_command, dry_run=args.dry_run)
    return 0


def command_uninstall(args: argparse.Namespace) -> int:
    configs = selected_agents(args)
    if not configs:
        print("No matching agents detected.")
        return 0
    for config in configs:
        uninstall_agent(config, dry_run=args.dry_run)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="dekk apxm mcp")
    subparsers = parser.add_subparsers(dest="command", required=True)
    for command in Command:
        sub = subparsers.add_parser(command.value)
        sub.add_argument(
            "--auto-detect",
            action="store_true",
            help="Target only detected agent installs.",
        )
        sub.add_argument(
            "--only",
            help="Comma-separated agents to target: claude,codex,gemini,cursor.",
        )
        sub.add_argument(
            "--dry-run",
            action="store_true",
            help="Print changes without writing files.",
        )
        sub.add_argument(
            "--server-command",
            default=default_server_command(),
            help=(
                "MCP server command to register. Defaults to apxm-mcp-server "
                "on PATH, otherwise an existing target/release or target/debug binary."
            ),
        )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.command == Command.INSTALL.value:
        return command_install(args)
    if args.command == Command.UNINSTALL.value:
        return command_uninstall(args)
    if args.command == Command.LIST.value:
        return list_agents(args)
    parser.error(f"unknown command: {args.command}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
