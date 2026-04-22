"""Unit tests for budget loading + enforcement."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from quality_eval.budgets import Budget, enforce, load_budget  # noqa: E402


def _write_metrics(dir_: Path, calls: int, tokens: int) -> None:
    (dir_ / "metrics.json").write_text(json.dumps({
        "token_accounting": {
            "total": {
                "input_tokens": tokens // 2,
                "output_tokens": tokens - tokens // 2,
                "total_tokens": tokens,
                "call_count": calls,
            },
            "per_node": {},
            "per_flow": {},
            "per_agent": {},
        }
    }))


def test_load_budget_round_trip(tmp_path: Path):
    p = tmp_path / "budget.toml"
    p.write_text("max_llm_calls = 3\nmax_total_tokens = 1500\n")
    b = load_budget(p)
    assert b.max_llm_calls == 3
    assert b.max_total_tokens == 1500
    assert b.has_caps


def test_load_budget_missing_file_returns_empty(tmp_path: Path):
    b = load_budget(tmp_path / "nope.toml")
    assert b.max_llm_calls == 0 and b.max_total_tokens == 0
    assert not b.has_caps


def test_enforce_within_budget_returns_empty(tmp_path: Path):
    _write_metrics(tmp_path, calls=2, tokens=500)
    assert enforce(tmp_path, Budget(max_llm_calls=3, max_total_tokens=1000)) == []


def test_enforce_calls_exceeded(tmp_path: Path):
    _write_metrics(tmp_path, calls=5, tokens=500)
    fails = enforce(tmp_path, Budget(max_llm_calls=3, max_total_tokens=1000))
    assert any("max_llm_calls: 5 > 3" in f for f in fails)


def test_enforce_tokens_exceeded(tmp_path: Path):
    _write_metrics(tmp_path, calls=1, tokens=2000)
    fails = enforce(tmp_path, Budget(max_llm_calls=3, max_total_tokens=1000))
    assert any("max_total_tokens: 2000 > 1000" in f for f in fails)


def test_enforce_both_exceeded_lists_both(tmp_path: Path):
    _write_metrics(tmp_path, calls=10, tokens=5000)
    fails = enforce(tmp_path, Budget(max_llm_calls=3, max_total_tokens=1000))
    assert len(fails) == 2


def test_enforce_no_caps_is_noop_even_without_metrics(tmp_path: Path):
    # No metrics.json on disk. Empty Budget → no-op (not a failure).
    assert enforce(tmp_path, Budget()) == []


def test_enforce_missing_metrics_with_caps_fails(tmp_path: Path):
    fails = enforce(tmp_path, Budget(max_llm_calls=1))
    assert fails and "metrics.json missing" in fails[0]
