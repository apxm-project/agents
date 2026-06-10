---
name: prompt-as-workflow
description: Use for complex coding-agent work that should be emitted as APXM AIR, compiled, dispatched, and summarized instead of executed as an untyped linear plan.
mcp_server: apxm-mcp-server
mcp_tool: prompt_as_workflow
resources:
  prompt: skill://prompt-as-workflow/prompt.md
  schema: skill://prompt-as-workflow/schema.json
---

# APXM Prompt As Workflow

Use this skill when a request needs a typed, inspectable APXM workflow rather
than a free-form checklist. Good triggers include audits, multi-stage
implementation work, refactors, operation additions, code-review councils, and
SDLC pipelines where multiple independent or sequential work packets should be
scheduled explicitly.

## Contract

The skill asks APXM to emit typed workflow JSON for the requested task, validates it
against `schema.json`, compiles it through the APXM compiler, and dispatches the
result through the runtime. The calling agent receives a compact summary and a
`trace_id`. Intermediate node outputs remain outside the agent context unless
the agent explicitly fetches them with a trace query tool.

## Inputs

- `task`: required natural-language task.
- `context`: optional compact context, such as relevant files, constraints, or
  repository state.
- `constraints`: optional JSON object for budget, model policy, sandbox policy,
  or maximum workflow size.

## Output Discipline

Return only:

- `status`
- `summary`
- `trace_id`
- `workflow`
- `node_count`
- `edge_count`
- `warnings`

Do not return the full execution trace by default.
