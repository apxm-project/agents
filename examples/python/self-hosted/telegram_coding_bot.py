#!/usr/bin/env python3
"""telegram_coding_bot.py -- Telegram -> coding agent -> reply via http_post.

One execution per inbound Telegram message. apxm-os fires this skill once
per webhook delivery (detach=true -> 202 fast-ack; no Telegram timeout).

Uses http_post (builtin) to send the reply — no pack capability needed.
Bot token injected via APXM_TELEGRAM_BOT_TOKEN env var set in OS agent manifest.

Compile:
  dekk agents compile examples/python/self-hosted/telegram_coding_bot.py

Install:
  mkdir -p ~/.apxm/skills/telegram-coding-bot
  cp .apxm/compiled/telegram_coding_bot.apxmobj ~/.apxm/skills/telegram-coding-bot/skill.apxmobj
"""

import os
from apxm import GraphRecorder, agent_cwd, compile, emit_air_if_requested
from apxm._generated.agents import codex


BOT_TOKEN = os.environ.get("APXM_TELEGRAM_BOT_TOKEN", "")


@compile()
def telegram_coding_bot(g: GraphRecorder):
    """Receive a Telegram message, spawn Codex, send reply.

    Parameters:
        event (str): Telegram message JSON (the message sub-object from payload_pointer)
    """
    g.param("event", "str")
    cwd = agent_cwd()

    # 1. Parse chat_id and user task from the Telegram message JSON
    parse = g.think(
        name="parse_message",
        prompt=(
            "You received a Telegram message JSON object (the 'message' field of a Telegram Update).\n"
            "Output ONLY compact JSON, no prose:\n\n"
            "Message: {event}\n\n"
            '{"chat_id":"<chat.id as string>","task":"<text field, or (no text) if missing>"}'
        ),
    )

    # 2. Spawn Codex to complete the coding task
    coder = g.spawn("coder", profile=codex, cwd=cwd)
    coding_result = coder.ask(
        prompt=(
            "Complete this coding task concisely. Output only the result:\n\n{parse[task]}"
        ),
    )

    # 3. Send reply via Telegram Bot API using http_post (builtin — no pack needed)
    # Bot token is in APXM_TELEGRAM_BOT_TOKEN env var, resolved at compile time here.
    # For production, store the token in apxm-auth and use provider.call instead.
    g.invoke(
        "send_reply",
        capability="http_post",
        params={
            "url": f"https://api.telegram.org/bot{BOT_TOKEN}/sendMessage",
            "body": {
                "chat_id": "{parse[chat_id]}",
                "text": "{coding_result}",
            },
        },
    )

    g.done()


emit_air_if_requested(telegram_coding_bot)
