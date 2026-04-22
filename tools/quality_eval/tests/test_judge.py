"""Unit tests for the judge layer.

LLMJudge requires a backend round-trip and lives behind a stubbable seam
(``LLMJudge.score`` calls ``dekk apxm execute``). We don't shell out from
unit tests; the SCORE-parsing logic is exercised separately via a tiny
helper kept in lock-step with the parser inside ``LLMJudge.score``.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from quality_eval.judge import LLMJudge, NullJudge, make_judge  # noqa: E402


def test_null_judge_always_passes():
    v = NullJudge().score("anything", "any prompt")
    assert v.passed and v.score == 5
    assert "NullJudge" in v.rationale


def test_make_judge_none():
    assert isinstance(make_judge("none"), NullJudge)


def test_make_judge_llm_requires_backend():
    with pytest.raises(ValueError):
        make_judge("llm")  # no backend


def test_make_judge_llm_returns_llm():
    j = make_judge("llm", backend="anthropic")
    assert isinstance(j, LLMJudge)
    assert j.backend == "anthropic"


def test_make_judge_unknown():
    with pytest.raises(ValueError):
        make_judge("magic")


# Mirror the regex used inside LLMJudge.score so the parser contract is
# under test even when we cannot drive a real backend.
_SCORE_RE = re.compile(r"SCORE\s*=\s*(\d)")


@pytest.mark.parametrize(
    "text, expected",
    [
        ("SCORE=5 RATIONALE=great", 5),
        ("SCORE = 3 RATIONALE=okay", 3),
        ("Reply: SCORE = 0 RATIONALE=bad", 0),
        ("no score here", 0),
        ("score=4 lowercase ignored", 0),
    ],
)
def test_score_parser_matches_implementation(text: str, expected: int):
    m = _SCORE_RE.search(text)
    parsed = int(m.group(1)) if m else 0
    assert parsed == expected


def test_llm_judge_build_graph_escapes_quotes(tmp_path: Path):
    j = LLMJudge(backend="anthropic")
    graph = j._build_graph('contains "quotes" and \\backslash', tmp_path)
    text = graph.read_text()
    # The MLIR string literal must escape both characters via json.dumps.
    assert '\\"quotes\\"' in text
    assert "\\\\backslash" in text
    assert text.startswith("module {")
