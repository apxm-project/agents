---
name: mcp-server
group: Domain
description: Use when working on APXM MCP surfaces: the Rust HTTP `/v1/mcp` endpoint, the Rust stdio `mcp-server` binary, or cross-agent MCP registration. Prefer server-owned HTTP MCP for workflow/orchestration control.
user-invocable: true
---

# APXM MCP Server

Load `_shared/apxm-development-rules.md` before broad work.

## What the MCP surfaces are

APXM has two Rust MCP surfaces:

- `apxm-server` exposes HTTP MCP at `POST /v1/mcp`. This is the
  server-owned control plane for long-running workflows, retained events,
  cancellation, server-owned sessions, skill inventory, and orchestration.
- `mcp-server` exposes stdio MCP for compile/query/debug tools. It does
  not own background workflow sessions or orchestration state.

MCP should stay a thin interface over APXM server/runtime capabilities.

## HTTP MCP Tools

- Compile/query: `validate`, `compile`, `ops_list`,
  `run`, `prompt_as_workflow`, `trace_fetch`, `aam_recall`,
  `evidence_lookup`, `capability_discovery`.
- Skills: `skills_list`, `skill_get`, `skill_validate`,
  `skill_call`.
- Workflow control: `workflow_start`, `workflow_status`,
  `workflow_events`, `workflow_cancel`.

## Stdio MCP Tools

The stdio binary exposes compile/query tools such as `validate`,
`compile`, `get_contract`, `analyze`, `prompt_as_workflow`,
`trace_fetch`, `aam_recall`, `evidence_lookup`, and
`capability_discovery`. Use HTTP MCP when a caller needs workflow start/status,
events, cancel, or orchestration.

## Registration

```bash
dekk agents mcp install     # registers with supported agent configs
                          # (Claude Code, Cursor, etc.)
```

The installation path is owned by `tools/scripts/apxm_mcp_install.py`.
Do not bypass it.

## Rules

- Keep MCP thin. Do not duplicate scheduling, worker admission, budget policy,
  trigger matching, or session ownership in MCP wrappers.
- Server-owned tools return APXM handles such as `execution_id`, `session_id`,
  `session_dir`, `workflow_path`, and retained event cursors. Do not invent
  shell process handles for server-managed runs.
- Secrets stay in the env — never accept `api_key` or
  `LLM_GATEWAY_KEY` as a tool argument.

## Diagnostics

- `dekk agents build-server` — compile the HTTP and stdio MCP binaries.
- `dekk agents mcp install` — register; surfaces config errors.

## Cross-surface registration (REST + MCP + A2A)

A new surface spans the handler, route/tool registration, the MCP manifest, the
Dekk wrapper, and a smoke test — miss one and it ships half-wired. A REST route
that should also be an MCP tool needs **both** the handler and the tool wrapper
(don't ship REST-only). Route paths, env names, response markers, and tool names
are contract strings — keep them in `contract.rs` / `apxm.contract`, not as
handler literals.

## Anti-patterns

- Putting business logic in the MCP server. It is a thin shim.
- Accepting secrets as tool arguments.
- Adding a second orchestration status/events/cancel control plane.
