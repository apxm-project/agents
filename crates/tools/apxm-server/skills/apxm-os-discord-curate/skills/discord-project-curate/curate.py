#!/usr/bin/env python3
"""APXM Python-frontend source for the discord-project-curate skill.

Emits canonical AIR for a one-step curation flow: an LLM role reads the apxm-os
`{ event, agent_context }` envelope (a Discord message + current beliefs) and
returns a CurationResult JSON under the `curation` output token, which apxm-os
applies as belief writes + an episode append.

Emit the canonical .air with:
    APXM_EMIT_AIR=1 python curate.py > skill.air
then compile via `dekk apxm compile skill.air -o skill.apxmobj`.
"""

from apxm import compile, GraphRecorder

_CURATE_PROMPT = """\
You maintain a project's memory from its Discord channel. The input is a JSON
envelope { event, agent_context }: a Discord MESSAGE_CREATE plus the agent's
current beliefs (agent_context.beliefs, scoped to agent_context.belief_scope).

Read the new message and the beliefs, then emit COMPACT JSON ONLY (no prose, no
markdown fences) with exactly this shape:

{ "belief_writes":  [ { "key": "project.<name>.<slug>", "value": <json> } ],
  "episode_appends":[ { "event_type": "discord.message", "payload": <message> } ] }

Rules:
1. Write a belief only for durable, reusable facts (decisions, owners, deadlines,
   requirements, status, links). Skip chit-chat.
2. Keys MUST start with the belief_scope prefix; out-of-scope writes are dropped.
3. Merge/refine existing beliefs rather than duplicating them.
4. Always append exactly one episode for the raw message.
5. If nothing durable was said, return belief_writes: [] with the one episode.
6. Emit valid JSON; close every array and object.

Envelope:
{payload}
"""


@compile()
def discord_project_curate(g: GraphRecorder, payload: str):
    """Curate one Discord message into project memory."""
    curation = g.ask(name="curation", prompt=_CURATE_PROMPT)
    g.done(source=curation)


if __name__ == "__main__":
    # With APXM_EMIT_AIR=1 this prints the canonical .air and exits.
    discord_project_curate.run_sync("{}")
