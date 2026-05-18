#!/usr/bin/env python3
"""loogle_row.py - Shared-prefix + per-question fanout for one LooGLE row.

Driven by a replay harness (mirrors `mooncake_replay.py`). One LooGLE
row is a long source document plus N sub-questions about it. The
adapter emits a fan-out graph: every question becomes its own ASK
node, and every ASK's prompt is `document + question`. That makes the
document the canonical shared prefix and exercises exactly the
long-shared-context prefix-cache pattern RadixAttention and the APXM
pin path target.

Why fan-out, not a chain: LooGLE questions are typically independent
("what is X mentioned in section 3", "summarize section 5"). A chain
would serialize them and lose the prefix-cache speedup that comes from
hitting the same document N times in parallel.

Env-var contract (set by the driver):
  LOOGLE_DOCUMENT          str   the long source document text
  LOOGLE_QUESTIONS_JSON    str   JSON list of question strings
  LOOGLE_ROW_INDEX         int   row index within the source split
  LOOGLE_MAX_QUESTIONS     int   cap on questions to fan out (default 6)
  APXM_MATRIX_VARIANT      int   tenant index (existing convention)
"""
import json
import os

from apxm import GraphRecorder, compile

from _config import VLLM, VLLM_ROUTE

ENV_DOCUMENT = "LOOGLE_DOCUMENT"
ENV_QUESTIONS = "LOOGLE_QUESTIONS_JSON"
ENV_ROW_INDEX = "LOOGLE_ROW_INDEX"
ENV_MAX_QUESTIONS = "LOOGLE_MAX_QUESTIONS"
ENV_VARIANT = "APXM_MATRIX_VARIANT"

DEFAULT_MAX_QUESTIONS = 6

DEFAULT_DOCUMENT = (
    "APXM is a library system for agent skills. A skill compiles to a "
    "typed AIR graph and a reusable artifact, the way a function "
    "compiles to an object file. The runtime enforces capabilities, "
    "emits reproducible session traces, and the compiler's optimizer "
    "eliminates redundant LLM calls across every existing skill."
)

DEFAULT_QUESTIONS = [
    "What is the analogy APXM uses to describe a skill?",
    "What does the runtime enforce?",
]


def _document() -> str:
    text = os.environ.get(ENV_DOCUMENT, "")
    return text if text else DEFAULT_DOCUMENT


def _questions() -> list[str]:
    raw = os.environ.get(ENV_QUESTIONS, "")
    if not raw:
        return DEFAULT_QUESTIONS
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        return DEFAULT_QUESTIONS
    if not isinstance(parsed, list):
        return DEFAULT_QUESTIONS
    return [str(q) for q in parsed if q]


def _max_questions() -> int:
    raw = os.environ.get(ENV_MAX_QUESTIONS, "")
    try:
        n = int(raw) if raw else DEFAULT_MAX_QUESTIONS
    except ValueError:
        n = DEFAULT_MAX_QUESTIONS
    return max(1, n)


def _row_index() -> str:
    return os.environ.get(ENV_ROW_INDEX, "0")


def _render_prompt(document: str, question: str) -> str:
    """Prefix-then-question. The document is byte-identical across all
    questions in this row, so the prefix cache sees the document once
    and reuses it for every question."""
    return f"Document:\n{document}\n\nQuestion: {question}\nAnswer:"


@compile(default_provider=VLLM, default_route=VLLM_ROUTE)
def loogle_row(g: GraphRecorder):
    document = _document()
    questions = _questions()[: _max_questions()]
    row_idx = _row_index()

    if not questions:
        questions = DEFAULT_QUESTIONS

    asks = []
    for i, question in enumerate(questions):
        node = g.ask(
            name=f"loogle_row_{row_idx}_q{i}",
            prompt=_render_prompt(document, question),
        )
        asks.append(node)

    # Merge to a single terminal so g.done has a well-defined sink — the
    # pattern stress/prefix_fanout_concurrent uses for fan-out graphs.
    terminal = g.merge(f"loogle_row_{row_idx}_merge", *asks)
    g.done(terminal)


if __name__ == "__main__":
    print(loogle_row._graph.to_air())
