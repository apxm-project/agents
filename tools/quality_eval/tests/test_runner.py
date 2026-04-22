"""Unit tests for runner.run_fixture / sample_fixture / format_result.

We monkeypatch ``runner._execute`` so no real backend is required; the
tests exercise the wire-up of rubric, judge, and budget against a session
tree we synthesise on disk.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Callable

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from quality_eval import runner  # noqa: E402
from quality_eval.judge import JudgeVerdict, NullJudge  # noqa: E402


def _write_session(session_dir: Path, output: str, *, calls: int = 1, tokens: int = 50) -> None:
    session_dir.mkdir(parents=True, exist_ok=True)
    (session_dir / "results.json").write_text(json.dumps({
        "node_outputs": {}, "token_values": {}, "exit_values": {"5": output},
        "final_node_id": 5, "final_output": output,
    }))
    (session_dir / "metrics.json").write_text(json.dumps({
        "token_accounting": {
            "total": {"input_tokens": tokens // 2, "output_tokens": tokens - tokens // 2,
                      "total_tokens": tokens, "call_count": calls},
            "per_node": {}, "per_flow": {}, "per_agent": {},
        }
    }))


def _stub_execute(output: str, calls: int = 1, tokens: int = 50) -> Callable:
    def _exec(graph: Path, opt_level: int, session_dir: Path) -> None:
        _write_session(session_dir, output, calls=calls, tokens=tokens)
    return _exec


def _make_fixture(root: Path, name: str, expected_toml: str, budget_toml: str = "") -> Path:
    fix = root / name
    fix.mkdir(parents=True)
    (fix / "graph.air").write_text("module { func.func @x() { return } }")
    (fix / "expected.toml").write_text(expected_toml)
    if budget_toml:
        (fix / "budget.toml").write_text(budget_toml)
    return fix


@pytest.fixture(autouse=True)
def _isolated_fixture_root(tmp_path: Path, monkeypatch):
    monkeypatch.setattr(runner, "FIXTURE_ROOT", tmp_path)
    return tmp_path


def test_run_fixture_passes_when_rubric_satisfied(monkeypatch, _isolated_fixture_root):
    _make_fixture(_isolated_fixture_root, "qa", 'must_contain = ["paris"]\n')
    monkeypatch.setattr(runner, "_execute", _stub_execute("The capital is Paris."))
    res = runner.run_fixture("qa", opt_level=2, backend="mock")
    assert res.passed
    assert res.rubric_failures == []
    assert res.budget_failures == []


def test_run_fixture_fails_on_rubric(monkeypatch, _isolated_fixture_root):
    _make_fixture(_isolated_fixture_root, "qa", 'must_contain = ["paris"]\n')
    monkeypatch.setattr(runner, "_execute", _stub_execute("The capital is Berlin."))
    res = runner.run_fixture("qa", opt_level=2, backend="mock")
    assert not res.passed
    assert any("must_contain: 'paris'" in f for f in res.rubric_failures)


def test_run_fixture_fails_on_budget(monkeypatch, _isolated_fixture_root):
    _make_fixture(_isolated_fixture_root, "qa",
                  'must_contain = ["paris"]\n',
                  budget_toml="max_llm_calls = 1\nmax_total_tokens = 30\n")
    monkeypatch.setattr(runner, "_execute", _stub_execute("Paris.", calls=1, tokens=200))
    res = runner.run_fixture("qa", opt_level=2, backend="mock")
    assert not res.passed
    assert any("max_total_tokens" in f for f in res.budget_failures)


def test_run_fixture_invokes_judge_when_prompt_present(monkeypatch, _isolated_fixture_root):
    _make_fixture(_isolated_fixture_root, "qa",
                  'must_contain = ["paris"]\n'
                  'judge_prompt = "Is Paris correct?"\n'
                  'judge_threshold = 4\n')
    monkeypatch.setattr(runner, "_execute", _stub_execute("Paris."))

    class StubJudge:
        def score(self, output, prompt, threshold=4):
            assert "Paris" in output and "correct" in prompt
            return JudgeVerdict(score=2, rationale="too short", passed=False)

    res = runner.run_fixture("qa", opt_level=2, backend="mock", judge=StubJudge())
    assert not res.passed
    assert res.judge_score == 2
    assert "judge: SCORE=2" in res.all_failures


def test_run_fixture_skips_judge_when_no_prompt(monkeypatch, _isolated_fixture_root):
    _make_fixture(_isolated_fixture_root, "qa", 'must_contain = ["paris"]\n')
    monkeypatch.setattr(runner, "_execute", _stub_execute("Paris."))
    res = runner.run_fixture("qa", opt_level=2, backend="mock", judge=NullJudge())
    assert res.passed
    assert res.judge_score is None  # no judge invocation


def test_run_fixture_reports_subprocess_failure(monkeypatch, _isolated_fixture_root):
    import subprocess
    _make_fixture(_isolated_fixture_root, "qa", 'must_contain = ["paris"]\n')

    def _boom(graph, opt_level, session_dir):
        raise subprocess.CalledProcessError(returncode=2, cmd=["dekk"])

    monkeypatch.setattr(runner, "_execute", _boom)
    res = runner.run_fixture("qa", opt_level=2, backend="mock")
    assert not res.passed
    assert "execute failed (exit 2)" in res.error


def test_run_fixture_missing_fixture_dir(_isolated_fixture_root):
    res = runner.run_fixture("does_not_exist", opt_level=2, backend="mock")
    assert not res.passed
    assert "missing expected.toml" in res.error


def test_sample_fixture_majority_pass(monkeypatch, _isolated_fixture_root):
    _make_fixture(_isolated_fixture_root, "qa", 'must_contain = ["paris"]\n')
    outputs = iter(["Paris.", "Berlin.", "Paris and the Eiffel Tower."])

    def _exec(graph, opt_level, session_dir):
        _write_session(session_dir, next(outputs))

    monkeypatch.setattr(runner, "_execute", _exec)
    stab = runner.sample_fixture("qa", samples=3, threshold=2, backend="mock")
    assert stab.pass_count == 2
    assert stab.passed


def test_sample_fixture_below_threshold(monkeypatch, _isolated_fixture_root):
    _make_fixture(_isolated_fixture_root, "qa", 'must_contain = ["paris"]\n')
    outputs = iter(["Berlin.", "Paris.", "Madrid."])

    def _exec(graph, opt_level, session_dir):
        _write_session(session_dir, next(outputs))

    monkeypatch.setattr(runner, "_execute", _exec)
    stab = runner.sample_fixture("qa", samples=3, threshold=2, backend="mock")
    assert stab.pass_count == 1
    assert not stab.passed


def test_sample_fixture_validates_args(_isolated_fixture_root):
    with pytest.raises(ValueError):
        runner.sample_fixture("qa", samples=0)
    with pytest.raises(ValueError):
        runner.sample_fixture("qa", samples=3, threshold=5)


def test_format_result_includes_failures(monkeypatch, _isolated_fixture_root):
    _make_fixture(_isolated_fixture_root, "qa", 'must_contain = ["paris"]\n')
    monkeypatch.setattr(runner, "_execute", _stub_execute("Berlin."))
    res = runner.run_fixture("qa", opt_level=2, backend="mock")
    out = runner.format_result(res)
    assert "FAIL" in out
    assert "must_contain: 'paris'" in out


def test_list_fixtures(_isolated_fixture_root):
    _make_fixture(_isolated_fixture_root, "alpha", 'must_contain = ["x"]\n')
    _make_fixture(_isolated_fixture_root, "beta", 'must_contain = ["y"]\n')
    # Skip a directory without expected.toml.
    (_isolated_fixture_root / "gamma").mkdir()
    assert runner.list_fixtures() == ["alpha", "beta"]
