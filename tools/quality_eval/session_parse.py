"""Read the entry function's terminal output from an APXM session directory.

Primary path: trust the runtime contract from PC8 — `results.json::final_output`
is always populated. Fallback path handles older sessions or partial writes by
reading `exit_values` (highest-id) then `token_values` (highest-id) and
stringifying via JSON for non-string Values.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


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


def extract_final_output(session_dir: Path) -> str:
    """Return the canonical terminal output for a session.

    Order of trust:
        1. `results.json::final_output`  (PC8-guaranteed)
        2. `results.json::exit_values`   (highest node id)
        3. `results.json::token_values`  (highest node id)
    Raises RuntimeError when none of the three yields anything.
    """
    results_path = Path(session_dir) / "results.json"
    data = json.loads(results_path.read_text())

    fo = data.get("final_output")
    if isinstance(fo, str) and fo:
        return fo

    exits = data.get("exit_values") or {}
    via_exit = _highest_id_value(exits)
    if via_exit:
        return via_exit

    tokens = data.get("token_values") or {}
    via_token = _highest_id_value(tokens)
    if via_token:
        return via_token

    raise RuntimeError(
        f"results.json at {results_path} has no extractable output "
        "(final_output empty, exit_values empty, token_values empty)"
    )
