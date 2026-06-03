---
name: discord-project-curate
description: on_event curation for the always-listening Discord project agent — turns each channel message into project belief writes + an episode the answer skill draws on.
fallback: native curation
---

# Discord Project Curate

The `on_event` skill for an apxm-os agent listening to a Discord project channel.
Each message arrives as a `{ event, agent_context }` envelope; this compiled flow
runs one LLM role that extracts durable project facts into `belief_writes`
(scope-filtered into the agent's belief store) and logs the message as an
`episode_append`.

## Contract
- **Input** `payload` — JSON `{ event, agent_context }` from apxm-os.
- **Output** `curation` (json) — `CurationResult { belief_writes, episode_appends }`.

## Build
Authored as an APXM Python-frontend flow (`curate.py`) that emits canonical AIR:
```
APXM_EMIT_AIR=1 python curate.py > skill.air
dekk apxm compile skill.air -o skill.apxmobj   # compiles skill.air -> skill.apxmobj
```
Compiled artifact is shipped; running the `ais.ask` role needs a configured model
backend on apxm-server.
