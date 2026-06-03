# apxm-os-discord-curate

A **skill-kind** pack (no `[app]` table): the compiled half of the
always-listening Discord project agent. It ships two `SkillManifest`-conforming,
hash-pinned skills that `apxm-server` lists and executes:

- `discord-project-curate` — ingest each channel message into project memory
  (`belief_scope project.<name>.*`). Default-armed by the channel cue in
  `skills/discord-project-curate/triggers.toml`.
- `discord-project-answer` — answer a question grounded **only** in the beliefs
  the curate skill accumulated, with no re-investigation at query time.
  Query-driven (invoked by the agent's `answer` POST), so it ships no trigger.

This is the canonical multi-skill operating pack and the only fully-compiled one
in the catalog (`skill.apxmobj` shipped for both skills).

## How it is wired

`apxm-server` loads and executes the compiled skills; `apxm-os` arms the curate
skill from the channel cue and drives the curate→answer loop. A deployment binds
the real `channel_id` and the `on_event`/`answer` mapping through an `agents.d/`
manifest — see `apxm-os` `examples/agents.d/discord-project-listener.toml`.

## Build

Each skill is an APXM Python-frontend flow (`curate.py`, `answer.py`) that emits
canonical AIR, compiled to `.apxmobj`:

```bash
APXM_EMIT_AIR=1 python skills/discord-project-curate/curate.py > skills/discord-project-curate/skill.air
dekk apxm compile skill.air -o skill.apxmobj   # compiles skill.air -> skill.apxmobj, seals pack_hash
```

Running the curate/answer LLM roles needs a model backend configured on
`apxm-server`.
