# Builtin skills

This directory is the `apxm-server` **builtin skill root** — compiled into the
server at build time via `APXM_BUILTIN_SKILL_ROOT` (see `../build.rs`) and
prepended to skill discovery ahead of `~/.apxm/libs` and any `--skill-root`.
The scan recurses, so both flat skill dirs and pack-layout dirs
(`<pack-id>/skills/<skill-id>/`) are discovered.

## Skills

- `apxm-plan-as-graph/` — plan-as-graph dispatcher: NL → AIR → compile → dispatch
  → trace. The flagship operating skill, a **source skill** (`SKILL.md` +
  `prompt.md` + `schema.json` + `examples/`) — no `.apxmobj` by design; the
  server runs it via its dispatcher path. Output contract: `summary` (json).
  (The former apxm-libs copy carried a `skill.air` whose signature returned
  `task_dag` instead — a divergence from this `summary` contract; it was **not**
  grafted on. Deciding the true output contract + producing a matching AIR is a
  plan-as-graph design question, separate from the dissolution.)
- `apxm-os-discord-curate/` — apxm-os listener skill pair
  (`discord-project-curate` + `discord-project-answer`): curate channel messages
  into project memory, then answer from accumulated context. Ships compiled
  (`skill.apxmobj`); `discord-project-curate` carries a `triggers.toml` sidecar
  that apxm-os reads to arm it on a Discord channel cue.

## History — the former apxm-libs repo

These operating skills previously lived in a separate `apxm-libs` repo (a
"compiled-skill library"). That repo was dissolved on 2026-06-03: with the
catalog down to two skills, a separate repo added no value over a skill folder
here. The decision trail and where the rest of its content went:

- Build-APXM contributor skills → `apxm/.agents` — see
  [`../../../../docs/skills-migration/evicted-build-apxm-skills.md`](../../../../docs/skills-migration/evicted-build-apxm-skills.md).
- Provider connectors → `apxm-auth` (`providers.toml`) — see
  [`../../../../docs/skills-migration/connectors-removed.md`](../../../../docs/skills-migration/connectors-removed.md).
- Generic obra/superpowers dev-workflow ports → dropped (host/plugin layer).

The pack validator was preserved as `tools/scripts/validate_pack.py`.

The optional sibling-clone discovery path was removed as part of the
dissolution: `apxm-server`'s `SIBLING_LIBS_DIR` constant and its lookup are
gone, the `dekk apxm libs` installer (`tools/scripts/apxm_libs.py`) and its
`.dekk.toml` command group were deleted, and `apxm-studio`'s
`default_packs_catalog_root` no longer walks for a sibling `apxm-libs/packs`.
Discovery now uses the builtin root (this dir) + `~/.apxm/libs` +
`--skill-root`/`APXM_SKILL_ROOTS`.
