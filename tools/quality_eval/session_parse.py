"""Read the entry function's terminal output from an APXM session directory.

Primary path: trust the runtime contract from PC8 — `results.json::final_output`
is always populated. Fallback path handles older sessions or partial writes by
reading `exit_values` (highest-id) then `token_values` (highest-id) and
stringifying via JSON for non-string Values.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

from ._keys import ResultsKeys, SessionFiles

# Reasoning models prepend a "thinking" block before the visible answer. Two
# observed shapes:
#   * Symmetric (DeepSeek-R1 etc.): "<think>...</think>\nanswer"
#   * Asymmetric (Qwen3.5-4B served by vLLM): "Thinking Process:\n...\n</think>\nanswer"
#     — no opening tag, just the closing one terminating the prelude.
# Both end with `</think>`, so anchor at start-of-text and consume up through
# the FIRST closing tag (lazy). Then also sweep any later symmetric blocks
# that might appear inline. Order matters: leading sweep first.
_LEADING_THINK = re.compile(r"\A.*?</think>\s*", re.DOTALL | re.IGNORECASE)
_INLINE_THINK = re.compile(r"<think>.*?</think>\s*", re.DOTALL | re.IGNORECASE)


def _strip_thinking_blocks(text: str) -> str:
    """Remove leading and inline thinking blocks emitted by reasoning models.

    Keeps the post-thinking content the user actually sees. Idempotent on
    text that has no thinking blocks (the leading regex requires a `</think>`,
    so plain text is left untouched).
    """
    text = _LEADING_THINK.sub("", text)
    text = _INLINE_THINK.sub("", text)
    return text.lstrip()


def _stringify(value: Any) -> str:
    if isinstance(value, str):
        return value
    return json.dumps(value)


def _highest_id_value(d: dict[str, Any]) -> str | None:
    if not d:
        return None
    # Keys are stringified u64 node ids on the Rust side; sort numerically.
    pick_id = max(d.keys(), key=lambda k: int(k))
    return _stringify(d[pick_id])


def _resolve_session_root(session_dir: Path) -> Path:
    """Accept either a session dir (contains results.json) or its parent.

    `dekk apxm execute --emit-session <PATH>` treats PATH as a base directory
    and creates `<PATH>/<stem>-<timestamp>/` underneath it. The harness
    passes a tempdir as PATH, so we must descend into the single child to
    locate `results.json`.
    """
    direct = session_dir / SessionFiles.RESULTS
    if direct.exists():
        return session_dir
    children = [p for p in session_dir.iterdir() if p.is_dir()]
    if len(children) == 1 and (children[0] / SessionFiles.RESULTS).exists():
        return children[0]
    raise FileNotFoundError(
        f"no {SessionFiles.RESULTS} under {session_dir} or its single subdir"
    )


def extract_final_output(session_dir: Path) -> str:
    """Return the canonical terminal output for a session.

    Order of trust:
        1. `results.json::final_output`  (PC8-guaranteed)
        2. `results.json::exit_values`   (highest node id)
        3. `results.json::token_values`  (highest node id)
    Raises RuntimeError when none of the three yields anything.
    """
    root = _resolve_session_root(Path(session_dir))
    results_path = root / SessionFiles.RESULTS
    data = json.loads(results_path.read_text())

    fo = data.get(ResultsKeys.FINAL_OUTPUT)
    if isinstance(fo, str) and fo:
        return _strip_thinking_blocks(fo)

    exits = data.get(ResultsKeys.EXIT_VALUES) or {}
    via_exit = _highest_id_value(exits)
    if via_exit:
        return _strip_thinking_blocks(via_exit)

    tokens = data.get(ResultsKeys.TOKEN_VALUES) or {}
    via_token = _highest_id_value(tokens)
    if via_token:
        return _strip_thinking_blocks(via_token)

    raise RuntimeError(
        f"results.json at {results_path} has no extractable output "
        "(final_output empty, exit_values empty, token_values empty)"
    )
