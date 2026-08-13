---
name: mcp-server
group: Domain
description: Use when working on APXM MCP contract constants or local stdio registration.
user-invocable: true
---

# APXM MCP surfaces

Load `_shared/apxm-development-rules.md` before broad work. MCP is a thin
transport over the APXM compiler and runtime contracts; it does not define
program lifecycle, scheduling, authority, or provider routing.

Local stdio registration is owned by `tools/scripts/apxm_mcp_install.py` and
must be exercised through `dekk agents mcp install`. Keep secrets in the
environment, keep handles typed, and route execution through the same exact
admission and capability boundaries as the CLI.
