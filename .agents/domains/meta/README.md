# Domain — meta

Skill authoring, MCP surfaces, and the agent-facing instruction outputs.

## Skills

- **mcp-server** — work on APXM MCP surfaces, local registration, and bridges.
- **design-docs** — gate overclaim/citation-drift in conceptual `docs/`.

Skill-authoring conventions live in `.agents/skills/README.md`
(§ How to add a skill).

## SSOT

- `.agents/project.md` — the body of every agent config.
- `.agents/skills/_shared/` — shared rules loaded by skills.
- `.agents/skills/<name>/SKILL.md` — individual skill files.

Mirrored outputs (no generator exists — update each by hand from
`.agents/project.md`):

- `AGENTS.md` — canonical portable instructions (Codex CLI per `.agents.json`,
  Aider, and other `AGENTS.md` readers; also the filename APXM writes for
  `codex` profile session nodes)
- `CLAUDE.md` — Claude Code entrypoint (same body as `AGENTS.md` today)
- `CODEX.md` — duplicate filename for Codex-named workflows; Codex CLI still
  resolves to `AGENTS.md` in `.agents.json`
- `.agents.json` (machine-readable manifest)
- `.cursorrules` (Cursor)
- `.github/copilot-instructions.md` (GitHub Copilot)

## MCP

- Any downstream-managed inbound MCP surface owns external auth and acting-
  principal resolution. It may project exact admitted Capabilities, but it does
  not expose an Agents-owned Agent Program lifecycle tool family.
- Rust `mcp-server` owns local stdio compile/query tools and outbound bridge
  glue.
- Register supported clients with `dekk agents mcp install`.

## CI

- `.github/workflows/validate.yml` (PR-2) — validates skills and MCP-surface
  compile coverage.

## Related rules

- `_shared/apxm-agent-operating-rules.md`
- `_shared/apxm-development-rules.md`
