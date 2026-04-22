"""Quality-rubric DSL backing tier-3 fixture scoring.

`expected.toml` is the on-disk format; `Rubric` is its in-memory shape.
`apply_rubric` is pure: no I/O, no LLM calls — those live in `judge.py`.

A rubric is a hard-fail conjunction of textual constraints
(must_contain / must_not_contain / regex_match / min_chars / max_chars)
plus an optional `judge_prompt` that the runner forwards to a `Judge`.
The judge's verdict is *not* applied here so unit tests stay hermetic.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import List


@dataclass
class Rubric:
    must_contain: List[str] = field(default_factory=list)
    must_not_contain: List[str] = field(default_factory=list)
    regex_match: List[str] = field(default_factory=list)
    min_chars: int = 0
    max_chars: int = 0
    case_sensitive: bool = False
    judge_prompt: str = ""
    judge_threshold: int = 4


@dataclass
class RubricResult:
    passed: bool
    failures: List[str] = field(default_factory=list)


def apply_rubric(output: str, r: Rubric) -> RubricResult:
    failures: List[str] = []
    haystack = output if r.case_sensitive else output.lower()

    for needle in r.must_contain:
        probe = needle if r.case_sensitive else needle.lower()
        if probe not in haystack:
            failures.append(f"must_contain: '{needle}'")

    for needle in r.must_not_contain:
        probe = needle if r.case_sensitive else needle.lower()
        if probe in haystack:
            failures.append(f"must_not_contain: '{needle}'")

    flags = 0 if r.case_sensitive else re.IGNORECASE
    for pattern in r.regex_match:
        if not re.search(pattern, output, flags):
            failures.append(f"regex_match: '{pattern}'")

    if r.min_chars and len(output) < r.min_chars:
        failures.append(f"min_chars: {len(output)} < {r.min_chars}")
    if r.max_chars and len(output) > r.max_chars:
        failures.append(f"max_chars: {len(output)} > {r.max_chars}")

    return RubricResult(passed=not failures, failures=failures)


def load_rubric(path: str | Path) -> Rubric:
    """Parse an `expected.toml` rubric file. Unknown keys are ignored so the
    schema can grow without breaking older fixtures."""
    try:
        import tomllib  # py311+
    except ModuleNotFoundError:  # pragma: no cover - fallback for older runtimes
        import tomli as tomllib  # type: ignore[no-redef]

    with open(path, "rb") as f:
        data = tomllib.load(f)

    return Rubric(
        must_contain=list(data.get("must_contain", [])),
        must_not_contain=list(data.get("must_not_contain", [])),
        regex_match=list(data.get("regex_match", [])),
        min_chars=int(data.get("min_chars", 0)),
        max_chars=int(data.get("max_chars", 0)),
        case_sensitive=bool(data.get("case_sensitive", False)),
        judge_prompt=str(data.get("judge_prompt", "")),
        judge_threshold=int(data.get("judge_threshold", 4)),
    )
