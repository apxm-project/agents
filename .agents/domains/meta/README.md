# Domain — meta

Skill authoring, MCP surfaces, and the agent-facing instruction outputs.

## Skills

- **mcp-server** — work on APXM MCP surfaces, local registration, and bridges.
- **design-docs** — gate overclaim/citation-drift in conceptual `docs/`.

Skill-authoring conventions live in `.agents/skills/README.md`
(§ How to add a skill).

## SSOT

- `AGENTS.md` — the body of every agent config.
- `.agents/skills/_shared/` — shared rules loaded by skills.
- `.agents/skills/<name>/SKILL.md` — individual skill files.

Generated adapters (synchronized by the workspace agent-skill tooling):

- `CLAUDE.md`, `CODEX.md`, `.cursorrules`, and
  `.github/copilot-instructions.md` — thin pointers to `AGENTS.md`
- `.agents.json` — machine-readable instruction and skill index
- `.claude/skills` — link to `.agents/skills`

## MCP

- Any downstream-managed inbound MCP surface owns external auth and acting-
  principal resolution. It may project exact admitted Capabilities, but it does
  not expose an Agents-owned Agent Program lifecycle tool family.
- Rust `mcp-server` owns local stdio compile/query tools and outbound bridge
  glue.
- Register supported clients with `dekk agents mcp install`.

## Related rules

- `_shared/apxm-agent-operating-rules.md`
- `_shared/apxm-development-rules.md`
