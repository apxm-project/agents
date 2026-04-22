"""Per-fixture spend caps backed by Phase A's `metrics.json::token_accounting`.

Two knobs only — `max_llm_calls` and `max_total_tokens`. They are absolute
ceilings, not soft warnings; a fixture run that exceeds either is a FAIL
even when the rubric is otherwise green. The intent is to catch a pass
that silently inflates token usage (or a backend swap that doubles
prompt size) before it eats CI budget.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path


@dataclass
class Budget:
    max_llm_calls: int = 0
    max_total_tokens: int = 0

    @property
    def has_caps(self) -> bool:
        return self.max_llm_calls > 0 or self.max_total_tokens > 0


def load_budget(path: str | Path) -> Budget:
    """Parse a `budget.toml` next to a fixture. Missing file → empty Budget
    (i.e. no caps), so fixtures don't have to declare one to run."""
    p = Path(path)
    if not p.exists():
        return Budget()
    try:
        import tomllib
    except ModuleNotFoundError:  # pragma: no cover
        import tomli as tomllib  # type: ignore[no-redef]
    with open(p, "rb") as f:
        data = tomllib.load(f)
    return Budget(
        max_llm_calls=int(data.get("max_llm_calls", 0)),
        max_total_tokens=int(data.get("max_total_tokens", 0)),
    )


def enforce(session_dir: Path, budget: Budget) -> list[str]:
    """Compare a session's `metrics.json` against `budget`.

    Returns a list of human-readable failures; empty list means within budget
    or no caps were declared. Missing/unparseable `metrics.json` is treated
    as an enforcement failure when caps exist (we cannot prove we stayed
    inside the budget) and as a no-op when no caps are declared.
    """
    if not budget.has_caps:
        return []

    metrics_path = Path(session_dir) / "metrics.json"
    if not metrics_path.exists():
        return [f"budget: metrics.json missing at {metrics_path}"]

    try:
        data = json.loads(metrics_path.read_text())
    except json.JSONDecodeError as e:
        return [f"budget: metrics.json parse error: {e}"]

    total = (data.get("token_accounting") or {}).get("total") or {}
    calls = int(total.get("call_count", 0))
    tokens = int(total.get("total_tokens", 0))

    failures: list[str] = []
    if budget.max_llm_calls and calls > budget.max_llm_calls:
        failures.append(f"max_llm_calls: {calls} > {budget.max_llm_calls}")
    if budget.max_total_tokens and tokens > budget.max_total_tokens:
        failures.append(f"max_total_tokens: {tokens} > {budget.max_total_tokens}")
    return failures
