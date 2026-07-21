# APXM domains — navigation

Domains group skills by concern. Each domain has a `README.md` that
lists the relevant skills, the canonical docs, and the workflows that
cut across them.

- [lifecycle/](lifecycle/README.md) — context → plan → execute →
  simplify → finish → commit
- [compiler/](compiler/README.md) — AIS dialect, passes, codegen
- [execution/](execution/README.md) — runtime, handlers, backends
- [operations/](operations/README.md) — vLLM service / zoo /
  storage layout
- [meta/](meta/README.md) — skill authoring, MCP server, agent
  contracts

Owner-local offline and observed prompt evaluation lives under `evaluation/`.

The skills themselves live in flat layout under
`.agents/skills/<name>/SKILL.md` (adapter-compatible). These
`domains/<area>/` directories are navigation only — they hold no
SKILL.md.
