#!/usr/bin/env python3
"""mooncake_row.py - Single-ASK graph for one Mooncake trace row.

Driven by `mooncake_replay.py`. Reads the row's synthesized prompt from
an env var (set by the driver per subprocess call) and emits a one-ASK
graph against the configured vLLM benchmark backend. The terminal is the
ASK node itself (matches the `g.done(<terminal>)` pattern in
`stress/prefix_fanout_concurrent.py` — the proven shape that the existing
benchmark harness validates against).

Env-var contract (set by the driver):
  MOONCAKE_INPUT_TEXT      str   prompt produced by _mooncake_hash.synthesize_prompt
  MOONCAKE_MAX_TOKENS      int   trace row's output_length (recorded by the
                                  driver in BatchRow.rows; not enforced
                                  per-call today — output cap requires a
                                  per-ask attribute that the GraphRecorder
                                  API does not expose yet)
  MOONCAKE_ROW_INDEX       int   row index within the trace (informational)
  APXM_MATRIX_VARIANT      int   tenant index (existing convention)
"""
import os

from apxm import compile, GraphRecorder

from _config import VLLM, VLLM_ROUTE

ENV_INPUT_TEXT = "MOONCAKE_INPUT_TEXT"
ENV_MAX_TOKENS = "MOONCAKE_MAX_TOKENS"
ENV_ROW_INDEX = "MOONCAKE_ROW_INDEX"
ENV_VARIANT = "APXM_MATRIX_VARIANT"

DEFAULT_PROMPT = (
    "Default Mooncake row prompt. Driver did not set MOONCAKE_INPUT_TEXT; "
    "answer briefly so the smoke run still completes."
)


def _row_prompt() -> str:
    text = os.environ.get(ENV_INPUT_TEXT, "")
    return text if text else DEFAULT_PROMPT


def _row_index() -> str:
    return os.environ.get(ENV_ROW_INDEX, "0")


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def mooncake_row(g: GraphRecorder):
    answer = g.ask(
        name=f"mooncake_row_{_row_index()}",
        prompt=_row_prompt(),
    )
    g.done(answer)


if __name__ == "__main__":
    print(mooncake_row._graph.to_air())
