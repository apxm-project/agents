#!/usr/bin/env python3
"""APXM Python-frontend source for the discord-project-answer skill.

Emits canonical AIR for a one-step answer flow: an LLM role answers a question
grounded ONLY in the agent's accumulated beliefs (the project memory the curate
skill built), returned under the `answer` output token.

Emit the canonical .air with:
    APXM_EMIT_AIR=1 python answer.py > skill.air
then compile via `dekk apxm compile skill.air -o skill.apxmobj`.
"""

from apxm import compile, GraphRecorder

_ANSWER_PROMPT = """\
You answer questions about a project using only its accumulated memory. The
input is a JSON envelope { question, agent_context } where agent_context.beliefs
is the project's memory (scoped to agent_context.belief_scope), built from
everything said in the channel.

Answer the question using ONLY agent_context.beliefs as ground truth.

Rules:
1. Be specific: cite the relevant decisions, owners, deadlines, and facts.
2. If the beliefs do not cover the question, say so plainly — do not invent.
3. Keep it concise and directly useful; no preamble, no restating the question.
4. Output the answer text only.

Envelope:
{payload}
"""


@compile()
def discord_project_answer(g: GraphRecorder, payload: str):
    """Answer a question grounded in the agent's beliefs."""
    answer = g.ask(name="answer", prompt=_ANSWER_PROMPT)
    g.done(source=answer)


if __name__ == "__main__":
    discord_project_answer.run_sync("{}")
