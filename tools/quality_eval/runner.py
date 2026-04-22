"""Compile + execute + score a quality fixture.

Each fixture lives at ``tests/quality_fixtures/<name>/`` with three files:
  - ``graph.air``      : the AIS graph to compile
  - ``expected.toml``  : rubric (PC1)
  - ``budget.toml``    : optional spend caps (PC6)

The runner shells out to ``dekk apxm execute`` (the public CLI) with
``--emit-session`` so the captured outputs land in a tempdir; we then
read the session via ``session_parse.extract_final_output`` (PC2),
apply the rubric, optionally invoke the judge (PC4), and finally
enforce the budget (PC6).

Backend selection is *not* a CLI flag on ``dekk apxm execute`` — it is
configured globally in ``~/.apxm/config.toml``. The runner accepts
``backend`` as a label for reporting only; switching backends in CI
means staging the right config beforehand.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

from ._keys import DEFAULT_JUDGE_THRESHOLD, Cli, FixtureFiles
from .budgets import enforce, load_budget
from .judge import Judge, NullJudge
from .rubric import Rubric, apply_rubric, load_rubric
from .session_parse import extract_final_output

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_ROOT = REPO_ROOT / "tests" / "quality_fixtures"


@dataclass
class FixtureResult:
    name: str
    opt_level: int
    backend: str
    passed: bool
    output: str
    rubric_failures: list[str] = field(default_factory=list)
    judge_score: int | None = None
    judge_rationale: str = ""
    budget_failures: list[str] = field(default_factory=list)
    golden_failure: str = ""
    error: str = ""

    @property
    def all_failures(self) -> list[str]:
        out = list(self.rubric_failures) + list(self.budget_failures)
        if self.golden_failure:
            out.append(self.golden_failure)
        if self.judge_score is not None and self.judge_score < DEFAULT_JUDGE_THRESHOLD:
            out.append(f"judge: SCORE={self.judge_score}")
        if self.error:
            out.append(f"error: {self.error}")
        return out


def list_fixtures() -> list[str]:
    if not FIXTURE_ROOT.exists():
        return []
    return sorted(
        p.name for p in FIXTURE_ROOT.iterdir() if (p / FixtureFiles.EXPECTED).exists()
    )


def _execute(graph: Path, opt_level: int, session_dir: Path) -> None:
    """Drive ``dekk apxm execute`` against the configured backend."""
    cmd = [
        Cli.DEKK, Cli.APXM, Cli.EXECUTE, str(graph),
        Cli.OPT_FLAG, str(opt_level),
        Cli.EMIT_SESSION_FLAG, str(session_dir),
    ]
    subprocess.check_call(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)


def run_fixture(
    name: str,
    opt_level: int = 2,
    backend: str = "configured",
    judge: Judge | None = None,
) -> FixtureResult:
    """Run a single fixture once. ``judge`` defaults to ``NullJudge`` so this
    is safe to call without a backend configured for grading."""
    judge = judge or NullJudge()
    fix = FIXTURE_ROOT / name
    if not (fix / FixtureFiles.EXPECTED).exists():
        return FixtureResult(
            name=name, opt_level=opt_level, backend=backend, passed=False,
            output="", error=f"fixture {name!r} missing {FixtureFiles.EXPECTED} at {fix}",
        )

    rubric: Rubric = load_rubric(fix / FixtureFiles.EXPECTED)
    budget = load_budget(fix / FixtureFiles.BUDGET)

    with tempfile.TemporaryDirectory(prefix="apxm-quality-") as tmp:
        session_dir = Path(tmp) / "session"
        try:
            _execute(fix / FixtureFiles.GRAPH, opt_level, session_dir)
        except subprocess.CalledProcessError as e:
            return FixtureResult(
                name=name, opt_level=opt_level, backend=backend, passed=False,
                output="", error=f"execute failed (exit {e.returncode})",
            )

        try:
            output = extract_final_output(session_dir)
        except (FileNotFoundError, RuntimeError) as e:
            return FixtureResult(
                name=name, opt_level=opt_level, backend=backend, passed=False,
                output="", error=f"session parse failed: {e}",
            )

        rubric_res = apply_rubric(output, rubric)
        budget_fails = enforce(session_dir, budget)

    # Optional byte-exact contract for deterministic templates. The plan's
    # optimisation_invariant fixture uses this to assert the const-string
    # path survives every -O level unchanged.
    golden_failure = ""
    golden_path = fix / FixtureFiles.GOLDEN
    if golden_path.exists():
        golden = golden_path.read_text()
        if output.rstrip("\n") != golden.rstrip("\n"):
            golden_failure = (
                f"golden_output mismatch: got {output!r}, expected {golden!r}"
            )

    judge_score: int | None = None
    judge_rationale = ""
    judge_passed = True
    if rubric.judge_prompt:
        verdict = judge.score(output, rubric.judge_prompt, threshold=rubric.judge_threshold)
        judge_score = verdict.score
        judge_rationale = verdict.rationale
        judge_passed = verdict.passed

    passed = (
        rubric_res.passed
        and judge_passed
        and not budget_fails
        and not golden_failure
    )
    return FixtureResult(
        name=name, opt_level=opt_level, backend=backend,
        passed=passed, output=output,
        rubric_failures=rubric_res.failures,
        judge_score=judge_score, judge_rationale=judge_rationale,
        budget_failures=budget_fails,
        golden_failure=golden_failure,
    )


# -----------------------------------------------------------------------------
# Stability sampling (PC5 lives here too — same module, no wrapper layer)


@dataclass
class StabilityResult:
    name: str
    runs: list[FixtureResult]
    pass_count: int
    threshold: int

    @property
    def passed(self) -> bool:
        return self.pass_count >= self.threshold


def sample_fixture(
    name: str,
    opt_level: int = 2,
    backend: str = "configured",
    judge: Judge | None = None,
    samples: int = 3,
    threshold: int = 2,
) -> StabilityResult:
    """Run a fixture ``samples`` times; pass when ``pass_count >= threshold``.

    Useful when a fixture's rubric is loose enough that a single non-deterministic
    backend reply can flicker. Default 3/2 matches the plan; CI can override
    via the ``--samples``/``--threshold`` CLI flags.
    """
    if samples < 1:
        raise ValueError("samples must be >= 1")
    if threshold < 1 or threshold > samples:
        raise ValueError(f"threshold {threshold} not in [1,{samples}]")
    runs = [run_fixture(name, opt_level, backend, judge) for _ in range(samples)]
    pass_count = sum(1 for r in runs if r.passed)
    return StabilityResult(name=name, runs=runs, pass_count=pass_count, threshold=threshold)


# -----------------------------------------------------------------------------
# Pretty-print one stability result for the CLI / CI logs.


def format_result(r: FixtureResult) -> str:
    line = f"{r.name} -O{r.opt_level} backend={r.backend}: {'PASS' if r.passed else 'FAIL'}"
    if r.all_failures:
        line += os.linesep + os.linesep.join(f"  - {f}" for f in r.all_failures)
    return line
