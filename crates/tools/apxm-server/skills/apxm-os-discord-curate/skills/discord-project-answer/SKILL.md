---
name: discord-project-answer
description: answer skill for the always-listening Discord project agent — answers a question grounded in the project beliefs the curate skill accumulated, with no re-investigation at query time.
fallback: native answering
---

# Discord Project Answer

The `answer` skill for an apxm-os agent that has been listening to a Discord
project channel and curating it into memory. This compiled flow runs one LLM
role that answers a question grounded ONLY in the agent's accumulated beliefs.

## Contract
- **Input** `payload` — JSON `{ question, agent_context }` from apxm-os.
- **Output** `answer` (string) — the rendered answer (also surfaced as content).

## Build
Authored as an APXM Python-frontend flow (`answer.py`) that emits canonical AIR:
```
APXM_EMIT_AIR=1 python answer.py > skill.air
dekk apxm compile skill.air -o skill.apxmobj   # compiles skill.air -> skill.apxmobj
```
Compiled artifact is shipped; running the `ais.ask` role needs a configured model
backend on apxm-server.
