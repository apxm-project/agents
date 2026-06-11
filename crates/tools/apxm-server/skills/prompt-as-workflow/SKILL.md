---
name: prompt-as-workflow
description: Use for complex coding-agent work that should be emitted as APXM AIR, compiled, dispatched, and summarized instead of executed as an untyped checklist.
mcp_server: apxm-mcp-server
mcp_tool: prompt_as_workflow
resources:
  prompt: skill://prompt-as-workflow/prompt.md
---

# APXM Prompt As Workflow

Use this skill when a request needs a typed, inspectable APXM workflow rather
than a free-form checklist. Good triggers include audits, multi-stage
implementation work, refactors, operation additions, code-review councils, and
SDLC pipelines where multiple independent or sequential work packets should be
scheduled explicitly.

## Contract

The skill asks APXM to emit canonical AIR for the requested task, compiles it
through the APXM compiler, and dispatches the result through the runtime. The
calling agent receives a compact summary, a `trace_id`, and the emitted `.air`
path. Intermediate node outputs remain outside the agent context unless the
agent explicitly fetches them with a trace query tool.

## Inputs

- `task`: required natural-language task.
- `context`: optional compact context, such as relevant files, constraints, or
  repository state.
- `constraints`: optional structured object for budget, model policy, sandbox
  policy, or maximum workflow size.

## Output Discipline

Return only:

- `status`
- `summary`
- `trace_id`
- `air_path`
- `air_hash`
- `node_count`
- `edge_count`
- `warnings`

Do not return the full execution trace by default.
