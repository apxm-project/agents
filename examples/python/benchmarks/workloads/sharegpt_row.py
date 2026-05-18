#!/usr/bin/env python3
"""sharegpt_row.py - Sequential ASK chain for one ShareGPT conversation.

Driven by a replay harness (e.g. `sharegpt_replay.py`, mirrors
`mooncake_replay.py`). Reads a ShareGPT conversation — a list of
alternating `human` / `gpt` turns — from an env var as JSON, and emits
a sequential ASK chain where each `human` turn becomes one ASK node.
Each ASK's prompt is the human turn rendered with the prior
conversation context, so the trace's true prompt-cache reuse pattern
(common conversation prefix grows turn-by-turn) is preserved.

Why a chain, not a single ASK: ShareGPT rows are multi-turn dialogues
whose value is the *sequential* prefix growth — collapsing to a single
prompt would lose the prefix-cache signal that makes ShareGPT
interesting as a workload. A sequential chain matches what a real
multi-turn user sees and what RadixAttention / the APXM pin path can
exploit.

Env-var contract (set by the driver):
  SHAREGPT_CONVERSATION_JSON   str   JSON list of {"from": "human"|"gpt",
                                       "value": "..."} turns
  SHAREGPT_ROW_INDEX            int   row index in the trace (informational)
  SHAREGPT_MAX_TURNS            int   cap on human turns to emit (default 8;
                                       large conversations would otherwise
                                       overflow max_model_len)
  APXM_MATRIX_VARIANT           int   tenant index (existing convention)
"""
import json
import os

from apxm import GraphRecorder, compile

from _config import VLLM, VLLM_ROUTE

ENV_CONVERSATION = "SHAREGPT_CONVERSATION_JSON"
ENV_ROW_INDEX = "SHAREGPT_ROW_INDEX"
ENV_MAX_TURNS = "SHAREGPT_MAX_TURNS"
ENV_VARIANT = "APXM_MATRIX_VARIANT"
# Cohort tag — all turns of the same conversation share the prefix
# growing-prefix structure. Stamping reuse_group lets the APXM pin
# path keep this conversation's KV blocks resident across iterations
# the same way Mooncake's hash_ids[0] cohort works (Wave 27).
ENV_REUSE_GROUP = "SHAREGPT_REUSE_GROUP"

DEFAULT_MAX_TURNS = 8

DEFAULT_CONVERSATION = [
    {"from": "human", "value": "Briefly: what is APXM?"},
]


def _load_conversation() -> list[dict]:
    raw = os.environ.get(ENV_CONVERSATION, "")
    if not raw:
        return DEFAULT_CONVERSATION
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        return DEFAULT_CONVERSATION
    if not isinstance(parsed, list):
        return DEFAULT_CONVERSATION
    return [t for t in parsed if isinstance(t, dict) and t.get("from") and "value" in t]


def _max_turns() -> int:
    raw = os.environ.get(ENV_MAX_TURNS, "")
    try:
        n = int(raw) if raw else DEFAULT_MAX_TURNS
    except ValueError:
        n = DEFAULT_MAX_TURNS
    return max(1, n)


def _row_index() -> str:
    return os.environ.get(ENV_ROW_INDEX, "0")


def _reuse_group() -> str | None:
    val = os.environ.get(ENV_REUSE_GROUP, "").strip()
    return val or None


def _render_prompt(history: list[dict], current_human: str) -> str:
    """Render the conversation prefix + the current human turn.

    The whole prior conversation is repeated each turn — that's exactly
    what produces the shared-prefix growth pattern the prefix cache
    exploits. The driver records `prompt_token_estimate` separately so
    we don't need to truncate here.
    """
    lines: list[str] = []
    for turn in history:
        role = "User" if turn["from"] == "human" else "Assistant"
        lines.append(f"{role}: {turn['value']}")
    lines.append(f"User: {current_human}")
    lines.append("Assistant:")
    return "\n".join(lines)


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def sharegpt_row(g: GraphRecorder):
    conversation = _load_conversation()
    max_turns = _max_turns()
    row_idx = _row_index()

    # Pre-walk to collect human turns we'll emit and the in-prefix
    # history each ASK sees. Each ASK's prompt is `history_so_far +
    # current_human` so the rendered prefix grows monotonically — the
    # exact pattern RadixAttention can deduplicate.
    history: list[dict] = []
    human_turns: list[tuple[str, list[dict]]] = []
    for turn in conversation:
        if turn["from"] == "human":
            human_turns.append((turn["value"], list(history)))
            if len(human_turns) >= max_turns:
                break
        history.append(turn)

    if not human_turns:
        human_turns = [(DEFAULT_CONVERSATION[0]["value"], [])]

    cohort = _reuse_group() or f"sharegpt-conversation-{row_idx}"
    last = None
    for i, (human_value, history_so_far) in enumerate(human_turns):
        node = g.ask(
            name=f"sharegpt_row_{row_idx}_turn_{i}",
            prompt=_render_prompt(history_so_far, human_value),
            reuse_group=cohort,
        )
        last = node
    g.done(last)


if __name__ == "__main__":
    print(sharegpt_row._graph.to_air())
