"""Unit tests for the rubric DSL + parser."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from quality_eval.rubric import Rubric, apply_rubric, load_rubric  # noqa: E402


def test_must_contain_passes_when_substring_present():
    res = apply_rubric("The capital is Paris.", Rubric(must_contain=["paris"]))
    assert res.passed
    assert res.failures == []


def test_must_contain_case_sensitive_fails():
    res = apply_rubric("Paris", Rubric(must_contain=["paris"], case_sensitive=True))
    assert not res.passed
    assert res.failures == ["must_contain: 'paris'"]


def test_must_not_contain_blocks_hallucination():
    res = apply_rubric("Christopher Columbus sailed in 1492.",
                       Rubric(must_not_contain=["1492"]))
    assert not res.passed
    assert "must_not_contain: '1492'" in res.failures[0]


def test_regex_match_finds_python_def():
    res = apply_rubric("def foo(x): return x", Rubric(regex_match=[r"def\s+\w+"]))
    assert res.passed


def test_regex_match_failure_lists_pattern():
    res = apply_rubric("no function here", Rubric(regex_match=[r"def\s+\w+"]))
    assert not res.passed
    assert res.failures[0].startswith("regex_match:")


def test_min_chars_fails_when_short():
    res = apply_rubric("short", Rubric(min_chars=50))
    assert not res.passed
    assert "min_chars" in res.failures[0]


def test_max_chars_fails_when_long():
    res = apply_rubric("x" * 100, Rubric(max_chars=10))
    assert not res.passed
    assert "max_chars" in res.failures[0]


def test_multiple_failures_accumulate():
    res = apply_rubric(
        "Berlin",
        Rubric(must_contain=["paris"], min_chars=20, regex_match=[r"\d+"]),
    )
    assert not res.passed
    assert len(res.failures) == 3


def test_load_rubric_round_trip(tmp_path: Path):
    p = tmp_path / "expected.toml"
    p.write_text(
        'must_contain = ["foo"]\n'
        'must_not_contain = ["bar"]\n'
        'regex_match = ["\\\\d+"]\n'
        'min_chars = 10\n'
        'max_chars = 200\n'
        'case_sensitive = true\n'
        'judge_prompt = "Is this faithful?"\n'
        'judge_threshold = 3\n'
    )
    r = load_rubric(p)
    assert r.must_contain == ["foo"]
    assert r.must_not_contain == ["bar"]
    assert r.regex_match == [r"\d+"]
    assert r.min_chars == 10
    assert r.max_chars == 200
    assert r.case_sensitive is True
    assert r.judge_prompt == "Is this faithful?"
    assert r.judge_threshold == 3


def test_load_rubric_ignores_unknown_keys(tmp_path: Path):
    p = tmp_path / "expected.toml"
    p.write_text('must_contain = ["x"]\nfuture_field = "foo"\n')
    r = load_rubric(p)
    assert r.must_contain == ["x"]
