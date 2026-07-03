# apxm-acp

Agent Client Protocol (ACP) client for spawning and communicating with coding agents over JSON-RPC 2.0 stdio.

## Overview

`apxm-acp` spawns coding agents as subprocesses, speaks JSON-RPC 2.0 over stdin/stdout, and integrates agent sessions with the APXM runtime's AAM memory and capability systems. Three CLIs (Claude, Codex, Gemini) are a tuned, verified working set with dedicated templates; any other ACP-speaking CLI (Copilot, Cursor, and the like) is added through configuration via the generic custom-command template rather than a hardcoded Rust variant.

## Module Structure

| Module | Description |
|--------|-------------|
| `protocol` | JSON-RPC 2.0 message framing and serialization |
| `session` | `AcpSession` lifecycle (spawn, send, receive, close) |
| `registry` | `AgentRegistry`: 3 tuned working-set templates + 1 generic custom-command template |
| `auth` | Host-local env credential lookup for ACP profiles; not durable credential custody |
| `content` | Structured content types (text, tool results) |
| `controls` | Permission modes and approval policies |
| `events` | Session-level event types |
| `reverse` | Reverse-request handling (file read/write, bash) |
| `terminal` | Terminal output capture from agent subprocesses |
| `aam_bridge` | Bridge between ACP sessions and AAM state |
| `constants` | Timeout values, permission mode strings |

## Key Exports

- `AcpSession` -- single agent session over JSON-RPC stdio
- `AgentRegistry` -- discovers, registers, and manages agent profiles
- `AcpAgentProfile` -- spawn command, route metadata, timeouts, permission mode for one agent
- `PermissionMode` -- `ApproveAll`, `ApproveReads`, `DenyAll`
- `CapabilityServerConfig` -- MCP server config provisioned to agent sessions
- `AcpError` -- error type covering spawn, protocol, timeout, and permission failures

## Built-in Agent Templates

- **Working set (tuned):** `claude`, `codex`, `gemini` — verified commands, descriptions, and timeouts.
- **`custom`:** generic custom-command template. Ships with an empty `command` (never a live route candidate) and documents how to add any other ACP-speaking CLI — e.g. copilot, pi, cursor, droid, kilocode, kimi, kiro, opencode, qoder, qwen, trae, iflow — via `apxm agent add <name> --command "..."` or a `~/.apxm/agents.toml` entry, with no dedicated Rust variant required.

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | Shared types and error definitions |
| apxm-runtime | AAM state, capability system |
| apxm-backend-registry | Backend registry and API-key reference lookup |
