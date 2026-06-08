---
name: apxm-mcp-server
description: Use when working on APXM MCP surfaces: the Rust HTTP `/v1/mcp` endpoint, the Rust stdio `apxm-mcp-server` binary, or cross-agent MCP registration. Prefer server-owned HTTP MCP for workflow/orchestration control.
user-invocable: true
---

# APXM MCP Server

Load `_shared/apxm-development-rules.md` before broad work.

## What the MCP surfaces are

APXM has two Rust MCP surfaces:

- `apxm-server` exposes HTTP MCP at `POST /v1/mcp`. This is the
  server-owned control plane for long-running workflows, retained events,
  cancellation, server-owned sessions, skill inventory, and orchestration.
- `apxm-mcp-server` exposes stdio MCP for compile/query/debug tools. It does
  not own background workflow sessions or orchestration state.

Do not resurrect the old FastMCP `mcp/apxm_server.py` design unless the repo
reintroduces it. MCP should stay a thin interface over APXM server/runtime
capabilities.

## HTTP MCP Tools

- Compile/query: `apxm_validate`, `apxm_compile`, `apxm_ops_list`,
  `apxm_run`, `apxm_plan_as_graph`, `apxm_trace_fetch`, `apxm_aam_recall`,
  `apxm_evidence_lookup`, `apxm_capability_list`.
- Skills: `apxm_skills_list`, `apxm_skill_get`, `apxm_skill_validate`,
  `apxm_skill_call`.
- Workflow control: `apxm_workflow_start`, `apxm_workflow_status`,
  `apxm_workflow_events`, `apxm_workflow_cancel`.
- Native orchestration: `apxm_orchestrate_start`.

`apxm_orchestrate_start` compiles a bounded task/worker plan into a
server-owned workflow. The orchestrator agent calls it once, records the
returned `execution_id`, then sleeps until `apxm_workflow_events` returns
`orchestrator_wake`, `execute_complete`, `error`, or `turn_aborted`, or
`apxm_workflow_status` reports a terminal state. Real ACP workers require
`admit_capabilities: ["SPAWN_AGENT"]`.

## Stdio MCP Tools

The stdio binary exposes compile/query tools such as `apxm_validate`,
`apxm_compile`, `apxm_get_contract`, `apxm_analyze`, `apxm_plan_as_graph`,
`apxm_trace_fetch`, `apxm_aam_recall`, `apxm_evidence_lookup`, and
`apxm_capability_list`. Use HTTP MCP when a caller needs workflow start/status,
events, cancel, or orchestration.

## Registration

```bash
dekk apxm mcp install     # registers with supported agent configs
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

- `cargo check -p apxm-server` — compile the HTTP and stdio MCP binaries.
- `dekk apxm mcp install` — register; surfaces config errors.

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
- Adding a second orchestration status/events/cancel control plane. Use
  `apxm_workflow_status/events/cancel` for runs started by
  `apxm_orchestrate_start`.
- Server middleware using Starlette `BaseHTTPMiddleware` — use raw ASGI. Its
  receive-queue treats disconnect polls as disconnects and silently nulls chat
  responses (`feedback_basehttpmiddleware_breaks_chat`).
