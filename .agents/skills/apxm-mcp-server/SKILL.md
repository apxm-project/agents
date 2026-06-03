---
name: apxm-mcp-server
description: Use when working on or registering the APXM MCP server (mcp/apxm_server.py). Each tool shells dekk apxm <cmd>; registration goes through dekk apxm mcp install.
user-invocable: true
---

# APXM MCP Server

Load `_shared/apxm-development-rules.md` before broad work.

## What the MCP server is

A FastMCP-based Python server at `mcp/apxm_server.py` that exposes
`dekk apxm` operations as MCP tools. Each tool shells the
corresponding `dekk apxm` command and returns `{exit_code, stdout,
stderr}` as JSON.

The pattern follows `carts-plugin/mcp/carts_server.py`. Every tool is
a thin wrapper — no MCP server should reimplement what Dekk already
does.

## Tools exposed (target set)

- **Build/test**: `apxm_build`, `apxm_test`, `apxm_doctor`,
  `apxm_codegen`.
- **Compile/run**: `apxm_compile`, `apxm_execute`, `apxm_run`,
  `apxm_validate`, `apxm_ops_list`.
- **vLLM ops**: `apxm_vllm_doctor`, `apxm_vllm_zoo_apply`,
  `apxm_vllm_service_list`, `apxm_vllm_service_exec`.
- **Checks**: `apxm_check_no_legacy`.

## Registration

```bash
dekk apxm mcp install     # registers with supported agent configs
                          # (Claude Code, Cursor, etc.)
```

The installation path is owned by `tools/scripts/apxm_mcp_install.py`.
Do not bypass it.

## Rules

- Every MCP tool **must** shell `dekk apxm <cmd>` — never reimplement
  the logic in Python. The Dekk command is the contract.
- Return `{exit_code, stdout, stderr}` verbatim. Do not parse or
  filter; the calling agent can grep its own output.
- No state in the MCP server. Every tool call is stateless; concurrent
  calls must not stomp on each other (which is automatic when each
  shells a fresh subprocess).
- Secrets stay in the env — never accept `api_key` or
  `LLM_GATEWAY_KEY` as a tool argument. The Dekk env already carries
  them via the conda shell.

## Implementation skeleton

```python
from mcp.server.fastmcp import FastMCP
import subprocess, json

mcp = FastMCP("apxm")

def _run(args: list[str]) -> dict:
    p = subprocess.run(["dekk", "apxm", *args], capture_output=True, text=True)
    return {"exit_code": p.returncode, "stdout": p.stdout, "stderr": p.stderr}

@mcp.tool()
def apxm_doctor() -> dict:
    """Run dekk apxm doctor."""
    return _run(["doctor"])

# ... one wrapper per command ...

if __name__ == "__main__":
    mcp.run()
```

## Diagnostics

- `python3 -m py_compile mcp/apxm_server.py` — syntax check.
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
- Parsing stdout in the server. Pass through; let the agent grep.
- Stateful MCP tools — concurrency hazard.
- Server middleware using Starlette `BaseHTTPMiddleware` — use raw ASGI. Its
  receive-queue treats disconnect polls as disconnects and silently nulls chat
  responses (`feedback_basehttpmiddleware_breaks_chat`).
