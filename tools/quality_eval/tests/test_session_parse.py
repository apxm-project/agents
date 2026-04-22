"""Unit tests for session output extraction."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

# Make ``tools/`` importable so ``from quality_eval.x import y`` works.
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from quality_eval.session_parse import extract_final_output  # noqa: E402

FIX = Path(__file__).parent / "fixtures" / "sample_session"


def test_extract_uses_results_final_output():
    assert extract_final_output(FIX) == "Paris."


def test_extract_falls_back_to_exit_values(tmp_path: Path):
    (tmp_path / "results.json").write_text(json.dumps({
        "node_outputs": {},
        "token_values": {},
        "exit_values": {"7": "Berlin.", "3": "earlier"},
        "final_node_id": None,
        "final_output": "",
    }))
    # Highest-id wins — 7 > 3.
    assert extract_final_output(tmp_path) == "Berlin."


def test_extract_falls_back_to_token_values_when_exits_empty(tmp_path: Path):
    (tmp_path / "results.json").write_text(json.dumps({
        "node_outputs": {},
        "token_values": {"11": "from-token"},
        "exit_values": {},
        "final_node_id": None,
        "final_output": "",
    }))
    assert extract_final_output(tmp_path) == "from-token"


def test_extract_jsonifies_non_string_value(tmp_path: Path):
    (tmp_path / "results.json").write_text(json.dumps({
        "node_outputs": {},
        "token_values": {},
        "exit_values": {"1": {"city": "Paris", "pop": 2_100_000}},
        "final_node_id": None,
        "final_output": "",
    }))
    out = extract_final_output(tmp_path)
    # Order within the dict is implementation-defined; reparse to compare.
    assert json.loads(out) == {"city": "Paris", "pop": 2_100_000}


def test_extract_raises_when_no_output_anywhere(tmp_path: Path):
    (tmp_path / "results.json").write_text(json.dumps({
        "node_outputs": {},
        "token_values": {},
        "exit_values": {},
        "final_node_id": None,
        "final_output": "",
    }))
    with pytest.raises(RuntimeError):
        extract_final_output(tmp_path)
