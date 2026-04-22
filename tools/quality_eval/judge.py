"""Pluggable judge for tier-3 quality fixtures.

Two implementations ship by default:
  - ``NullJudge`` always returns a passing verdict and is the safe default
    for CI runs without API keys. It does not silently mask quality bugs
    because the rubric (must_contain/regex_match/etc.) still gates the run;
    the judge only matters for fixtures whose rubric carries a non-empty
    ``judge_prompt``.
  - ``LLMJudge`` shells out to ``dekk apxm execute`` against a configured
    backend, runs a single ASK whose template asks the model to grade the
    candidate output on a 0-5 scale, and parses ``SCORE=<n>`` out of the
    reply. Rationale is the verbatim model output (trimmed).
"""

from __future__ import annotations

import json
import re
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from ._keys import DEFAULT_JUDGE_MODEL, DEFAULT_JUDGE_THRESHOLD, Cli


@dataclass
class JudgeVerdict:
    score: int          # 0-5
    rationale: str
    passed: bool        # score >= threshold


class Judge(Protocol):
    def score(self, output: str, prompt: str, threshold: int = DEFAULT_JUDGE_THRESHOLD) -> JudgeVerdict: ...


class NullJudge:
    """Always passes. Use when no real backend is configured."""

    def score(self, output: str, prompt: str, threshold: int = DEFAULT_JUDGE_THRESHOLD) -> JudgeVerdict:
        return JudgeVerdict(
            score=5,
            rationale="NullJudge: backend-free CI mode; rubric still applied",
            passed=True,
        )


class LLMJudge:
    """Real-backend judge. Spawns an ephemeral single-ASK graph per call.

    Driving via the public ``dekk apxm execute`` CLI (rather than linking the
    Rust driver directly) keeps the harness's blast radius small: a backend
    misconfiguration manifests as a subprocess exit code, not an in-process
    crash that takes the runner down with it.
    """

    JUDGE_TEMPLATE = (
        "You are grading a candidate output against a rubric.\n"
        "RUBRIC:\n{prompt}\n\n"
        "CANDIDATE OUTPUT:\n{output}\n\n"
        "Reply on a single line in this exact format:\n"
        "SCORE=<integer 0-5> RATIONALE=<one short sentence>"
    )

    def __init__(self, backend: str, model: str = DEFAULT_JUDGE_MODEL):
        self.backend = backend
        self.model = model

    def _build_graph(self, full_prompt: str, dest: Path) -> Path:
        graph = dest / "judge.air"
        # json.dumps gives us a properly-escaped MLIR string literal.
        escaped = json.dumps(full_prompt)
        graph.write_text(
            "module {\n"
            "  func.func @judge() -> !ais.token attributes {ais.entry} {\n"
            f"    %v = ais.ask {escaped} : !ais.token\n"
            '    ais.print "{v}" [%v : !ais.token] {input_names = ["v"]}\n'
            "    func.return %v : !ais.token\n"
            "  }\n"
            "}\n"
        )
        return graph

    def score(self, output: str, prompt: str, threshold: int = DEFAULT_JUDGE_THRESHOLD) -> JudgeVerdict:
        full = self.JUDGE_TEMPLATE.format(prompt=prompt, output=output)
        with tempfile.TemporaryDirectory(prefix="apxm-judge-") as tmp:
            tmpdir = Path(tmp)
            graph = self._build_graph(full, tmpdir)
            session = tmpdir / "session"
            cmd = [
                Cli.DEKK, Cli.APXM, Cli.EXECUTE, str(graph),
                Cli.OPT_FLAG, "2",
                Cli.EMIT_SESSION_FLAG, str(session),
            ]
            try:
                subprocess.check_call(
                    cmd,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.STDOUT,
                )
            except subprocess.CalledProcessError as e:
                return JudgeVerdict(
                    score=0,
                    rationale=f"LLMJudge subprocess failed: {e}",
                    passed=False,
                )

            from .session_parse import extract_final_output
            verdict_text = extract_final_output(session)

        m = re.search(r"SCORE\s*=\s*(\d)", verdict_text)
        score = int(m.group(1)) if m else 0
        rationale = verdict_text.strip()
        return JudgeVerdict(score=score, rationale=rationale, passed=score >= threshold)


def make_judge(kind: str, backend: str | None = None) -> Judge:
    """Factory used by ``__main__`` and the runner.

    `kind ∈ {"none", "llm"}`. ``llm`` requires a backend name.
    """
    if kind == "none":
        return NullJudge()
    if kind == "llm":
        if not backend:
            raise ValueError("LLMJudge requires --backend; refusing NullJudge fallback "
                             "to avoid silently downgrading a quality gate")
        return LLMJudge(backend=backend)
    raise ValueError(f"unknown judge kind {kind!r}")
