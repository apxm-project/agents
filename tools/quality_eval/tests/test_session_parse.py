"""Unit tests for session output extraction."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

# Make ``tools/`` importable so ``from quality_eval.x import y`` works.
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from quality_eval.session_parse import (  # noqa: E402
    _resolve_session_root,
    _strip_thinking_blocks,
    extract_final_output,
)

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


def test_resolve_session_root_descends_into_single_subdir(tmp_path: Path):
    # `dekk apxm execute --emit-session <PATH>` treats PATH as a base dir
    # and creates `<PATH>/<stem>-<timestamp>/` underneath. The harness
    # passes a tempdir, so resolution must descend one level.
    inner = tmp_path / "qa_factual-20260422T172024"
    inner.mkdir()
    (inner / "results.json").write_text("{}")
    assert _resolve_session_root(tmp_path) == inner


def test_resolve_session_root_accepts_direct_dir(tmp_path: Path):
    (tmp_path / "results.json").write_text("{}")
    assert _resolve_session_root(tmp_path) == tmp_path


def test_resolve_session_root_raises_on_ambiguous_layout(tmp_path: Path):
    # Two candidate subdirs with no results.json at the top: don't guess.
    (tmp_path / "a").mkdir()
    (tmp_path / "b").mkdir()
    with pytest.raises(FileNotFoundError):
        _resolve_session_root(tmp_path)


def test_strip_thinking_blocks_removes_single_block():
    raw = "<think>let me reason step by step</think>Paris."
    assert _strip_thinking_blocks(raw) == "Paris."


def test_strip_thinking_blocks_removes_multiline_block():
    raw = "<think>\nthe capital of France\nis Paris\n</think>\nParis."
    assert _strip_thinking_blocks(raw) == "Paris."


def test_strip_thinking_blocks_removes_multiple_blocks():
    raw = "<think>first</think>middle<think>second</think>final"
    assert _strip_thinking_blocks(raw) == "middlefinal"


def test_strip_thinking_blocks_passes_through_unchanged():
    assert _strip_thinking_blocks("just an answer") == "just an answer"


def test_strip_thinking_blocks_handles_asymmetric_qwen3_format():
    # vLLM-served Qwen3.5-4B emits "Thinking Process:\n...\n</think>\nanswer"
    # with NO opening <think> tag. Strip must still drop the prelude.
    raw = (
        "Thinking Process:\n\n"
        "1. Analyze the request\n"
        "2. Recall the answer\n"
        "</think>\n\nParis"
    )
    assert _strip_thinking_blocks(raw) == "Paris"


def test_strip_thinking_blocks_leaves_text_without_close_tag_alone():
    # No </think> anywhere → leave the text exactly as-is. Guards against the
    # leading regex over-matching when the model didn't emit a thinking block.
    assert _strip_thinking_blocks("Paris is the capital.") == "Paris is the capital."


def test_extract_strips_thinking_blocks_from_final_output(tmp_path: Path):
    (tmp_path / "results.json").write_text(json.dumps({
        "node_outputs": {},
        "token_values": {},
        "exit_values": {},
        "final_node_id": None,
        "final_output": "<think>capital of France is Paris</think>Paris.",
    }))
    assert extract_final_output(tmp_path) == "Paris."


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
