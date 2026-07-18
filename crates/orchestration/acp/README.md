# apxm-acp prototype migration baseline

> **Status: pre-canonical implementation evidence.** This crate is not the
> target ACP contract. Its handwritten wire schema, process/router integration,
> AAM prompt injection, local permission modes, ambient credentials, mutable
> commands and package ranges must be deleted or rebuilt by the
> [ACP full-replacement plan](../../../docs/agents/acp-and-routing-full-replacement-plan.md).
> The normative contract is
> [ACP interoperability and selection](../../../docs/agents/acp-and-routing-contract.md).

Agent Client Protocol (ACP) client for spawning and communicating with coding agents over JSON-RPC 2.0 stdio.

## Overview

The current crate spawns processes over JSON-RPC stdio and integrates them with
prototype AAM/process semantics. Its built-in names and generic custom-command
path are not support claims. Canonical support will mean an exact signed ACP
adapter/profile that passes the Compatibility Set's protocol, supply-chain,
confinement, authority, lifecycle and evidence matrix.

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

## Prototype built-in templates

- `claude`, `codex`, and `gemini` record prototype commands only.
- `custom` and user-defined profiles remain available only as explicit
  migration, development, and configuration inputs. They are not production
  support claims and cannot be promoted implicitly.

The target first proves exact immutable `claude-agent-acp` and `codex-acp`
profiles. An adapter may use a vendor SDK or App Server internally, but APXM
core depends only on its ACP Client Port and the admitted profile contract.
Another agent is supported only after the same exact conformance and admission
process; APXM never treats an arbitrary local command as supported ACP.

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | Shared types and error definitions |
| apxm-runtime | AAM state, capability system |
| apxm-backend-registry | Backend registry and API-key reference lookup |
