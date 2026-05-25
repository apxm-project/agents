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
import sys
from pathlib import Path

_REPO_ROOT = Path(__file__).resolve().parents[4]
_APXM_PKG_DIR = str(_REPO_ROOT / "crates" / "compiler" / "apxm-frontend" / "python")
if _APXM_PKG_DIR not in sys.path:
    sys.path.insert(0, _APXM_PKG_DIR)

from apxm.contract import EnvVar, WireKey  # noqa: E402

from apxm import compile, GraphRecorder

from _config import VLLM, VLLM_ROUTE

ENV_INPUT_TEXT = "MOONCAKE_INPUT_TEXT"
ENV_INPUT_TEXT_PATH = "MOONCAKE_INPUT_TEXT_PATH"
ENV_MAX_TOKENS = "MOONCAKE_MAX_TOKENS"
ENV_ROW_INDEX = "MOONCAKE_ROW_INDEX"
ENV_VARIANT = EnvVar.APXM_MATRIX_VARIANT.value
# Cohort tag derived from the trace's hash_ids[0] (the first
# prefix-cache block id). Rows that share their first hash_id land in
# the same APXM reuse_group, which is the runtime's cohort hook for
# the pin path. Without this, the Mooncake replay's natural
# prefix-cache sharing structure is invisible to the APXM scheduler.
# Driver-set via the row adapter env var below.
ENV_REUSE_GROUP = "MOONCAKE_REUSE_GROUP"

DEFAULT_PROMPT = (
    "Default Mooncake row prompt. Driver did not set MOONCAKE_INPUT_TEXT; "
    "answer briefly so the smoke run still completes."
)


def _row_prompt() -> str:
    # Prefer path-based delivery — Mooncake prompts routinely exceed
    # 20 KB, and Linux ARG_MAX caps cumulative env+arg size, so
    # passing prompts inline overflows at moderate concurrency. The
    # driver writes the prompt to a tempfile and exports the path here.
    path = os.environ.get(ENV_INPUT_TEXT_PATH, "")
    if path:
        try:
            with open(path, "r", encoding="utf-8") as fh:
                text = fh.read()
            return text if text else DEFAULT_PROMPT
        except OSError:
            pass
    text = os.environ.get(ENV_INPUT_TEXT, "")
    return text if text else DEFAULT_PROMPT


def _row_index() -> str:
    return os.environ.get(ENV_ROW_INDEX, "0")


def _reuse_group() -> str | None:
    """Cohort tag for the APXM pin path. The driver derives it from
    `hash_ids[0]` of the trace row — rows sharing their first
    prefix-cache block land in the same cohort. When unset (legacy
    driver or smoke-only invocation), return None so the runtime
    treats the request as solo."""
    val = os.environ.get(ENV_REUSE_GROUP, "").strip()
    return val or None


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def mooncake_row(g: GraphRecorder):
    kwargs = {
        "name": f"mooncake_row_{_row_index()}",
        "prompt": _row_prompt(),
    }
    cohort = _reuse_group()
    if cohort:
        kwargs[WireKey.REUSE_GROUP.value] = cohort
    answer = g.ask(**kwargs)
    g.done(answer)


if __name__ == "__main__":
    print(mooncake_row._graph.to_air())
