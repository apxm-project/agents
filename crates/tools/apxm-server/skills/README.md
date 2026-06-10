# Builtin skills

This directory is the `apxm-server` **builtin skill root** — compiled into the
server at build time via `APXM_BUILTIN_SKILL_ROOT` (see `../build.rs`) and
prepended to skill discovery ahead of `~/.apxm/libs` and any `--skill-root`.
The scan recurses, so both flat skill dirs and pack-layout dirs
(`<pack-id>/skills/<skill-id>/`) are discovered.

## Skills

- `prompt-as-workflow/` — prompt-as-workflow dispatcher: NL → AIR → compile → dispatch
  → trace. The flagship operating skill, a **source skill** (`SKILL.md` +
  `prompt.md` + `schema.json` + `examples/`) — no `.apxmobj` by design; the
  server runs it via its dispatcher path. Output contract: `summary` (json).

## Boundary

The generic APXM repo keeps only provider-agnostic builtins here.
Integration-specific skills, provider connectors, triggers, credentials, and
listener examples belong in integration packs or downstream deployments, not in
`apxm-server`.

Discovery uses this builtin root plus `~/.apxm/libs`,
`--skill-root`, and `APXM_SKILL_ROOTS`.
