"""Per-fixture spend caps backed by session `metrics.json` token accounting.

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

from ._keys import BudgetKeys, MetricsKeys, SessionFiles


@dataclass
class Budget:
    max_llm_calls: int = 0
    max_total_tokens: int = 0
    has_max_llm_calls: bool = False
    has_max_total_tokens: bool = False

    def __post_init__(self) -> None:
        # Direct test/helper construction historically used positive values to
        # mean "cap declared". Keep that ergonomic path while allowing loaded
        # TOML files to declare an explicit zero cap.
        if self.max_llm_calls > 0:
            self.has_max_llm_calls = True
        if self.max_total_tokens > 0:
            self.has_max_total_tokens = True

    @property
    def has_caps(self) -> bool:
        return self.has_max_llm_calls or self.has_max_total_tokens


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
        max_llm_calls=int(data.get(BudgetKeys.MAX_LLM_CALLS, 0)),
        max_total_tokens=int(data.get(BudgetKeys.MAX_TOTAL_TOKENS, 0)),
        has_max_llm_calls=BudgetKeys.MAX_LLM_CALLS in data,
        has_max_total_tokens=BudgetKeys.MAX_TOTAL_TOKENS in data,
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

    metrics_path = Path(session_dir) / SessionFiles.METRICS
    if not metrics_path.exists():
        return [f"budget: {SessionFiles.METRICS} missing at {metrics_path}"]

    try:
        data = json.loads(metrics_path.read_text())
    except json.JSONDecodeError as e:
        return [f"budget: {SessionFiles.METRICS} parse error: {e}"]

    token_accounting = data.get(MetricsKeys.TOKEN_ACCOUNTING)
    if token_accounting is None:
        runtime = data.get(MetricsKeys.SECTION_RUNTIME) or {}
        token_accounting = runtime.get(MetricsKeys.TOKEN_ACCOUNTING)
    total = (token_accounting or {}).get(MetricsKeys.TOTAL) or {}
    calls = int(total.get(MetricsKeys.CALL_COUNT, 0))
    tokens = int(total.get(MetricsKeys.TOTAL_TOKENS, 0))

    failures: list[str] = []
    if budget.has_max_llm_calls and calls > budget.max_llm_calls:
        failures.append(f"{BudgetKeys.MAX_LLM_CALLS}: {calls} > {budget.max_llm_calls}")
    if budget.has_max_total_tokens and tokens > budget.max_total_tokens:
        failures.append(f"{BudgetKeys.MAX_TOTAL_TOKENS}: {tokens} > {budget.max_total_tokens}")
    return failures
